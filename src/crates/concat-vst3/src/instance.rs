// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! One instantiated VST3 plugin, with safe lifecycle and parameter I/O.
//!
//! A `Plugin` owns a `PluginHandle` into the C++ bridge. It is thread-affine:
//! VST3 plugins are generally not thread-safe, so a `Plugin` must be used
//! from the same thread that created it (typically a decode worker). It is
//! `Send` so it can be moved between threads, but callers must not use it
//! concurrently from two threads.
//!
//! When the `native` feature is disabled, all operations return
//! [`Error::BridgeUnavailable`].

use crate::bridge;
#[cfg(feature = "native")]
use crate::buffer;
use crate::errors::Error;
use crate::latency_tail::{LatencyInfo, TailInfo, MAX_SAFE_TAIL_SECONDS};
use crate::parameters::ParameterInfo;

/// One live VST3 plugin instance.
///
/// When `native` is off, `Plugin::create` always returns
/// [`Error::BridgeUnavailable`].
#[derive(Debug)]
pub struct Plugin {
    #[cfg(feature = "native")]
    inner: NativePlugin,
    #[cfg(not(feature = "native"))]
    inner: (),
}

/// Native-side plugin state, only present when the `native` feature is on.
#[cfg(feature = "native")]
#[derive(Debug)]
struct NativePlugin {
    /// The module that produced this plugin (kept alive so the .dll isn't
    /// unloaded while we hold an instance).
    _module: bridge::Module,
    handle: bridge::PluginHandle,
    initialized: bool,
    active: bool,
    num_channels: i32,
    max_block_size: i32,
    sample_rate: f64,
    /// Reusable planar scratch buffers. VST3 expects one contiguous buffer
    /// per channel, not a pointer into an interleaved stream.
    planar_in: Vec<f32>,
    planar_out: Vec<f32>,
    input_ptrs: Vec<*const f32>,
    output_ptrs: Vec<*mut f32>,
}

#[cfg(feature = "native")]
impl Plugin {
    /// Creates a plugin instance from `class_id` inside `module`.
    ///
    /// Does not call `initialize`: use [`Plugin::initialize`] before
    /// processing.
    pub fn create(module: bridge::Module, class_id: &bridge::ClassId) -> Result<Self, Error> {
        unsafe {
            let handle = bridge::vst3_plugin_create(module.handle, class_id as *const _);
            if handle.is_null() {
                return Err(Error::Instantiation(format!("class {}", class_id.to_fuid())));
            }
            Ok(Self {
                inner: NativePlugin {
                    _module: module,
                    handle,
                    initialized: false,
                    active: false,
                    num_channels: 0,
                    max_block_size: 0,
                    sample_rate: 0.0,
                    planar_in: Vec::new(),
                    planar_out: Vec::new(),
                    input_ptrs: Vec::new(),
                    output_ptrs: Vec::new(),
                },
            })
        }
    }

    /// Prepares the plugin for processing at `sample_rate` Hz, with blocks
    /// of up to `max_block_size` frames and `num_channels` channels.
    pub fn initialize(
        &mut self,
        sample_rate: f64,
        max_block_size: i32,
        num_channels: i32,
    ) -> Result<(), Error> {
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return Err(Error::Unsupported(format!(
                "sample rate must be finite and positive, got {sample_rate}"
            )));
        }
        if max_block_size <= 0 {
            return Err(Error::Unsupported(format!(
                "maximum block size must be positive, got {max_block_size}"
            )));
        }
        if num_channels != 1 && num_channels != 2 {
            return Err(Error::Unsupported(format!(
                "only mono and stereo are supported, got {num_channels} channels"
            )));
        }

        let inner = self.inner_mut()?;
        let ok = unsafe {
            bridge::vst3_plugin_initialize(inner.handle, sample_rate, max_block_size, num_channels)
        };
        if ok == 0 {
            return Err(Error::NotInitialized(format!(
                "sample_rate={}, max_block_size={}, num_channels={}",
                sample_rate, max_block_size, num_channels
            )));
        }

        let capacity = (max_block_size as usize)
            .checked_mul(num_channels as usize)
            .ok_or_else(|| Error::Unsupported("block/channel layout overflow".to_owned()))?;
        inner.planar_in.clear();
        inner.planar_in.resize(capacity, 0.0);
        inner.planar_out.clear();
        inner.planar_out.resize(capacity, 0.0);
        inner.input_ptrs.clear();
        inner.input_ptrs.reserve(num_channels as usize);
        inner.output_ptrs.clear();
        inner.output_ptrs.reserve(num_channels as usize);
        inner.sample_rate = sample_rate;
        inner.max_block_size = max_block_size;
        inner.num_channels = num_channels;
        inner.initialized = true;
        Ok(())
    }

    /// Starts or stops processing. While inactive the plugin consumes no CPU
    /// and passes no audio.
    pub fn set_active(&mut self, active: bool) -> Result<(), Error> {
        let inner = self.inner_mut()?;
        let ok = unsafe { bridge::vst3_plugin_activate(inner.handle, active as i32) };
        if ok == 0 {
            return Err(Error::Processing(format!(
                "could not {} plugin",
                if active { "activate" } else { "deactivate" }
            )));
        }
        inner.active = active;
        Ok(())
    }

    /// Sets bypass on or off. Bypassed plugins pass audio through
    /// unprocessed but remain loaded and initialized.
    pub fn set_bypass(&mut self, bypass: bool) -> Result<(), Error> {
        let inner = self.inner_mut()?;
        unsafe { bridge::vst3_plugin_set_bypass(inner.handle, bypass as i32) };
        Ok(())
    }

    /// Returns the plugin's algorithmic latency in samples.
    pub fn latency(&self) -> Result<LatencyInfo, Error> {
        let inner = self.inner_ref()?;
        let samples = unsafe { bridge::vst3_plugin_get_latency(inner.handle) };
        Ok(LatencyInfo {
            samples: samples.max(0) as u32,
        })
    }

    /// Returns the estimated tail time in seconds.
    pub fn tail(&self) -> Result<TailInfo, Error> {
        let inner = self.inner_ref()?;
        let seconds = unsafe { bridge::vst3_plugin_get_tail_seconds(inner.handle) };
        Ok(TailInfo {
            seconds: if seconds.is_finite() {
                seconds.clamp(0.0, MAX_SAFE_TAIL_SECONDS)
            } else {
                0.0
            },
        })
    }

    /// Returns the number of parameters the plugin exposes.
    pub fn num_parameters(&self) -> Result<i32, Error> {
        let inner = self.inner_ref()?;
        Ok(unsafe { bridge::vst3_plugin_num_parameters(inner.handle) })
    }

    /// Returns information about the parameter at `index`.
    pub fn parameter_info(&self, index: i32) -> Result<ParameterInfo, Error> {
        let inner = self.inner_ref()?;
        let mut info = std::mem::MaybeUninit::<bridge::ParameterInfo>::uninit();
        let ok = unsafe {
            bridge::vst3_plugin_get_parameter_info(inner.handle, index, info.as_mut_ptr())
        };
        if ok == 0 {
            return Err(Error::Processing(format!("parameter info for index {index}")));
        }
        Ok(unsafe { info.assume_init() }.into())
    }

    /// Returns the normalized value (0.0..=1.0) of `param_id`.
    pub fn get_parameter(&self, param_id: i32) -> Result<f64, Error> {
        let inner = self.inner_ref()?;
        Ok(unsafe { bridge::vst3_plugin_get_parameter(inner.handle, param_id) })
    }

    /// Sets the normalized value (0.0..=1.0) of `param_id`.
    pub fn set_parameter(&mut self, param_id: i32, value: f64) -> Result<(), Error> {
        let inner = self.inner_mut()?;
        unsafe { bridge::vst3_plugin_set_parameter(inner.handle, param_id, value) };
        Ok(())
    }

    /// Applies named normalized values, leaving unspecified parameters at
    /// their current plugin state.
    pub fn set_parameters_by_name(
        &mut self,
        values: &std::collections::BTreeMap<String, f64>,
    ) -> Result<(), Error> {
        let count = self.num_parameters()?;
        for index in 0..count {
            let info = self.parameter_info(index)?;
            let Some(value) = values.get(&info.name).copied() else {
                continue;
            };
            if !value.is_finite() {
                return Err(Error::Unsupported(format!(
                    "parameter {} is not finite",
                    info.name
                )));
            }
            self.set_parameter(info.id, value.clamp(0.0, 1.0))?;
        }
        Ok(())
    }

    /// Serializes the plugin's current state into a byte vector.
    pub fn save_state(&self) -> Result<Vec<u8>, Error> {
        let inner = self.inner_ref()?;
        let mut data: *mut std::os::raw::c_void = std::ptr::null_mut();
        let mut size: std::os::raw::c_int = 0;
        let ok = unsafe { bridge::vst3_plugin_get_state(inner.handle, &mut data, &mut size) };
        if ok == 0 || data.is_null() {
            return Err(Error::State("get_state returned null".into()));
        }
        if size < 0 {
            unsafe { bridge::vst3_plugin_state_free(data) };
            return Err(Error::State("get_state returned a negative size".into()));
        }
        let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, size as usize) };
        let out = bytes.to_vec();
        unsafe { bridge::vst3_plugin_state_free(data) };
        Ok(out)
    }

    /// Restores plugin state previously obtained from [`Plugin::save_state`].
    pub fn load_state(&mut self, state: &[u8]) -> Result<(), Error> {
        let size = std::os::raw::c_int::try_from(state.len())
            .map_err(|_| Error::State("state blob exceeds the bridge size limit".into()))?;
        let inner = self.inner_mut()?;
        let ok = unsafe {
            bridge::vst3_plugin_set_state(
                inner.handle,
                state.as_ptr() as *const std::os::raw::c_void,
                size,
            )
        };
        if ok == 0 {
            return Err(Error::State("set_state rejected the blob".into()));
        }
        Ok(())
    }

    /// Processes one block of interleaved audio.
    ///
    /// VST3 audio buffers are planar, so this deinterleaves the input into
    /// reusable per-channel scratch buffers, processes the block, and then
    /// interleaves the output back into `interleaved`. The conversion is
    /// allocation-free after the first block at a given size.
    pub fn process_interleaved(
        &mut self,
        interleaved: &mut [f32],
        num_frames: i32,
    ) -> Result<bridge::ProcessResult, Error> {
        let inner = self.inner_mut()?;
        if !inner.initialized {
            return Err(Error::NotInitialized(
                "process_interleaved called before initialize".to_owned(),
            ));
        }
        if num_frames <= 0 {
            return Err(Error::Unsupported(format!(
                "frame count must be positive, got {num_frames}"
            )));
        }

        let num_channels = inner.num_channels as usize;
        if num_channels != 1 && num_channels != 2 {
            return Err(Error::Unsupported(format!(
                "only mono and stereo are supported, got {num_channels} channels"
            )));
        }

        let frames = num_frames as usize;
        if frames > inner.max_block_size as usize {
            return Err(Error::Unsupported(format!(
                "block of {frames} frames exceeds the configured maximum of {}",
                inner.max_block_size
            )));
        }

        let total_samples = frames
            .checked_mul(num_channels)
            .ok_or_else(|| Error::Unsupported("frame/channel layout overflow".to_owned()))?;
        if interleaved.len() < total_samples {
            return Err(Error::Processing(format!(
                "buffer too small: need {total_samples} samples, got {}",
                interleaved.len()
            )));
        }

        if !inner.active {
            return Ok(bridge::ProcessResult {
                frames_processed: num_frames,
                tail_pending: false,
            });
        }

        if inner.planar_in.len() < total_samples {
            inner.planar_in.resize(total_samples, 0.0);
        }
        if inner.planar_out.len() < total_samples {
            inner.planar_out.resize(total_samples, 0.0);
        }

        buffer::deinterleave(
            &interleaved[..total_samples],
            frames,
            num_channels,
            &mut inner.planar_in[..total_samples],
        )?;

        // Rebuild these each block: a previous resize may have moved the
        // scratch allocation, and the pointers must describe the current one.
        inner.input_ptrs.clear();
        let input_base = inner.planar_in.as_ptr();
        for channel in 0..num_channels {
            // SAFETY: `planar_in` has at least `total_samples` initialized
            // floats, so `channel * frames` is in bounds for each channel.
            inner
                .input_ptrs
                .push(unsafe { input_base.add(channel * frames) });
        }

        inner.output_ptrs.clear();
        let output_base = inner.planar_out.as_mut_ptr();
        for channel in 0..num_channels {
            // SAFETY: `planar_out` has at least `total_samples` initialized
            // floats, so `channel * frames` is in bounds for each channel.
            inner
                .output_ptrs
                .push(unsafe { output_base.add(channel * frames) });
        }

        let result = unsafe {
            bridge::vst3_plugin_process(
                inner.handle,
                inner.input_ptrs.as_ptr(),
                inner.output_ptrs.as_mut_ptr(),
                num_channels as i32,
                num_frames,
            )
        };

        if result.frames_processed != num_frames {
            return Err(Error::Processing(format!(
                "plugin processed {} of {num_frames} frames",
                result.frames_processed
            )));
        }

        buffer::interleave(
            &inner.planar_out[..total_samples],
            frames,
            num_channels,
            &mut interleaved[..total_samples],
        )?;

        Ok(result)
    }

    /// Returns the sample rate the plugin was initialized with.
    pub fn sample_rate(&self) -> f64 {
        self.inner.sample_rate
    }

    /// Returns whether the plugin is initialized.
    pub fn is_initialized(&self) -> bool {
        self.inner.initialized
    }

    /// Returns whether the plugin is active (processing).
    pub fn is_active(&self) -> bool {
        self.inner.active
    }

    fn inner_ref(&self) -> Result<&NativePlugin, Error> {
        Ok(&self.inner)
    }

    fn inner_mut(&mut self) -> Result<&mut NativePlugin, Error> {
        Ok(&mut self.inner)
    }
}

#[cfg(not(feature = "native"))]
impl Plugin {
    pub fn create(_module: bridge::Module, _class_id: &bridge::ClassId) -> Result<Self, Error> {
        Err(Error::BridgeUnavailable)
    }

    pub fn initialize(
        &mut self,
        _sample_rate: f64,
        _max_block_size: i32,
        _num_channels: i32,
    ) -> Result<(), Error> {
        Err(Error::BridgeUnavailable)
    }

    pub fn set_active(&mut self, _active: bool) -> Result<(), Error> {
        Err(Error::BridgeUnavailable)
    }

    pub fn set_bypass(&mut self, _bypass: bool) -> Result<(), Error> {
        Err(Error::BridgeUnavailable)
    }

    pub fn latency(&self) -> Result<LatencyInfo, Error> {
        Err(Error::BridgeUnavailable)
    }

    pub fn tail(&self) -> Result<TailInfo, Error> {
        Err(Error::BridgeUnavailable)
    }

    pub fn num_parameters(&self) -> Result<i32, Error> {
        Err(Error::BridgeUnavailable)
    }

    pub fn parameter_info(&self, _index: i32) -> Result<ParameterInfo, Error> {
        Err(Error::BridgeUnavailable)
    }

    pub fn get_parameter(&self, _param_id: i32) -> Result<f64, Error> {
        Err(Error::BridgeUnavailable)
    }

    pub fn set_parameter(&mut self, _param_id: i32, _value: f64) -> Result<(), Error> {
        Err(Error::BridgeUnavailable)
    }

    pub fn set_parameters_by_name(
        &mut self,
        _values: &std::collections::BTreeMap<String, f64>,
    ) -> Result<(), Error> {
        Err(Error::BridgeUnavailable)
    }

    pub fn save_state(&self) -> Result<Vec<u8>, Error> {
        Err(Error::BridgeUnavailable)
    }

    pub fn load_state(&mut self, _state: &[u8]) -> Result<(), Error> {
        Err(Error::BridgeUnavailable)
    }

    pub fn process_interleaved(
        &mut self,
        _interleaved: &mut [f32],
        num_frames: i32,
    ) -> Result<bridge::ProcessResult, Error> {
        Ok(bridge::ProcessResult {
            frames_processed: num_frames,
            tail_pending: false,
        })
    }

    pub fn sample_rate(&self) -> f64 {
        0.0
    }

    pub fn is_initialized(&self) -> bool {
        false
    }

    pub fn is_active(&self) -> bool {
        false
    }
}

impl Drop for Plugin {
    fn drop(&mut self) {
        #[cfg(feature = "native")]
        {
            if !self.inner.handle.is_null() {
                unsafe {
                    bridge::vst3_plugin_destroy(self.inner.handle);
                }
            }
        }
    }
}

// Safety: a Plugin is thread-affine but Send is fine because callers must
// guarantee single-threaded use. We do not implement Sync because a plugin
// must not be shared between threads.
unsafe impl Send for Plugin {}
