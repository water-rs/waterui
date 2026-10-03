//! Bundled font registration.
//!
//! The CLI stages the fonts `Water.toml` declares into the app bundle's
//! `fonts/` resource directory. Every font file there registers into the
//! process's font tables before the app body runs, so text that names the
//! family resolves on the first frame rather than after a lazy lookup.

use std::{collections::BTreeMap, path::PathBuf};

/// Successful process registrations owned by one native runtime.
///
/// Core Text's process font catalog is shared by every mount. Identical bytes
/// may arrive under different package paths, so a path-only inventory cannot
/// identify registrations. Keep the exact registered data, including every
/// face in a font collection, rather than treating a family name as identity.
#[derive(Default)]
pub struct FontRegistrations {
    registered: BTreeMap<Box<[u8]>, PathBuf>,
}

impl core::fmt::Debug for FontRegistrations {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FontRegistrations")
            .field("registered_paths", &self.registered.values())
            .finish()
    }
}

impl FontRegistrations {
    /// Registers new font data from this instance's resource directory.
    ///
    /// Successful registrations survive unmounting and are reused on remount.
    /// Original registered files must remain in place until process exit, as
    /// required by Core Text. Distinct data is submitted to Core Text even if
    /// it names an existing family: invalid data and catalog conflicts fail.
    /// A missing directory means the bundle declares no fonts.
    pub fn register_bundle_fonts(&mut self, resources: &waterui_core::ResourceContext) {
        let entries = match std::fs::read_dir(resources.fonts()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!(
                "cannot read font directory {}: {error}",
                resources.fonts().display()
            ),
        };
        for entry in entries {
            let path = entry.expect("font directory entry must be readable").path();
            // The CLI also stages its family-to-file JSON manifest here.
            if path
                .file_name()
                .is_some_and(|name| name == "waterui-fonts.json")
            {
                continue;
            }
            let metadata = std::fs::metadata(&path).unwrap_or_else(|error| {
                panic!(
                    "cannot read bundled font metadata {}: {error}",
                    path.display()
                )
            });
            if !metadata.is_file() {
                continue;
            }
            let data = std::fs::read(&path).unwrap_or_else(|error| {
                panic!("cannot read bundled font {}: {error}", path.display())
            });
            if self.registered.contains_key(data.as_slice()) {
                continue;
            }
            cocoa_ui::fonts::register_font(&path).unwrap_or_else(|error| {
                panic!(
                    "failed to register bundled font {}: {error}",
                    path.display()
                )
            });
            self.registered.insert(data.into_boxed_slice(), path);
        }
    }
}
