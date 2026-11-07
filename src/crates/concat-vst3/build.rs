// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Build script for concat-vst3.
//!
//! When the `native` feature is enabled and `VST3_SDK_DIR` points at an
//! unpacked Steinberg VST3 SDK, this compiles the C++ host bridge
//! (`src/bridge.cpp`) into a static library and tells Cargo to link it.
//! Otherwise the crate compiles to a stub: every plugin lookup reports
//! unavailable, and the crate still builds on machines without the SDK or a
//! C++ compiler.

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let native = env::var("CARGO_FEATURE_NATIVE").unwrap_or_default() == "1";

    if !native {
        return;
    }

    // Cargo does not track this by default. Without it, toggling the SDK path
    // can leave a cached stub-only build script result in place.
    println!("cargo:rerun-if-env-changed=VST3_SDK_DIR");

    let sdk_dir = match env::var("VST3_SDK_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => {
            println!(
                "cargo:warning=concat-vst3: native feature enabled but VST3_SDK_DIR is not set; \
                 building stub-only"
            );
            return;
        }
    };

    if !sdk_dir.is_dir() {
        println!(
            "cargo:warning=concat-vst3: VST3_SDK_DIR {:?} is not a directory; building stub-only",
            sdk_dir
        );
        return;
    }

    let compiler = match find_cpp_compiler() {
        Some(c) => c,
        None => {
            println!(
                "cargo:warning=concat-vst3: no C++ compiler found; building stub-only"
            );
            return;
        }
    };

    let pluginterfaces = sdk_dir.join("pluginterfaces");
    let base_dir = sdk_dir.join("base").join("source");

    if !pluginterfaces.is_dir() {
        println!(
            "cargo:warning=concat-vst3: VST3_SDK_DIR lacks pluginterfaces/; building stub-only"
        );
        return;
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let bridge_cpp = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("bridge.cpp");

    println!("cargo:rerun-if-changed={}", bridge_cpp.display());
    println!("cargo:rerun-if-changed={}", pluginterfaces.display());

    let mut cpp_build = cc::Build::new();
    cpp_build
        .cpp(true)
        .compiler(compiler)
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
    }

    cpp_build.compile("concat_vst3_bridge");

    println!("cargo:rustc-link-lib=static=concat_vst3_bridge");
    println!(
        "cargo:rustc-link-search=native={}",
        out_dir.display()
    );
}

fn find_cpp_compiler() -> Option<String> {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        which("cl.exe").ok()
    } else {
        for name in ["clang++", "g++", "c++"] {
            if let Ok(path) = which(name) {
                return Some(path);
            }
        }
        None
    }
}

fn which(name: &str) -> std::io::Result<String> {
    let output = std::process::Command::new("where").arg(name).output()?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout);
        return text
            .lines()
            .next()
            .filter(|l| !l.is_empty())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("{name} not found on PATH"),
                )
            });
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("{name} not found on PATH"),
    ))
}