// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Tests that run when the `native` feature is OFF (stub mode).
//! These verify that concat-vst3 compiles and behaves correctly without
//! the VST3 SDK or a C++ compiler.

#[cfg(not(feature = "native"))]
use concat_vst3::{Error, PluginRef, Registry};

#[cfg(not(feature = "native"))]
#[test]
fn stub_registry_reports_no_plugins() {
    let registry = Registry::scan().expect("registry scan should not fail in stub mode");
    assert_eq!(registry.len(), 0);
    assert!(registry.is_empty());
}

#[cfg(not(feature = "native"))]
#[test]
fn stub_resolve_returns_none_for_any_class_id() {
    let registry = Registry::scan().expect("registry scan should not fail");
    assert!(registry.resolve("nonexistent-class-id").is_none());
}

#[cfg(not(feature = "native"))]
#[test]
fn stub_create_plugin_returns_class_not_found() {
    let registry = Registry::scan().expect("registry scan should not fail");
    let err = registry.create_plugin("nonexistent-class-id").unwrap_err();
    assert!(matches!(err, Error::ClassNotFound(_)));
}

#[cfg(not(feature = "native"))]
#[test]
fn plugin_ref_serializes_and_deserializes() {
    let ref_ = PluginRef {
        class_id: "57616c686c614c614c6156616c68616c".to_string(),
        name: "Test Plugin".to_string(),
        vendor: "Test Vendor".to_string(),
    };

    let json = serde_json::to_string(&ref_).unwrap();
    let restored: PluginRef = serde_json::from_str(&json).unwrap();
    assert_eq!(ref_, restored);
}

#[cfg(not(feature = "native"))]
#[test]
fn class_id_from_fuid_parses_correctly() {
    use concat_vst3::bridge::ClassId;
    let guid = ClassId::from_fuid("57616c68-6c61-4c61-4c61-56616c68616c").unwrap();
    // Round-trip through to_fuid.
    let fuid = guid.to_fuid();
    assert_eq!(fuid.len(), 32);
    let reparsed = ClassId::from_fuid(&fuid).unwrap();
    assert_eq!(guid, reparsed);
}

#[cfg(not(feature = "native"))]
#[test]
fn class_id_from_fuid_rejects_invalid_input() {
    use concat_vst3::bridge::ClassId;
    assert!(ClassId::from_fuid("not-a-guid").is_none());
    assert!(ClassId::from_fuid("too-short").is_none());
    assert!(ClassId::from_fuid("").is_none());
}

#[cfg(not(feature = "native"))]
#[test]
fn class_id_preserves_every_tuid_byte() {
    use concat_vst3::bridge::ClassId;

    // Every byte value, so an accidental endian swap or signed-buffer bug
    // cannot pass unnoticed.
    let bytes: [u8; 16] = [
        0x00, 0x01, 0x7f, 0x80, 0xfe, 0xff, 0x34, 0xfc, 0x1a, 0x75, 0x10, 0xab, 0xcd, 0xef, 0x55,
        0xaa,
    ];
    let class_id = ClassId::from_tuid_bytes(bytes);
    assert_eq!(class_id.to_tuid_bytes(), bytes);

    let fuid = class_id.to_fuid();
    assert_eq!(fuid.len(), 32);
    let reparsed = ClassId::from_fuid(&fuid).expect("FUID should parse");
    assert_eq!(reparsed, class_id);
    assert_eq!(reparsed.to_tuid_bytes(), bytes);
}
