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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted_foreground: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent_foreground: Option<String>,
}

impl ThemeConfig {
    #[must_use]
    pub fn is_empty(&self) -> bool {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleMount {
    pub name: String,
    pub root: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetRole {
    Regular,
    AppIcon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedAsset {
    pub mount: String,
    pub source_path: PathBuf,
    pub relative_path: PathBuf,
    pub logical_path: PathBuf,
    pub kind: AssetKind,
    pub role: AssetRole,
}

impl PlannedAsset {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleManifest {
    pub crate_root: PathBuf,
    pub assets_root: PathBuf,
    pub mounts: Vec<BundleMount>,
    pub assets: Vec<PlannedAsset>,
}

#[derive(Debug, Error)]
pub enum PlannerError {
    #[error("Failed to read Water.toml at '{path}': {source}")]
    ReadWaterToml {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Invalid Water.toml at '{path}': {source}")]
    InvalidWaterToml {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("Failed to read Rust source '{path}': {source}")]
    ReadSource {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to parse Rust source '{path}': {source}")]
    ParseSource { path: PathBuf, source: syn::Error },
    #[error("include_bundle mount '{name}' already exists")]
    DuplicateMount { name: String },
    #[error("include_bundle mount '{name}' conflicts with app assets namespace")]
    MountNamespaceConflict { name: String },
    #[error("include_bundle mount '{name}' points to missing directory '{path}'")]
    MissingMountRoot { name: String, path: PathBuf },
    #[error("Asset path collision at '{logical_path}' between '{first}' and '{second}'")]
    AssetCollision {
        logical_path: String,
        first: PathBuf,
        second: PathBuf,
    },
    #[error(
        "Generated Rust asset path collision at '{module_path}' between '{first}' and '{second}'"
    )]
    ModuleCollision {
        module_path: String,
        first: PathBuf,
        second: PathBuf,
    },
    #[error("App icon source '{path}' must be a square raster image or SVG")]
    InvalidIconSource { path: PathBuf },
    #[error("Only one root-level Icon.* asset is allowed, found '{first}' and '{second}'")]
    DuplicateIcon { first: PathBuf, second: PathBuf },
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
        let as_ident: syn::Ident = input.parse()?;
        if as_ident != "as" {
            return Err(syn::Error::new(as_ident.span(), "expected `as`"));
        }
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
