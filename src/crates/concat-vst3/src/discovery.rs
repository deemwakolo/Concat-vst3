// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! OS-specific VST3 plugin discovery.
//!
//! Scans the standard VST3 installation folders on each supported platform
//! and returns one descriptor for every audio-effect class exported by every
//! module it can load. Discovery is deliberately tolerant: an unreadable
//! entry, a 32-bit module, or a broken plug-in is logged and skipped so the
//! rest of the registry still populates.

use crate::errors::Error;
use crate::registry::PluginDescriptor;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// How far below a standard VST3 folder to look for vendor sub-folders.
const MAX_SCAN_DEPTH: usize = 4;

/// Discovers all VST3 audio-effect plugins installed on this machine.
pub fn discover_plugins() -> Result<Vec<PluginDescriptor>, Error> {
    let mut found: Vec<PluginDescriptor> = Vec::new();

    for folder in standard_vst3_folders() {
        match scan_folder(&folder) {
            Ok(mut descriptors) => found.append(&mut descriptors),
            Err(e) => {
                log::warn!("could not scan VST3 folder {:?}: {}", folder, e);
            }
        }
    }

    // The same class can be visible through more than one search root (for
    // example, a symlink or a duplicate installation). The class ID is the
    // stable identity; keep the first path.
    let mut seen: HashSet<String> = HashSet::new();
    found.retain(|descriptor| seen.insert(descriptor.class_id.clone()));

    Ok(found)
}

/// Returns the standard VST3 plugin folders for the current OS.
fn standard_vst3_folders() -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = Vec::new();

    if cfg!(target_os = "windows") {
        if let Ok(profile) = std::env::var("USERPROFILE") {
            folders.push(PathBuf::from(profile).join("Documents").join("VST3"));
        }
        if let Ok(common) = std::env::var("COMMONPROGRAMFILES") {
            folders.push(PathBuf::from(common).join("VST3"));
        }
        if let Ok(common_x86) = std::env::var("COMMONPROGRAMFILES(X86)") {
            folders.push(PathBuf::from(common_x86).join("VST3"));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            folders.push(PathBuf::from(local).join("Programs").join("Common").join("VST3"));
        }
        // Keep the well-known defaults even if the environment variable is
        // absent in a reduced service environment.
        folders.push(PathBuf::from("C:\\Program Files\\Common Files\\VST3"));
        folders.push(PathBuf::from(
            "C:\\Program Files (x86)\\Common Files\\VST3",
        ));
    } else if cfg!(target_os = "macos") {
        folders.push(
            std::env::var("HOME")
                .map(PathBuf::from)
                .map(|home| home.join("Library/Audio/Plug-Ins/VST3"))
                .unwrap_or_default(),
        );
        folders.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
    } else {
        if let Ok(home) = std::env::var("HOME") {
            let mut path = PathBuf::from(home);
            path.push(".vst3");
            folders.push(path);
        }
        folders.push(PathBuf::from("/usr/lib/vst3"));
        folders.push(PathBuf::from("/usr/local/lib/vst3"));
    }

    folders
}

/// Scans one search root, recursing through vendor directories.
fn scan_folder(folder: &Path) -> Result<Vec<PluginDescriptor>, Error> {
    let mut found = Vec::new();
    if !folder.is_dir() {
        return Ok(found);
    }
    scan_dir(folder, 0, &mut found);
    Ok(found)
}

/// Recursively scans `directory`, treating `.vst3` directories as bundles.
fn scan_dir(directory: &Path, depth: usize, found: &mut Vec<PluginDescriptor>) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }

    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("could not read VST3 directory {:?}: {}", directory, e);
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                log::warn!("could not read entry in {:?}: {}", directory, e);
                continue;
            }
        };

        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(e) => {
                log::warn!("could not stat VST3 entry {:?}: {}", entry.path(), e);
                continue;
            }
        };

        // Avoid symlink cycles. VST3 installations do not require symlinked
        // directories on the platforms Concat ships on.
        if file_type.is_symlink() {
            continue;
        }

        let path = entry.path();
        if file_type.is_dir() {
            if is_vst3_bundle(&path) {
                scan_bundle(&path, found);
            } else {
                scan_dir(&path, depth + 1, found);
            }
        } else if file_type.is_file() && is_plugin_binary(&path) {
            found.extend(descriptors_for_binary(&path));
        }
    }
}

/// Scans a `.vst3` bundle for its platform binary.
fn scan_bundle(bundle: &Path, found: &mut Vec<PluginDescriptor>) {
    let contents = bundle.join("Contents");
    if !contents.is_dir() {
        return;
    }
    // Contents/<platform>/<binary> is the normal layout. The extra depth also
    // covers bundles that keep the binary directly under Contents.
    scan_dir(&contents, 1, found);
}

/// Returns true if `path` is a VST3 bundle directory.
fn is_vst3_bundle(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("vst3"))
}

/// Returns true if `path` looks like a loadable plugin binary.
fn is_plugin_binary(path: &Path) -> bool {
    let extension = path.extension().and_then(|extension| extension.to_str());
    if extension.is_some_and(|extension| {
        extension.eq_ignore_ascii_case("vst3")
            || extension.eq_ignore_ascii_case("dll")
            || extension.eq_ignore_ascii_case("so")
    }) {
        return true;
    }

    // macOS bundles use Contents/MacOS/<binary> with no extension.
    if cfg!(target_os = "macos") {
        return path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "MacOS");
    }

    false
}

/// Enumerates every audio-effect class exported by one module binary.
#[cfg(feature = "native")]
fn descriptors_for_binary(binary_path: &Path) -> Vec<PluginDescriptor> {
    use crate::bridge;

    let Some(path) = binary_path.to_str() else {
        log::warn!("skipping VST3 binary with non-UTF-8 path: {:?}", binary_path);
        return Vec::new();
    };

    let module = match bridge::Module::load(path) {
        Ok(module) => module,
        Err(e) => {
            log::warn!("skipping VST3 module {:?}: {}", binary_path, e);
            return Vec::new();
        }
    };
    let count = match module.num_classes() {
        Ok(count) => count,
        Err(e) => {
            log::warn!("could not enumerate VST3 module {:?}: {}", binary_path, e);
            return Vec::new();
        }
    };
    let fallback_name = binary_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("Unknown VST3 plugin")
        .to_owned();

    let mut found = Vec::new();
    for index in 0..count {
        let info = match module.class_info_at(index) {
            Ok(info) => info,
            Err(e) => {
                log::warn!(
                    "could not read class {} from VST3 module {:?}: {}",
                    index,
                    binary_path,
                    e
                );
                continue;
            }
        };

        // The bridge already filters on kVstAudioEffectClass. Keep this check
        // as a second line of defense against controller classes.
        if info.category() != "Audio Module Class" {
            continue;
        }

        // VST3 uses the same top-level category for every audio module. The
        // sub-category is what distinguishes effects (`Fx|...`) from
        // instruments (`Instrument|...`) and generators.
        let is_audio_effect = info
            .sub_categories()
            .split('|')
            .next()
            .map(|first| first.trim() == "Fx")
            .unwrap_or(false);
        if !is_audio_effect {
            continue;
        }

        let class_id = info.class_id().to_fuid();
        let Some(parsed) = bridge::ClassId::from_fuid(&class_id) else {
            log::warn!(
                "VST3 module {:?} returned an unparseable class ID {:?}",
                binary_path,
                class_id
            );
            continue;
        };
        if parsed.to_fuid() != class_id {
            log::warn!(
                "VST3 module {:?} returned a non-round-tripping class ID {:?}",
                binary_path,
                class_id
            );
            continue;
        }

        let name = {
            let name = info.name();
            if name.trim().is_empty() {
                fallback_name.clone()
            } else {
                name
            }
        };

        found.push(PluginDescriptor {
            class_id,
            name,
            vendor: info.vendor(),
            path: binary_path.to_path_buf(),
        });
    }

    found
}

/// Stub builds have no VST3 SDK, so they report no plugins rather than fake
/// path-derived identities. This keeps stub builds truthful and compilable.
#[cfg(not(feature = "native"))]
fn descriptors_for_binary(_binary_path: &Path) -> Vec<PluginDescriptor> {
    Vec::new()
}
