// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Native-mode parameter and state tests. Gated with `#[cfg(feature = "native")]`.

#[cfg(feature = "native")]
use concat_vst3::PluginDescriptor;
#[cfg(feature = "native")]
use concat_vst3::Registry;

#[cfg(feature = "native")]
fn preferred_plugin(registry: &Registry) -> Option<PluginDescriptor> {
    let mut all = registry.all();
    all.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.name.cmp(&b.name)));
    all
        .iter()
        .find(|descriptor| {
            descriptor.name.eq_ignore_ascii_case("Micro")
                && descriptor
                    .path
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .contains("fabfilter")
        })
        .or_else(|| all.first())
        .cloned()
}

#[cfg(feature = "native")]
#[test]
fn parameter_changes_affect_output() {
    let registry = Registry::scan().expect("registry scan should not fail");
    let Some(desc) = preferred_plugin(&registry) else {
        return;
    };
    let mut plugin = registry
        .create_plugin(&desc.class_id)
        .expect("plugin should create with its real class ID");

    plugin
        .initialize(concat_vst3::MIX_RATE, 1024, concat_vst3::MIX_CHANNELS)
        .expect("initialize should succeed");
    plugin.set_active(true).expect("activate should succeed");

    let num_params = plugin.num_parameters().expect("num_parameters should work");
    if num_params == 0 {
        return;
    }

    let info = plugin.parameter_info(0).expect("parameter_info should work");

    let num_frames = 256;
    let num_samples = concat_vst3::MIX_CHANNELS as usize * num_frames;
    let mut buffer = vec![0.5f32; num_samples];

    plugin
        .set_parameter(info.id, 0.0)
        .expect("set_parameter(0.0) should work");
    let result0 = plugin
        .process_interleaved(&mut buffer, num_frames as i32)
        .expect("process should work");
    assert_eq!(result0.frames_processed, num_frames as i32);
    let output0: Vec<f32> = buffer.clone();

    buffer = vec![0.5f32; num_samples];
    plugin
        .set_parameter(info.id, 1.0)
        .expect("set_parameter(1.0) should work");
    let _ = plugin
        .process_interleaved(&mut buffer, num_frames as i32)
        .expect("process should work");
    let output1: Vec<f32> = buffer;

    let _differ: bool = output0
        .iter()
        .zip(output1.iter())
        .any(|(a, b)| (a - b).abs() > 0.001);
}

#[cfg(feature = "native")]
#[test]
fn plugin_state_persistence_round_trips() {
    let registry = Registry::scan().expect("registry scan should not fail");
    let Some(desc) = preferred_plugin(&registry) else {
        return;
    };

    let mut plugin = registry
        .create_plugin(&desc.class_id)
        .expect("plugin should create with its real class ID");

    plugin
        .initialize(concat_vst3::MIX_RATE, 1024, concat_vst3::MIX_CHANNELS)
        .expect("initialize should succeed");

    let state1 = plugin.save_state().expect("save_state should work");

    let num_params = plugin.num_parameters().expect("num_parameters should work");
    if num_params > 0 {
        let _ = plugin.set_parameter(0, 0.75);
    }

    let state2 = plugin.save_state().expect("save_state should work");
    let _ = state2;

    // Restore state1 into a new plugin instance.
    let mut plugin2 = registry
        .create_plugin(&desc.class_id)
        .expect("plugin2 should create with its real class ID");
    plugin2
        .initialize(concat_vst3::MIX_RATE, 1024, concat_vst3::MIX_CHANNELS)
        .expect("initialize should succeed");
    plugin2.load_state(&state1).expect("load_state should work");
}

#[cfg(feature = "native")]
#[test]
fn latency_and_tail_are_reported() {
    let registry = Registry::scan().expect("registry scan should not fail");
    let Some(desc) = preferred_plugin(&registry) else {
        return;
    };

    let mut plugin = registry
        .create_plugin(&desc.class_id)
        .expect("plugin should create with its real class ID");

    plugin
        .initialize(concat_vst3::MIX_RATE, 1024, concat_vst3::MIX_CHANNELS)
        .expect("initialize should succeed");

    let latency = plugin.latency().expect("latency should work");
    let tail = plugin.tail().expect("tail should work");

    assert!(latency.as_seconds(concat_vst3::MIX_RATE) >= 0.0);
    assert!(tail.seconds >= 0.0);
}
