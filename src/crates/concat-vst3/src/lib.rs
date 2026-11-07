// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Native VST3 audio-effect hosting for Concat.
//!
//! This crate discovers, loads, and drives VST3 audio-effect plugins through
//! a C++ host bridge exposed over a C ABI. It is intentionally headless: it
//! knows nothing about FFmpeg, the timeline, or the UI. It depends on
//! `concat-core` only.
//!
//! # Feature: `native`
//!
//! Off by default. When enabled, `build.rs` compiles the C++ bridge from
//! `src/bridge.cpp` against the Steinberg VST3 SDK headers found at
//! `VST3_SDK_DIR`, and links it into this crate. When off (the default for
//! machines without the SDK or a C++ compiler), every plugin lookup reports
//! unavailable and the crate compiles as a stub.
//!
//! # Architecture
//!
//! - [`Registry`] scans the OS-standard VST3 folders and caches discovered
//!   plugins by class ID.
//! - [`Plugin`] wraps one instantiated plugin: initialization, parameter
//!   I/O, audio-block processing, state serialization, bypass, latency and
//!   tails.
//! - All processing happens off the CPAL real-time thread: the host calls
//!   `Plugin::process` from decode workers (preview) or the export render
//!   thread, never from the audio output callback.

pub mod bridge;
pub mod buffer;
pub mod discovery;
pub mod errors;
pub mod instance;
pub mod latency_tail;
pub mod parameters;
pub mod registry;

pub use errors::Error;
pub use instance::Plugin;
pub use latency_tail::{LatencyInfo, TailInfo};
pub use parameters::ParameterInfo;
pub use registry::{PluginDescriptor, Registry};

/// The sample rate Concat mixes at, in Hz. Both preview and export use this.
pub const MIX_RATE: f64 = 48_000.0;

/// The channel count Concat mixes in: stereo.
pub const MIX_CHANNELS: i32 = 2;

/// A plugin reference stored in a project: enough identity to find or
/// re-create the plugin without depending on its installed file path.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PluginRef {
    /// The VST3 class ID (FUID) as a 32-hex-digit string.
    pub class_id: String,
    /// Human-readable plugin name (informational only).
    pub name: String,
    /// Human-readable vendor name (informational only).
    pub vendor: String,
}
