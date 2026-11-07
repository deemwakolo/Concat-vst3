// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Plugin registry: discovers installed VST3 plugins and caches them by
//! class ID.
//!
//! The registry is the single source of truth for what plugins are
//! available. It scans OS-standard VST3 folders at construction time and
//! indexes each discovered plugin by its class ID (FUID).

use crate::discovery;
use crate::errors::Error;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

/// One discovered plugin, enough to display in a UI and to instantiate.
#[derive(Clone, Debug)]
pub struct PluginDescriptor {
    /// The VST3 class ID (FUID) as a 32-hex-digit string.
    pub class_id: String,
    /// Human-readable plugin name.
    pub name: String,
    /// Human-readable vendor name.
    pub vendor: String,
    /// File path to the plugin module.
    pub path: PathBuf,
}

/// The registry of discovered plugins.
pub struct Registry {
    /// Plugins indexed by class ID FUID string.
    plugins: RwLock<HashMap<String, PluginDescriptor>>,
}

impl Registry {
    /// An empty registry, used when discovery itself fails.
    pub fn empty() -> Self {
        Self {
            plugins: RwLock::new(HashMap::new()),
        }
    }

    /// Scans OS-standard VST3 folders and builds the registry.
    ///
    /// On failure to scan a particular folder, that folder is skipped and
    /// scanning continues. A registry with no plugins is still valid.
    pub fn scan() -> Result<Self, Error> {
        let plugins = discovery::discover_plugins()?;
        let mut map = HashMap::new();
        for desc in &plugins {
            map.insert(desc.class_id.clone(), desc.clone());
        }
        Ok(Self {
            plugins: RwLock::new(map),
        })
    }

    /// Returns the descriptor for `class_id`, or None if not found.
    pub fn resolve(&self, class_id: &str) -> Option<PluginDescriptor> {
        self.plugins.read().unwrap().get(class_id).cloned()
    }

    /// Returns all discovered plugin descriptors.
    pub fn all(&self) -> Vec<PluginDescriptor> {
        self.plugins
            .read()
            .unwrap()
            .values()
            .cloned()
            .collect()
    }

    /// Returns the number of discovered plugins.
    pub fn len(&self) -> usize {
        self.plugins.read().unwrap().len()
    }

    /// Returns true if no plugins were discovered.
    pub fn is_empty(&self) -> bool {
        self.plugins.read().unwrap().is_empty()
    }

    /// Instantiates a plugin from `class_id` without initializing it.
    ///
    /// In native mode the class ID must be one returned by discovery. In stub
    /// mode no modules can be loaded, so this always reports the class as not
    /// found rather than pretending to create an instance.
    ///
    /// The caller must call [`crate::Plugin::initialize`] before processing.
    pub fn create_plugin(&self, class_id: &str) -> Result<crate::instance::Plugin, Error> {
        #[cfg(feature = "native")]
        {
            let module = self.load_module(class_id)?;
            let class_id_guid = crate::bridge::ClassId::from_fuid(class_id)
                .ok_or_else(|| Error::ClassNotFound(class_id.to_owned()))?;
            crate::instance::Plugin::create(module, &class_id_guid)
        }
        #[cfg(not(feature = "native"))]
        {
            Err(Error::ClassNotFound(class_id.to_owned()))
        }
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::scan().unwrap_or_else(|_| Self::empty())
    }
}

/// Native-only: load a module and create a plugin instance.
///
/// These methods are only available when the `native` feature is on.
#[cfg(feature = "native")]
impl Registry {
    /// Loads and returns the module for `class_id`.
    ///
    /// The returned [`crate::bridge::Module`] keeps the DLL loaded until it is
    /// dropped. A [`crate::Plugin`] takes ownership of that module so an
    /// instance cannot outlive its code.
    pub fn load_module(&self, class_id: &str) -> Result<crate::bridge::Module, Error> {
        let desc = self
            .resolve(class_id)
            .ok_or_else(|| Error::ClassNotFound(class_id.to_owned()))?;

        let path = desc.path.to_string_lossy();
        crate::bridge::Module::load(&path)
    }
}
