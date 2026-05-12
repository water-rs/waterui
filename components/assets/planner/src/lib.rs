//! Shared asset discovery and planning for `WaterUI` applications.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use heck::ToSnakeCase;
use serde::{Deserialize, Serialize};
use syn::visit::Visit;
use syn::{File, LitStr, Token, parse::Parse, parse::ParseStream};
use thiserror::Error;
use walkdir::WalkDir;
use waterui_assets::AssetKind;

/// Theme color overrides discovered from asset metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeConfig {
    /// Window or page background color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Main surface color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    /// Secondary surface color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_variant: Option<String>,
    /// Border color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<String>,
    /// Primary foreground color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground: Option<String>,
    /// Muted foreground color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted_foreground: Option<String>,
    /// Accent color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    /// Foreground color used on accent surfaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent_foreground: Option<String>,
}

impl ThemeConfig {
    /// Returns whether no theme colors are configured.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.background.is_none()
            && self.surface.is_none()
            && self.surface_variant.is_none()
            && self.border.is_none()
            && self.foreground.is_none()
            && self.muted_foreground.is_none()
            && self.accent.is_none()
            && self.accent_foreground.is_none()
    }
}

/// Asset bundle mounted from an application or included bundle root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleMount {
    /// Logical mount name.
    pub name: String,
    /// Filesystem root for this mount.
    pub root: PathBuf,
}

/// Semantic role assigned to a planned asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetRole {
    /// Normal asset exposed through generated asset modules.
    Regular,
    /// Root-level application icon asset.
    AppIcon,
}

/// Asset discovered during bundle planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedAsset {
    /// Mount name, empty for the main application asset root.
    pub mount: String,
    /// Absolute source path on disk.
    pub source_path: PathBuf,
    /// Path relative to the asset mount.
    pub relative_path: PathBuf,
    /// Logical path exposed to generated code.
    pub logical_path: PathBuf,
    /// Inferred asset kind.
    pub kind: AssetKind,
    /// Inferred semantic role.
    pub role: AssetRole,
}

impl PlannedAsset {
    /// Returns Rust module path segments for this asset.
    #[must_use]
    pub fn module_segments(&self) -> Vec<String> {
        self.logical_path
            .parent()
            .into_iter()
            .flat_map(Path::components)
            .filter_map(|component| match component {
                std::path::Component::Normal(value) => value.to_str(),
                _ => None,
            })
            .map(rust_identifier)
            .collect()
    }

    /// Returns the generated Rust item name for this asset.
    ///
    /// # Panics
    ///
    /// Panics when the planned asset has no UTF-8 file stem.
    #[must_use]
    pub fn item_name(&self) -> String {
        let stem = self
            .logical_path
            .file_stem()
            .and_then(OsStr::to_str)
            .expect("planned asset must have a UTF-8 stem");
        rust_identifier(stem)
    }
}

/// Complete manifest describing assets discovered for a crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleManifest {
    /// Root directory of the application crate.
    pub crate_root: PathBuf,
    /// Main application assets directory.
    pub assets_root: PathBuf,
    /// Additional bundle mounts discovered from `include_bundle!`.
    pub mounts: Vec<BundleMount>,
    /// Planned assets across the main root and all mounts.
    pub assets: Vec<PlannedAsset>,
}

/// Errors produced while discovering and planning asset bundles.
#[derive(Debug, Error)]
pub enum PlannerError {
    /// Failed to read `Water.toml`.
    #[error("Failed to read Water.toml at '{path}': {source}")]
    ReadWaterToml {
        /// Path to `Water.toml`.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// Failed to parse `Water.toml`.
    #[error("Invalid Water.toml at '{path}': {source}")]
    InvalidWaterToml {
        /// Path to `Water.toml`.
        path: PathBuf,
        /// Underlying TOML parse error.
        source: toml::de::Error,
    },
    /// Failed to read a Rust source file.
    #[error("Failed to read Rust source '{path}': {source}")]
    ReadSource {
        /// Source file path.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// Failed to parse a Rust source file.
    #[error("Failed to parse Rust source '{path}': {source}")]
    ParseSource {
        /// Source file path.
        path: PathBuf,
        /// Underlying Rust parser error.
        source: syn::Error,
    },
    /// Duplicate `include_bundle!` mount name.
    #[error("include_bundle mount '{name}' already exists")]
    DuplicateMount {
        /// Duplicate mount name.
        name: String,
    },
    /// Included bundle namespace conflicts with the application asset namespace.
    #[error("include_bundle mount '{name}' conflicts with app assets namespace")]
    MountNamespaceConflict {
        /// Conflicting mount name.
        name: String,
    },
    /// Included bundle root does not exist.
    #[error("include_bundle mount '{name}' points to missing directory '{path}'")]
    MissingMountRoot {
        /// Mount name.
        name: String,
        /// Missing mount root path.
        path: PathBuf,
    },
    /// Two assets resolve to the same logical path.
    #[error("Asset path collision at '{logical_path}' between '{first}' and '{second}'")]
    AssetCollision {
        /// Colliding logical asset path.
        logical_path: String,
        /// First source path seen.
        first: PathBuf,
        /// Second source path seen.
        second: PathBuf,
    },
    /// Two assets resolve to the same generated Rust module path.
    #[error(
        "Generated Rust asset path collision at '{module_path}' between '{first}' and '{second}'"
    )]
    ModuleCollision {
        /// Colliding generated module path.
        module_path: String,
        /// First source path seen.
        first: PathBuf,
        /// Second source path seen.
        second: PathBuf,
    },
    /// Root-level icon asset is not a supported image.
    #[error("App icon source '{path}' must be a square raster image or SVG")]
    InvalidIconSource {
        /// Invalid icon source path.
        path: PathBuf,
    },
    /// More than one root-level icon asset was found.
    #[error("Only one root-level Icon.* asset is allowed, found '{first}' and '{second}'")]
    DuplicateIcon {
        /// First icon path seen.
        first: PathBuf,
        /// Second icon path seen.
        second: PathBuf,
    },
}

#[derive(Debug, Deserialize)]
struct WaterToml {
    #[serde(default)]
    package: WaterPackage,
}

#[derive(Debug, Deserialize, Default)]
struct WaterPackage {
    #[serde(default = "default_assets_path")]
    assets_path: String,
}

fn default_assets_path() -> String {
    "assets".to_string()
}

#[must_use]
/// Converts an arbitrary path segment into a Rust identifier.
pub fn rust_identifier(segment: &str) -> String {
    let mut ident = segment.to_snake_case();
    ident.retain(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    ident = ident.trim_matches('_').to_string();
    if ident.is_empty() {
        ident = "asset".to_string();
    }
    if ident
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_digit())
    {
        ident.insert_str(0, "asset_");
    }
    if is_rust_keyword(&ident) {
        format!("r#{ident}")
    } else {
        ident
    }
}

/// Reads the configured assets path from `Water.toml`.
///
/// # Errors
///
/// Returns [`PlannerError`] when `Water.toml` cannot be read or parsed.
pub fn read_assets_path(crate_root: &Path) -> Result<String, PlannerError> {
    let path = crate_root.join("Water.toml");
    let text = fs::read_to_string(&path).map_err(|source| PlannerError::ReadWaterToml {
        path: path.clone(),
        source,
    })?;
    let water: WaterToml =
        toml::from_str(&text).map_err(|source| PlannerError::InvalidWaterToml { path, source })?;
    Ok(water.package.assets_path)
}

/// Discovers `include_bundle!` mounts from Rust source files under `src`.
///
/// # Errors
///
/// Returns [`PlannerError`] when source files cannot be read or parsed, a mount
/// is duplicated, or a referenced mount directory is missing.
pub fn discover_bundle_mounts(crate_root: &Path) -> Result<Vec<BundleMount>, PlannerError> {
    let mut mounts = Vec::new();
    let src_root = crate_root.join("src");
    if !src_root.exists() {
        return Ok(mounts);
    }
    let mut seen = BTreeSet::new();
    for entry in WalkDir::new(&src_root).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_file() || path.extension() != Some(OsStr::new("rs")) {
            continue;
        }
        let source = fs::read_to_string(path).map_err(|source| PlannerError::ReadSource {
            path: path.to_path_buf(),
            source,
        })?;
        let file = syn::parse_file(&source).map_err(|source| PlannerError::ParseSource {
            path: path.to_path_buf(),
            source,
        })?;
        let mut visitor = IncludeBundleVisitor {
            crate_root,
            source_path: path,
            mounts: Vec::new(),
        };
        visitor.visit_file(&file);
        for mount in visitor.mounts {
            if !mount.root.is_dir() {
                return Err(PlannerError::MissingMountRoot {
                    name: mount.name,
                    path: mount.root,
                });
            }
            if !seen.insert(mount.name.clone()) {
                return Err(PlannerError::DuplicateMount { name: mount.name });
            }
            mounts.push(mount);
        }
    }
    Ok(mounts)
}

/// Plans all assets for a crate and its included bundles.
///
/// # Errors
///
/// Returns [`PlannerError`] when bundle discovery fails or assets collide by
/// logical path, generated module path, or app-icon role.
pub fn plan_bundle(crate_root: &Path, assets_path: &str) -> Result<BundleManifest, PlannerError> {
    let assets_root = crate_root.join(assets_path);
    let mounts = discover_bundle_mounts(crate_root)?;
    let mut assets = Vec::new();
    let mut by_logical = BTreeMap::<String, PathBuf>::new();
    let mut by_module = BTreeMap::<String, PathBuf>::new();
    let mut root_namespaces = BTreeSet::<String>::new();
    let mut app_icon_source: Option<PathBuf> = None;

    if assets_root.exists() {
        collect_mount_assets(
            "",
            &assets_root,
            &mut assets,
            &mut by_logical,
            &mut by_module,
            &mut root_namespaces,
            &mut app_icon_source,
        )?;
    }

    for mount in &mounts {
        let namespace = rust_identifier(&mount.name);
        if root_namespaces.contains(&namespace) {
            return Err(PlannerError::MountNamespaceConflict {
                name: mount.name.clone(),
            });
        }
        collect_mount_assets(
            &mount.name,
            &mount.root,
            &mut assets,
            &mut by_logical,
            &mut by_module,
            &mut root_namespaces,
            &mut app_icon_source,
        )?;
    }

    assets.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    Ok(BundleManifest {
        crate_root: crate_root.to_path_buf(),
        assets_root,
        mounts,
        assets,
    })
}

fn collect_mount_assets(
    mount_name: &str,
    root: &Path,
    assets: &mut Vec<PlannedAsset>,
    by_logical: &mut BTreeMap<String, PathBuf>,
    by_module: &mut BTreeMap<String, PathBuf>,
    root_namespaces: &mut BTreeSet<String>,
    app_icon_source: &mut Option<PathBuf>,
) -> Result<(), PlannerError> {
    if !root.exists() {
        return Ok(());
    }
    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        if is_ignored_metadata_file(file_name) {
            continue;
        }
        let relative_path = path
            .strip_prefix(root)
            .expect("asset path must remain under mount root")
            .to_path_buf();
        let logical_path = if mount_name.is_empty() {
            relative_path.clone()
        } else {
            Path::new(mount_name).join(&relative_path)
        };
        let logical_key = logical_path.to_string_lossy().replace('\\', "/");
        if let Some(first) = by_logical.insert(logical_key.clone(), path.to_path_buf()) {
            return Err(PlannerError::AssetCollision {
                logical_path: logical_key,
                first,
                second: path.to_path_buf(),
            });
        }

        let role = infer_role(mount_name, &relative_path, path, app_icon_source)?;
        let kind = infer_kind(path);
        let asset = PlannedAsset {
            mount: mount_name.to_string(),
            source_path: path.to_path_buf(),
            relative_path,
            logical_path,
            kind,
            role,
        };

        let module_key = asset_module_key(&asset);
        if let Some(first) = by_module.insert(module_key.clone(), path.to_path_buf()) {
            return Err(PlannerError::ModuleCollision {
                module_path: module_key,
                first,
                second: path.to_path_buf(),
            });
        }

        if mount_name.is_empty() {
            if let Some(first) = asset.module_segments().first() {
                root_namespaces.insert(first.clone());
            } else {
                root_namespaces.insert(asset.item_name());
            }
        }
        assets.push(asset);
    }
    Ok(())
}

fn asset_module_key(asset: &PlannedAsset) -> String {
    let mut parts = asset.module_segments();
    parts.push(asset.item_name());
    parts.join("::")
}

fn infer_role(
    mount_name: &str,
    relative_path: &Path,
    absolute_path: &Path,
    app_icon_source: &mut Option<PathBuf>,
) -> Result<AssetRole, PlannerError> {
    if !mount_name.is_empty() {
        return Ok(AssetRole::Regular);
    }
    if relative_path.components().count() != 1 {
        return Ok(AssetRole::Regular);
    }
    let Some(stem) = relative_path.file_stem().and_then(OsStr::to_str) else {
        return Ok(AssetRole::Regular);
    };
    if stem != "Icon" {
        return Ok(AssetRole::Regular);
    }
    if !matches!(infer_kind(absolute_path), AssetKind::Image) {
        return Err(PlannerError::InvalidIconSource {
            path: absolute_path.to_path_buf(),
        });
    }
    if let Some(first) = app_icon_source.replace(absolute_path.to_path_buf()) {
        return Err(PlannerError::DuplicateIcon {
            first,
            second: absolute_path.to_path_buf(),
        });
    }
    Ok(AssetRole::AppIcon)
}

fn infer_kind(path: &Path) -> AssetKind {
    let Some(ext) = path.extension().and_then(OsStr::to_str) else {
        return AssetKind::Data;
    };
    let lowered = ext.to_ascii_lowercase();
    AssetKind::from_extension(lowered.as_str())
}

fn is_ignored_metadata_file(name: &str) -> bool {
    matches!(name, ".DS_Store" | "Thumbs.db" | "desktop.ini")
}

fn is_rust_keyword(ident: &str) -> bool {
    matches!(
        ident,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
    )
}

struct IncludeBundleArgs {
    path: LitStr,
    mount: syn::Ident,
}

impl Parse for IncludeBundleArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let path: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;
        input.parse::<Token![as]>()?;
        input.parse::<Token![=]>()?;
        let mount: syn::Ident = input.parse()?;
        Ok(Self { path, mount })
    }
}

struct IncludeBundleVisitor<'a> {
    crate_root: &'a Path,
    source_path: &'a Path,
    mounts: Vec<BundleMount>,
}

impl Visit<'_> for IncludeBundleVisitor<'_> {
    fn visit_macro(&mut self, mac: &syn::Macro) {
        if mac.path.is_ident("include_bundle") {
            let args = mac
                .parse_body::<IncludeBundleArgs>()
                .unwrap_or_else(|error| {
                    panic!(
                        "Failed to parse include_bundle! in '{}': {error}",
                        self.source_path.display()
                    )
                });
            let root = self.crate_root.join(args.path.value());
            self.mounts.push(BundleMount {
                name: args.mount.to_string(),
                root,
            });
        }
        syn::visit::visit_macro(self, mac);
    }

    fn visit_file(&mut self, node: &File) {
        let _ = self.source_path;
        syn::visit::visit_file(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use tempfile::tempdir;

    #[test]
    fn rust_identifier_normalizes_segments() {
        assert_eq!(rust_identifier("hello-world"), "hello_world");
        assert_eq!(rust_identifier("123abc"), "asset_123abc");
        assert_eq!(rust_identifier("match"), "r#match");
    }

    #[test]
    fn ignores_known_metadata_files() {
        assert!(is_ignored_metadata_file(".DS_Store"));
        assert!(is_ignored_metadata_file("Thumbs.db"));
        assert!(!is_ignored_metadata_file(".well-known"));
    }

    #[test]
    fn plan_bundle_marks_root_icon() {
        let temp = tempdir().expect("tempdir");
        fs::write(
            temp.path().join("Water.toml"),
            "[package]\nname = 'Demo'\nbundle_identifier = 'dev.waterui.demo'\n",
        )
        .expect("write Water.toml");
        fs::create_dir_all(temp.path().join("assets")).expect("create assets");
        fs::write(temp.path().join("assets/Icon.png"), b"png").expect("write icon");

        let manifest = plan_bundle(temp.path(), "assets").expect("plan bundle");
        assert_eq!(manifest.assets.len(), 1);
        assert_eq!(manifest.assets[0].role, AssetRole::AppIcon);
    }

    #[test]
    fn plan_bundle_rejects_mount_namespace_conflict() {
        let temp = tempdir().expect("tempdir");
        fs::write(
            temp.path().join("Water.toml"),
            "[package]\nname = 'Demo'\nbundle_identifier = 'dev.waterui.demo'\n",
        )
        .expect("write Water.toml");
        fs::create_dir_all(temp.path().join("assets/web")).expect("create web dir");
        fs::write(temp.path().join("assets/web/index.html"), b"hi").expect("write asset");
        fs::create_dir_all(temp.path().join("dist")).expect("create dist");
        fs::write(temp.path().join("dist/app.js"), b"console.log(1)").expect("write dist asset");
        fs::create_dir_all(temp.path().join("src")).expect("create src");
        fs::write(
            temp.path().join("src/lib.rs"),
            "include_bundle!(\"dist\", as = web);",
        )
        .expect("write src");

        let error = plan_bundle(temp.path(), "assets").expect_err("must reject collision");
        assert!(matches!(error, PlannerError::MountNamespaceConflict { .. }));
    }
}
