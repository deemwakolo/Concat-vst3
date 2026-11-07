// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Native-mode processing tests. Gated with `#[cfg(feature = "native")]`.

#[cfg(feature = "native")]
use concat_vst3::PluginDescriptor;
#[cfg(feature = "native")]
use concat_vst3::Registry;

/// Returns a stable, well-behaved effect when one is installed. The VST3
/// registry is backed by a hash map, so tests must sort before choosing a
/// plugin rather than relying on iteration order.
#[cfg(feature = "native")]
fn preferred_plugin(registry: &Registry) -> Option<PluginDescriptor> {
    let mut all = registry.all();
    all.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.name.cmp(&b.name)));

    all.iter()
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
fn processing_handles_silence_without_corruption() {
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

    let num_frames = 1024;
    let num_samples = concat_vst3::MIX_CHANNELS as usize * num_frames;
    let mut buffer = vec![0.0f32; num_samples];

    let result = plugin
        .process_interleaved(&mut buffer, num_frames as i32)
        .expect("process should succeed");
    assert_eq!(result.frames_processed, num_frames as i32);

    let peak = buffer.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    assert!(
        peak < 0.01,
        "silence in should produce silence out, peak was {peak}"
    );
}

#[cfg(feature = "native")]
#[test]
fn processing_preserves_finite_stereo_samples() {
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

    let num_frames = 512;
    let num_samples = concat_vst3::MIX_CHANNELS as usize * num_frames;
    let mut buffer: Vec<f32> = (0..num_samples)
        .map(|i| {
            let frame = i / concat_vst3::MIX_CHANNELS as usize;
            let t = frame as f32 / concat_vst3::MIX_RATE as f32;
            (440.0 * 2.0 * std::f32::consts::PI * t).sin() * 0.25
        })
        .collect();

    let result = plugin
        .process_interleaved(&mut buffer, num_frames as i32)
        .expect("process should succeed");
    assert_eq!(result.frames_processed, num_frames as i32);
    assert!(
        buffer.iter().all(|sample| sample.is_finite()),
        "processing produced a non-finite sample"
    );
}

#[cfg(feature = "native")]
#[test]
fn bypass_passes_audio_through() {
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

    let num_frames = 512;
    let num_samples = concat_vst3::MIX_CHANNELS as usize * num_frames;
    let mut buffer: Vec<f32> = (0..num_samples)
        .map(|i| {
            let frame = i / concat_vst3::MIX_CHANNELS as usize;
            let t = frame as f32 / concat_vst3::MIX_RATE as f32;
            (440.0 * 2.0 * std::f32::consts::PI * t).sin() * 0.5
        })
        .collect();
    let original = buffer.clone();

    plugin
        .set_bypass(false)
        .expect("set_bypass(false) should work");
    let _processed = plugin
        .process_interleaved(&mut buffer, num_frames as i32)
        .expect("processing should succeed");

    buffer = original.clone();
    plugin.set_bypass(true).expect("set_bypass(true) should work");
    let result = plugin
        .process_interleaved(&mut buffer, num_frames as i32)
        .expect("bypassed processing should succeed");
    assert_eq!(result.frames_processed, num_frames as i32);

    for (index, (expected, actual)) in original.iter().zip(buffer.iter()).enumerate() {
        assert!(
            (expected - actual).abs() < 1e-6,
            "bypass should pass audio through, sample {index} differs: {expected} vs {actual}"
        );
    }
}

#[cfg(feature = "native")]
#[test]
fn unsupported_channel_and_block_configurations_are_rejected() {
    let registry = Registry::scan().expect("registry scan should not fail");
    let Some(desc) = preferred_plugin(&registry) else {
        return;
    };

    let mut plugin = registry
        .create_plugin(&desc.class_id)
        .expect("plugin should create with its real class ID");

    assert!(plugin.initialize(concat_vst3::MIX_RATE, 0, 2).is_err());
    assert!(plugin.initialize(concat_vst3::MIX_RATE, 1024, 0).is_err());
    assert!(plugin.initialize(concat_vst3::MIX_RATE, 1024, 3).is_err());
    assert!(plugin.initialize(0.0, 1024, 2).is_err());

    plugin
        .initialize(concat_vst3::MIX_RATE, 1024, 2)
        .expect("valid initialize should succeed");
    plugin.set_active(true).expect("activate should succeed");

    let mut buffer = vec![0.0f32; 2048];
    assert!(plugin.process_interleaved(&mut buffer, 0).is_err());
    assert!(plugin.process_interleaved(&mut buffer, 1025).is_err());
    assert!(
        plugin
            .process_interleaved(&mut buffer[..10], 16)
            .is_err(),
        "a short interleaved buffer must be rejected"
    );
}

#[cfg(feature = "native")]
#[test]
fn mono_processing_is_supported_when_the_plugin_accepts_mono() {
    let registry = Registry::scan().expect("registry scan should not fail");
    let Some(desc) = preferred_plugin(&registry) else {
        return;
    };

    let mut plugin = registry
        .create_plugin(&desc.class_id)
        .expect("plugin should create with its real class ID");

    if plugin
        .initialize(concat_vst3::MIX_RATE, 256, 1)
        .is_err()
    {
        return; // This particular effect is stereo-only.
    }
    plugin.set_active(true).expect("activate should succeed");

    let num_frames = 256;
    let mut buffer: Vec<f32> = (0..num_frames)
        .map(|frame| {
            let t = frame as f32 / concat_vst3::MIX_RATE as f32;
            (440.0 * 2.0 * std::f32::consts::PI * t).sin() * 0.25
        })
        .collect();

    let result = plugin
        .process_interleaved(&mut buffer, num_frames as i32)
        .expect("mono processing should succeed");
    assert_eq!(result.frames_processed, num_frames as i32);
    assert!(buffer.iter().all(|sample| sample.is_finite()));
}
