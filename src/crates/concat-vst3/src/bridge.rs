// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Rust FFI bindings to the C++ VST3 host bridge compiled from `bridge.cpp`.
//!
//! Every function here is a plain C symbol exported by the static library
//! built by `build.rs`. The bridge owns the VST3 C++ objects; Rust only
//! holds an opaque handle and calls through this ABI.
//!
//! When the `native` feature is disabled, the FFI declarations and `Module`
//! are unavailable: all plugin operations return [`Error::BridgeUnavailable`].

#[cfg(feature = "native")]
use crate::errors::Error;
#[cfg(feature = "native")]
use std::ffi::CString;
use std::os::raw::c_char;
use std::os::raw::c_void;
#[cfg(feature = "native")]
use std::os::raw::c_double;
#[cfg(feature = "native")]
use std::os::raw::c_int;
#[cfg(feature = "native")]
use std::os::raw::c_float;

/// Opaque handle to a loaded VST3 plugin module.
pub type ModuleHandle = *mut c_void;

/// Opaque handle to an instantiated VST3 plugin.
pub type PluginHandle = *mut c_void;

/// A VST3 plugin class identifier (a 128-bit GUID).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(C)]
pub struct ClassId {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl ClassId {
    /// Parses a VST3 class ID from its FUID string form, e.g.
    /// `"57616c68-6c61-4c61-4c61-56616c68616c"`.
    pub fn from_fuid(fuid: &str) -> Option<Self> {
        let hex: String = fuid.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if hex.len() != 32 {
            return None;
        }
        let mut bytes = [0u8; 16];
        for i in 0..16 {
            let hi = hex_digit(hex.as_bytes()[i * 2])?;
            let lo = hex_digit(hex.as_bytes()[i * 2 + 1])?;
            bytes[i] = (hi << 4) | lo;
        }
        Some(Self::from_tuid_bytes(bytes))
    }

    /// Builds a class ID from the exact 16 bytes of a VST3 `TUID`.
    ///
    /// The bytes are stored in the same order a VST3 factory returns them, so
    /// `to_tuid_bytes` is byte-for-byte reversible even where the textual GUID
    /// endianness differs between platforms.
    pub fn from_tuid_bytes(bytes: [u8; 16]) -> Self {
        let mut data4 = [0u8; 8];
        data4.copy_from_slice(&bytes[8..16]);
        Self {
            data1: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            data2: u16::from_le_bytes([bytes[4], bytes[5]]),
            data3: u16::from_le_bytes([bytes[6], bytes[7]]),
            data4,
        }
    }

    /// Returns the exact 16-byte `TUID` representation of this class ID.
    pub fn to_tuid_bytes(self) -> [u8; 16] {
        let mut bytes = [0u8; 16];
        bytes[0..4].copy_from_slice(&self.data1.to_le_bytes());
        bytes[4..6].copy_from_slice(&self.data2.to_le_bytes());
        bytes[6..8].copy_from_slice(&self.data3.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.data4);
        bytes
    }

    /// The FUID string form: 32 hex digits.
    pub fn to_fuid(self) -> String {
        self.to_tuid_bytes()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }
}

/// Information about one plugin parameter, as reported by the controller.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct ParameterInfo {
    /// Stable parameter ID used for get/set and automation.
    pub id: i32,
    /// Short name for display (UTF-8, null-terminated in the C string).
    pub name: [c_char; 64],
    /// Units string for display (UTF-8, null-terminated), e.g. "dB", "ms", "%".
    pub units: [c_char; 64],
    /// Default normalized value (0.0..=1.0).
    pub default_normalized: f64,
    /// Step size for quantization (0.0 = continuous).
    pub step_size: f64,
}

/// Metadata for one audio-effect class exported by a VST3 module.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct ClassInfo {
    /// The exact 16-byte VST3 `TUID` returned by the factory.
    pub class_id: [u8; 16],
    /// Human-readable class name (UTF-8).
    pub name: [c_char; 128],
    /// Human-readable vendor name (UTF-8); may be empty if the module has none.
    pub vendor: [c_char; 128],
    /// VST3 class category; discovery only accepts `Audio Module Class`.
    pub category: [c_char; 64],
    /// VST3 sub-category list, e.g. `Fx|Filter` or `Instrument|Synth`.
    pub sub_categories: [c_char; 128],
}

impl ClassInfo {
    /// The class ID as a strongly typed value.
    pub fn class_id(&self) -> ClassId {
        ClassId::from_tuid_bytes(self.class_id)
    }

    /// The class name as a Rust string.
    pub fn name(&self) -> String {
        c_string_to_string(&self.name)
    }

    /// The vendor name as a Rust string.
    pub fn vendor(&self) -> String {
        c_string_to_string(&self.vendor)
    }

    /// The VST3 category as a Rust string.
    pub fn category(&self) -> String {
        c_string_to_string(&self.category)
    }

    /// The VST3 sub-category list as a Rust string.
    pub fn sub_categories(&self) -> String {
        c_string_to_string(&self.sub_categories)
    }
}

/// Result of a processing call.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct ProcessResult {
    /// Number of input frames consumed.
    pub frames_processed: i32,
    /// Whether the plugin still has tail output pending after this call.
    pub tail_pending: bool,
}

// FFI declarations: only available when the `native` feature is on.
#[cfg(feature = "native")]
unsafe extern "C" {
    pub fn vst3_module_load(path: *const c_char) -> ModuleHandle;
    pub fn vst3_module_unload(handle: ModuleHandle);

    pub fn vst3_module_num_classes(module: ModuleHandle) -> c_int;

    pub fn vst3_module_get_class_id(module: ModuleHandle, index: i32) -> *mut c_char;

    pub fn vst3_module_get_class_info(
        module: ModuleHandle,
        index: i32,
        out: *mut ClassInfo,
    ) -> c_int;

    pub fn vst3_plugin_create(
        module: ModuleHandle,
        class_id: *const ClassId,
    ) -> PluginHandle;
    pub fn vst3_plugin_destroy(plugin: PluginHandle);

    pub fn vst3_plugin_initialize(
        plugin: PluginHandle,
        sample_rate: c_double,
        max_block_size: i32,
        num_channels: i32,
    ) -> c_int;

    pub fn vst3_plugin_activate(plugin: PluginHandle, active: c_int) -> c_int;

    pub fn vst3_plugin_get_latency(plugin: PluginHandle) -> i32;
    pub fn vst3_plugin_get_tail_seconds(plugin: PluginHandle) -> c_double;

    pub fn vst3_plugin_num_parameters(plugin: PluginHandle) -> i32;

    pub fn vst3_plugin_get_parameter_info(
        plugin: PluginHandle,
        index: i32,
        out: *mut ParameterInfo,
    ) -> c_int;

    pub fn vst3_plugin_get_parameter(plugin: PluginHandle, param_id: i32) -> c_double;
    pub fn vst3_plugin_set_parameter(plugin: PluginHandle, param_id: i32, value: c_double);

    pub fn vst3_plugin_get_state(
        plugin: PluginHandle,
        out_data: *mut *mut c_void,
        out_size: *mut c_int,
    ) -> c_int;
    pub fn vst3_plugin_set_state(plugin: PluginHandle, data: *const c_void, size: c_int) -> c_int;
    pub fn vst3_plugin_state_free(data: *mut c_void);

    pub fn vst3_plugin_process(
        plugin: PluginHandle,
        inputs: *const *const c_float,
        outputs: *mut *mut c_float,
        num_channels: i32,
        num_frames: i32,
    ) -> ProcessResult;

    pub fn vst3_plugin_set_bypass(plugin: PluginHandle, bypass: c_int);
}

/// Stub module type used when the `native` feature is off, so that APIs
/// mentioning a module still type-check while every operation reports
/// [`Error::BridgeUnavailable`].
#[cfg(not(feature = "native"))]
#[derive(Debug)]
pub struct Module;

/// High-level wrapper around `vst3_module_load` / `vst3_module_unload`.
///
/// Only available when the `native` feature is on.
#[cfg(feature = "native")]
#[derive(Debug)]
pub struct Module {
    pub(crate) handle: ModuleHandle,
    pub(crate) path: String,
}

#[cfg(feature = "native")]
impl Module {
    /// Loads a VST3 plugin module from `path`.
    pub fn load(path: &str) -> Result<Self, Error> {
        let c_path = CString::new(path).map_err(|_| Error::ModuleLoad(path.to_owned()))?;
        unsafe {
            let handle = vst3_module_load(c_path.as_ptr());
            if handle.is_null() {
                return Err(Error::ModuleLoad(path.to_owned()));
            }
            Ok(Self { handle, path: path.to_owned() })
        }
    }

    /// Returns the number of VST3 plugin classes in this module.
    pub fn num_classes(&self) -> Result<c_int, Error> {
        let count = unsafe { vst3_module_num_classes(self.handle) };
        if count < 0 {
            Err(Error::BridgeUnavailable)
        } else {
            Ok(count)
        }
    }

    /// Returns metadata for the audio-effect class at `index`.
    ///
    /// The index addresses only audio-effect classes; controller and
    /// instrument classes are excluded by the bridge.
    pub fn class_info_at(&self, index: i32) -> Result<ClassInfo, Error> {
        if index < 0 {
            return Err(Error::ClassNotFound(format!("class index {index}")));
        }
        let mut info = std::mem::MaybeUninit::<ClassInfo>::uninit();
        let ok = unsafe { vst3_module_get_class_info(self.handle, index, info.as_mut_ptr()) };
        if ok == 0 {
            return Err(Error::ClassNotFound(format!("class index {index}")));
        }
        Ok(unsafe { info.assume_init() })
    }

    /// Returns the class ID of the audio-effect class at the given index as a
    /// lowercase hex-encoded FUID string.
    pub fn class_id_at(&self, index: i32) -> Result<String, Error> {
        Ok(self.class_info_at(index)?.class_id().to_fuid())
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

#[cfg(feature = "native")]
impl Drop for Module {
    fn drop(&mut self) {
        unsafe {
            vst3_module_unload(self.handle);
        }
    }
}

#[cfg(feature = "native")]
unsafe impl Send for Module {}

#[cfg(feature = "native")]
unsafe impl Sync for Module {}

/// Converts a null-terminated C string buffer to a Rust String, truncating
/// at the first NUL byte. Always available.
pub fn c_string_to_string(buf: &[c_char]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let bytes: Vec<u8> = buf[..len].iter().map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Parses a single hexadecimal digit byte. Always available.
fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
