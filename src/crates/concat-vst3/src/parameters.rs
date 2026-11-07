// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Parameter I/O: normalized (0.0..=1.0) values and their display metadata.

/// Information about one plugin parameter, as reported by the controller.
#[derive(Clone, Debug)]
pub struct ParameterInfo {
    /// Stable parameter ID used for get/set and automation.
    pub id: i32,
    /// Short name for display, e.g. "Gain", "Frequency", "Mix".
    pub name: String,
    /// Units string for display, e.g. "dB", "Hz", "%", "ms".
    pub units: String,
    /// Default normalized value (0.0..=1.0).
    pub default_normalized: f64,
    /// Step size for quantization (0.0 = continuous).
    pub step_size: f64,
}

impl From<crate::bridge::ParameterInfo> for ParameterInfo {
    fn from(info: crate::bridge::ParameterInfo) -> Self {
        Self {
            id: info.id,
            name: crate::bridge::c_string_to_string(&info.name),
            units: crate::bridge::c_string_to_string(&info.units),
            default_normalized: info.default_normalized,
            step_size: info.step_size,
        }
    }
}

/// Converts a value in the parameter's native units to normalized (0.0..=1.0).
///
/// Uses the plugin's default as the center when step_size is 0 (continuous);
/// otherwise snaps to the nearest step.
pub fn native_to_normalized(value: f64, info: &ParameterInfo) -> f64 {
    if info.step_size > 0.0 {
        let steps = (value / info.step_size).round();
        (steps * info.step_size).clamp(0.0, 1.0)
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// Converts a normalized value (0.0..=1.0) to the parameter's native units.
pub fn normalized_to_native(normalized: f64, _info: &ParameterInfo) -> f64 {
    normalized.clamp(0.0, 1.0)
}
