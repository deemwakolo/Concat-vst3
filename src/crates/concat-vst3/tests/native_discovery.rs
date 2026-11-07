// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Native discovery tests: real factory enumeration and class-ID fidelity.

#[cfg(feature = "native")]
use concat_vst3::bridge::ClassId;
#[cfg(feature = "native")]
use concat_vst3::Registry;

#[cfg(feature = "native")]
#[test]
fn discovered_class_ids_match_the_factory() {
    let registry = Registry::scan().expect("registry scan should not fail");
    let plugins = registry.all();
    if plugins.is_empty() {
        return; // No effects installed on this machine.
    }

    for descriptor in &plugins {
        let class_id = ClassId::from_fuid(&descriptor.class_id)
            .expect("discovery must emit a parseable 32-hex-digit class ID");
        assert_eq!(
            class_id.to_fuid(),
            descriptor.class_id,
            "class ID text must round-trip without byte loss"
        );
        assert!(!descriptor.name.trim().is_empty());
        assert!(!descriptor.class_id.contains('-'));
    }

    // Check a few descriptors against the module's actual factory. Loading
    // every installed plug-in repeatedly is unnecessary and slow.
    let mut checked = 0;
    for descriptor in plugins.iter().take(3) {
        let module = registry
            .load_module(&descriptor.class_id)
            .expect("discovered module should load");
        let count = module.num_classes().expect("factory enumeration should work");
        assert!(count > 0, "a discovered module must expose an effect class");

        let mut matching_classes = 0;
        for index in 0..count {
            let info = module
                .class_info_at(index)
                .expect("every enumerated effect index should have metadata");
            assert_eq!(
                info.category(),
                "Audio Module Class",
                "only audio-effect classes may enter the registry"
            );
            if info.class_id().to_fuid() == descriptor.class_id {
                matching_classes += 1;
            }
        }
        assert_eq!(
            matching_classes, 1,
            "the registry class ID must identify exactly one factory class"
        );
        checked += 1;
    }
    assert!(checked > 0);
}

#[cfg(feature = "native")]
#[test]
fn controller_classes_are_not_reported_as_effects() {
    let registry = Registry::scan().expect("registry scan should not fail");
    let plugins = registry.all();
    if plugins.is_empty() {
        return;
    }

    // FabFilter Micro exports three classes: an audio effect, its controller,
    // and a compatibility class. Only the audio effect may be discovered.
    let descriptor = plugins.iter().find(|descriptor| {
        descriptor.name.eq_ignore_ascii_case("Micro")
            && descriptor
                .path
                .to_string_lossy()
                .to_ascii_lowercase()
                .contains("fabfilter")
    });
    let Some(descriptor) = descriptor else {
        return; // FabFilter Micro is not installed on this machine.
    };

    let module = registry
        .load_module(&descriptor.class_id)
        .expect("module should load");
    let count = module.num_classes().expect("factory enumeration should work");
    assert_eq!(
        count, 1,
        "FabFilter Micro has three factory classes but only one audio effect"
    );

    let info = module.class_info_at(0).expect("effect metadata should exist");
    assert_eq!(info.category(), "Audio Module Class");
    assert_eq!(info.class_id().to_fuid(), descriptor.class_id);
    assert!(
        !info.vendor().is_empty(),
        "vendor should fall back to PFactoryInfo when PClassInfo2 leaves it empty"
    );

    assert!(module.class_info_at(-1).is_err());
    assert!(module.class_info_at(count).is_err());
}

#[cfg(feature = "native")]
#[test]
fn invalid_class_indices_are_rejected() {
    let registry = Registry::scan().expect("registry scan should not fail");
    let plugins = registry.all();
    let Some(descriptor) = plugins.first() else {
        return;
    };

    let module = registry
        .load_module(&descriptor.class_id)
        .expect("module should load");
    let count = module.num_classes().expect("factory enumeration should work");

    assert!(module.class_info_at(-1).is_err());
    assert!(module.class_info_at(count).is_err());
    assert!(module.class_info_at(count + 1000).is_err());
}
