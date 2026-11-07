// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Errors raised by the VST3 host bridge.

use thiserror::Error;

/// Any failure in the VST3 host layer.
#[derive(Debug, Error)]
pub enum Error {
    /// The VST3 SDK is not available: `VST3_SDK_DIR` unset, no C++ compiler,
    /// or the `native` feature is disabled.
    #[error("VST3 native bridge is unavailable (set VST3_SDK_DIR, install a C++ compiler, enable the `native` feature)")]
    BridgeUnavailable,

    /// A VST3 plugin module could not be loaded (file missing, wrong format,
    /// missing `GetPluginFactory` export).
    #[error("could not load VST3 plugin module: {0}")]
    ModuleLoad(String),

    /// An OS-level file-system error during plugin discovery.
    #[error("VST3 plugin discovery I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A requested plugin class ID was not found in the module's factory.
    #[error("VST3 plugin class not found: {0}")]
    ClassNotFound(String),

    /// The plugin instance could not be created.
    #[error("could not instantiate VST3 plugin: {0}")]
    Instantiation(String),

    /// The plugin is not in a state where the requested operation is allowed.
    #[error("VST3 plugin not initialized: {0}")]
    NotInitialized(String),

    /// Audio processing failed (the plugin returned an error result).
    #[error("VST3 audio processing failed: {0}")]
    Processing(String),

    /// A configuration the host cannot safely represent was requested, such
    /// as an unsupported channel count or a block larger than the setup.
    #[error("unsupported VST3 configuration: {0}")]
    Unsupported(String),

    /// State serialization/deserialization failed.
    #[error("VST3 plugin state error: {0}")]
    State(String),

    /// The plugin reports it cannot process audio (e.g. it is an instrument,
    /// not an effect).
    #[error("VST3 plugin is not an audio effect: {0}")]
    NotAnEffect(String),
}
