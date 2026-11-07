// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Tests that run when the `native` feature is ON.
//! These require VST3_SDK_DIR, a C++ compiler, and at least one installed
//! VST3 effect plugin. Gated with `#[cfg(feature = "native")]`.

#[cfg(feature = "native")]
use concat_vst3::Registry;

#[cfg(feature = "native")]
#[test]
fn load_and_unload_test_plugin() {
    let registry = Registry::scan().expect("registry scan should not fail");
    let plugins = registry.all();
    if plugins.is_empty() {
        return; // No plugins installed: skip gracefully.
    }

    let desc = &plugins[0];
    let plugin = registry
        .create_plugin(&desc.class_id)
        .expect("plugin should create with its real class ID");
    assert!(!plugin.is_initialized());
    // Destroy is implicit via Drop when plugin goes out of scope.
}

#[cfg(feature = "native")]
#[test]
fn missing_plugin_does_not_panic() {
    let registry = Registry::scan().expect("registry scan should not fail");
    let err = registry
        .create_plugin("00000000000000000000000000000000")
        .unwrap_err();
    assert!(matches!(err, concat_vst3::Error::ClassNotFound(_)));
}
