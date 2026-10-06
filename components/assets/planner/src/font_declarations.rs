//! Font declarations a resolved Cargo graph carries in its packages'
//! `[[package.metadata.waterui.assets.font]]` tables.
//!
//! The `water` CLI reads them to stage an application's fonts, and a test
//! host reads them to register the crate-local files a test binary would
//! otherwise never see; both go through [`dependency_font_declarations`].

use std::collections::{HashMap, HashSet};
use std::path::{Component, PathBuf};

use cargo_metadata::{Metadata, PackageId};
use serde::Deserialize;
use thiserror::Error;

/// A font declaration — from `[[assets.font]]` in `Water.toml` or a crate's
/// `[package.metadata.waterui.assets.font]` Cargo.toml metadata.
#[derive(Debug, Clone)]
pub struct FontDeclaration {
    /// Font family name (used as `font_family` in Text).
    pub name: String,
    /// Source of the font file.
    pub source: FontSource,
    /// Crate or project that declared this font.
    pub crate_name: String,
}

/// Source of a font file.
#[derive(Debug, Clone)]
pub enum FontSource {
    /// Font bundled with the crate at a local path.
    Local {
        /// Absolute path to the crate root.
        crate_root: PathBuf,
        /// Relative path within the crate.
        relative_path: PathBuf,
    },
    /// Font that must be fetched out of band into the font cache.
    Remote {
        /// URL to fetch the font from when pre-seeding the cache.
        url: String,
    },
    /// Font from the built-in registry.
    BuiltIn,
}

/// A package's font declarations that cannot be read.
#[derive(Debug, Error)]
pub enum FontDeclarationError {
    /// The package's `[package.metadata.waterui.assets]` table does not parse.
    #[error("crate {crate_name} declares malformed `[package.metadata.waterui.assets]`: {source}")]
    Malformed {
        /// The declaring package.
        crate_name: String,
        /// Why the table does not parse.
        source: serde_json::Error,
    },
    /// A declaration's `local_path` is rooted instead of crate-relative.
    #[error(
        "crate {crate_name} declares font `{family}` with local_path `{}`; local_path must be \
         relative to the crate root",
        local_path.display()
    )]
    AbsoluteLocalPath {
        /// The declaring package.
        crate_name: String,
        /// The declared family.
        family: String,
        /// The rooted path it declares.
        local_path: PathBuf,
    },
    /// A declaration sets both `local_path` and `remote_path`.
    #[error(
        "crate {crate_name} declares font `{family}` with both local_path and remote_path; a \
         declaration has exactly one source"
    )]
    ConflictingSources {
        /// The declaring package.
        crate_name: String,
        /// The declared family.
        family: String,
    },
    /// The metadata was produced without dependency resolution (`--no-deps`),
    /// so a declaration's `required-feature` cannot be checked.
    #[error(
        "cargo metadata carries no resolved dependency graph (was it run with `--no-deps`?); \
         font declarations' `required-feature` cannot be checked without one"
    )]
    Unresolved,
}

/// The part of `[package.metadata.waterui]` this module reads; the table's
/// other keys belong to other readers.
#[derive(Debug, Deserialize)]
struct WaterUIMetadata {
    #[serde(default)]
    assets: AssetsMetadata,
}

#[derive(Debug, Default, Deserialize)]
struct AssetsMetadata {
    #[serde(default)]
    font: Vec<FontMetadata>,
}

#[derive(Debug, Deserialize)]
struct FontMetadata {
    name: String,
    #[serde(default)]
    local_path: Option<String>,
    #[serde(default)]
    remote_path: Option<String>,
    /// Feature that must be enabled on the declaring package for the font to
    /// be declared.
    #[serde(default, rename = "required-feature")]
    required_feature: Option<String>,
}

/// Every font declaration in `metadata`'s packages.
///
/// A declaration whose `required-feature` is not enabled on its package in
/// the resolved graph is left out. A `local_path` is resolved against the
/// declaring package's manifest directory.
///
/// # Errors
///
/// [`FontDeclarationError::Unresolved`] if `metadata` carries no resolved
/// graph (`cargo metadata --no-deps`),
/// [`FontDeclarationError::Malformed`] if a package's
/// `[package.metadata.waterui.assets]` does not parse,
/// [`FontDeclarationError::AbsoluteLocalPath`] if a declaration's
/// `local_path` is rooted, and [`FontDeclarationError::ConflictingSources`]
/// if a declaration sets both `local_path` and `remote_path`.
///
/// # Panics
///
/// Panics if `metadata` names a package manifest without a parent
/// directory, which `cargo metadata` never reports.
pub fn dependency_font_declarations(
    metadata: &Metadata,
) -> Result<Vec<FontDeclaration>, FontDeclarationError> {
    let resolve = metadata
        .resolve
        .as_ref()
        .ok_or(FontDeclarationError::Unresolved)?;
    let enabled_features: HashMap<&PackageId, HashSet<&str>> = resolve
        .nodes
        .iter()
        .map(|node| (&node.id, node.features.iter().map(|f| f.as_str()).collect()))
        .collect();

    let mut fonts = Vec::new();
    for package in &metadata.packages {
        let Some(waterui) = package.metadata.get("waterui") else {
            continue;
        };
        let parsed: WaterUIMetadata =
            serde_json::from_value(waterui.clone()).map_err(|source| {
                FontDeclarationError::Malformed {
                    crate_name: package.name.to_string(),
                    source,
                }
            })?;
        let features = enabled_features.get(&package.id);
        for font in parsed.assets.font {
            if let Some(required) = &font.required_feature
                && !features.is_some_and(|features| features.contains(required.as_str()))
            {
                continue;
            }
            let source = match (font.local_path, font.remote_path) {
                (Some(_), Some(_)) => {
                    return Err(FontDeclarationError::ConflictingSources {
                        crate_name: package.name.to_string(),
                        family: font.name,
                    });
                }
                (Some(local_path), None) => {
                    let relative_path = PathBuf::from(local_path);
                    // A rooted path in any flavor — `/x`, `\x`, `C:\x`, `C:x` —
                    // escapes the crate root on some platform.
                    if matches!(
                        relative_path.components().next(),
                        Some(Component::Prefix(_) | Component::RootDir)
                    ) {
                        return Err(FontDeclarationError::AbsoluteLocalPath {
                            crate_name: package.name.to_string(),
                            family: font.name,
                            local_path: relative_path,
                        });
                    }
                    let crate_root = package
                        .manifest_path
                        .parent()
                        .expect("a package's manifest path names a file inside its directory")
                        .as_std_path()
                        .to_path_buf();
                    FontSource::Local {
                        crate_root,
                        relative_path,
                    }
                }
                (None, Some(url)) => FontSource::Remote { url },
                (None, None) => FontSource::BuiltIn,
            };
            fonts.push(FontDeclaration {
                name: font.name,
                source,
                crate_name: package.name.to_string(),
            });
        }
    }
    Ok(fonts)
}
