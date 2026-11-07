// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Build script for concat-vst3.
//!
//! When the `native` feature is enabled, this compiles the C++ host bridge
//! (`src/bridge.cpp`) against the Steinberg VST3 SDK and links it into the
//! crate. The `native` feature is intentionally optional: with it disabled,
//! `concat-vst3` builds as a stub and does not require the SDK or a C++
//! compiler.

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let native = env::var("CARGO_FEATURE_NATIVE").unwrap_or_default() == "1";

    if !native {
        return;
    }

    println!("cargo:rerun-if-env-changed=VST3_SDK_DIR");
    println!("cargo:rerun-if-env-changed=CC");
    println!("cargo:rerun-if-env-changed=CXX");
    println!("cargo:rerun-if-env-changed=PATH");

    let sdk_dir = match env::var("VST3_SDK_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => {
            panic!(
                "concat-vst3: native feature enabled but VST3_SDK_DIR is not set; \
                 set it to an unpacked Steinberg VST3 SDK"
            );
        }
    };

    if !sdk_dir.is_dir() {
        panic!(
            "concat-vst3: VST3_SDK_DIR {:?} is not a directory",
            sdk_dir
        );
    }

    let pluginterfaces = sdk_dir.join("pluginterfaces");
    let base_dir = sdk_dir.join("base").join("source");

    if !pluginterfaces.is_dir() {
        panic!(
            "concat-vst3: VST3_SDK_DIR {:?} lacks pluginterfaces/",
            sdk_dir
        );
    }

    // Let cc-rs perform the actual compiler discovery. Its logic is more
    // reliable than shelling out to `where`, especially for MSVC environments
    // initialized by vcvars64.bat, and it validates that the selected compiler
    // can target the requested Rust target.
    let compiler = cc::Build::new()
        .try_get_compiler()
        .unwrap_or_else(|error| {
            panic!("concat-vst3: native feature enabled but no C++ compiler found: {error}")
        });

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let bridge_cpp = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("bridge.cpp");

    println!("cargo:rerun-if-changed={}", bridge_cpp.display());
    println!("cargo:rerun-if-changed={}", pluginterfaces.display());

    let mut cpp_build = cc::Build::new();
    cpp_build
        .cpp(true)
        .compiler(compiler.path())
        .file(&bridge_cpp)
        // The SDK headers refer to each other as "pluginterfaces/base/...",
        // so both the SDK root and the base header directory must be on the
        // include path.
        .include(&sdk_dir)
        .include(&pluginterfaces)
        .include(&base_dir)
        .out_dir(&out_dir)
        .flag_if_supported("-std=c++17")
        .flag_if_supported("-Wall");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        cpp_build.flag("/EHsc");
        cpp_build.define("NOMINMAX", None);
    }

    cpp_build.compile("concat_vst3_bridge");

    println!("cargo:rustc-link-lib=static=concat_vst3_bridge");
    println!(
        "cargo:rustc-link-search=native={}",
        out_dir.display()
    );
}
