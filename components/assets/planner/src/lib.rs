//! Shared asset discovery and planning for `WaterUI` applications.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use heck::ToSnakeCase;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use walkdir::WalkDir;
use waterui_assets_core::AssetKind;

mod color;
mod launch;

pub use color::{HexColor, InvalidHexColor};
pub use launch::{ColorScheme, LaunchConfig, LaunchPlan};

/// The `[theme]` section of `Water.toml`: the theme color slots.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeConfig {
    /// Window or page background color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<HexColor>,
    /// Main surface color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<HexColor>,
    /// Secondary surface color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_variant: Option<HexColor>,
    /// Border color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<HexColor>,
    /// Primary foreground color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground: Option<HexColor>,
    /// Muted foreground color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted_foreground: Option<HexColor>,
    /// Accent color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<HexColor>,
    /// Foreground color used on accent surfaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent_foreground: Option<HexColor>,
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

/// Symbol prefix for `include_bundle!` mount metadata statics.
///
/// `include_bundle!` emits one `#[used] static` per mounted directory whose
/// mangled name ends in `{BUNDLE_META_PREFIX}<mount>`; the CLI enumerates the
/// compiled artifact's symbol table and decodes the matching
/// [`BundleMountMeta`] payload.
pub const BUNDLE_META_PREFIX: &str = "waterui_meta_bundle_";

/// One bundle mount declared by `include_bundle!`, carried to the CLI as the
/// NUL-terminated payload of a `#[used]` static.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleMountMeta {
    /// Logical mount name; `"assets"` is the main application asset root.
    pub mount: String,
    /// Absolute path of the mounted directory at expansion time.
    pub path: PathBuf,
    /// Absolute path of the toolchain project that produces `path` — the
    /// frontend root for a web mount (`include_web!`), `None` for a plain
    /// `include_bundle!`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<PathBuf>,
}

impl BundleMountMeta {
    /// Leaf of the metadata symbol's demangled name.
    #[must_use]
    pub fn symbol_leaf(&self) -> String {
        format!("{BUNDLE_META_PREFIX}{}", rust_identifier(&self.mount))
    }

    /// Serialize as the symbol payload: JSON followed by a NUL terminator.
    ///
    /// # Panics
    ///
    /// Panics if serialization fails, which cannot happen for this type.
    #[must_use]
    pub fn to_payload(&self) -> Vec<u8> {
        let mut payload =
            serde_json::to_vec(self).expect("BundleMountMeta serialization cannot fail");
        payload.push(0);
        payload
    }

    /// Decode a symbol payload read from an artifact.
    ///
    /// A trailing NUL terminator is tolerated; anything else malformed is an
    /// error.
    ///
    /// # Errors
    ///
    /// Returns [`PlannerError::InvalidMountMeta`] when the payload is not a
    /// serialized [`BundleMountMeta`].
    pub fn from_payload(bytes: &[u8]) -> Result<Self, PlannerError> {
        let json = bytes.split(|byte| *byte == 0).next().unwrap_or_default();
        serde_json::from_slice(json).map_err(|source| PlannerError::InvalidMountMeta { source })
    }
}

/// Semantic role assigned to a planned asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetRole {
    /// Normal asset exposed through generated asset modules.
    Regular,
    /// Root-level application icon asset (`Icon.*`).
    AppIcon,
    /// Root-level launch screen artwork (`Launch.*`).
    LaunchImage,
}

impl AssetRole {
    /// The file stem that claims this role at the asset root, or `None` for
    /// a regular asset.
    #[must_use]
    pub const fn root_stem(self) -> Option<&'static str> {
        match self {
            Self::Regular => None,
            Self::AppIcon => Some("Icon"),
            Self::LaunchImage => Some("Launch"),
        }
    }

    /// The roles a file at the asset root can claim by its stem.
    const ROOT_ARTWORK: [Self; 2] = [Self::AppIcon, Self::LaunchImage];

    fn for_root_stem(stem: &str) -> Self {
        Self::ROOT_ARTWORK
            .into_iter()
            .find(|role| role.root_stem() == Some(stem))
            .unwrap_or(Self::Regular)
    }
}

/// The root-level artwork files seen so far while planning, by stem: each
/// role may be claimed by exactly one file.
#[derive(Default)]
struct RootArtwork(BTreeMap<&'static str, PathBuf>);

impl RootArtwork {
    fn claim(&mut self, role: AssetRole, path: &Path) -> Result<(), PlannerError> {
        let Some(stem) = role.root_stem() else {
            return Ok(());
        };
        self.0
            .insert(stem, path.to_path_buf())
            .map_or(Ok(()), |first| {
                Err(PlannerError::DuplicateArtwork {
                    stem,
                    first,
                    second: path.to_path_buf(),
                })
            })
    }
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
    /// Returns Rust module path segments for this asset, relative to its
    /// mount's generated module.
    #[must_use]
    pub fn module_segments(&self) -> Vec<String> {
        self.relative_path
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

impl BundleManifest {
    /// The root-level artwork claiming `role` (`Icon.*`, `Launch.*`), if the
    /// project provides one.
    #[must_use]
    pub fn root_artwork(&self, role: AssetRole) -> Option<&PlannedAsset> {
        self.assets.iter().find(|asset| asset.role == role)
    }
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
    /// A `waterui_meta_bundle_*` symbol payload is malformed.
    #[error("Invalid bundle mount metadata payload: {source}")]
    InvalidMountMeta {
        /// Underlying JSON decode error.
        source: serde_json::Error,
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
    /// Root-level artwork (`Icon.*`, `Launch.*`) is not a supported image.
    #[error("Root-level {stem}.* asset '{path}' must be a square raster image or SVG")]
    InvalidArtworkSource {
        /// The file stem that names the role.
        stem: &'static str,
        /// Invalid artwork source path.
        path: PathBuf,
    },
    /// More than one root-level file claims the same artwork role.
    #[error("Only one root-level {stem}.* asset is allowed, found '{first}' and '{second}'")]
    DuplicateArtwork {
        /// The file stem that names the role.
        stem: &'static str,
        /// First artwork path seen.
        first: PathBuf,
        /// Second artwork path seen.
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

/// Plans one mounted directory.
///
/// `mount` is `""` or `"assets"` for the main application asset root; root
/// artwork roles and unprefixed logical paths apply only to that main mount.
/// Every other mount prefixes its logical paths with the mount name and
/// yields only [`AssetRole::Regular`] assets.
///
/// # Errors
///
/// Returns [`PlannerError`] when `root` is not a directory or assets collide
/// by logical path, generated module path, or root artwork role.
pub fn plan_mount(root: &Path, mount: &str) -> Result<Vec<PlannedAsset>, PlannerError> {
    let main = mount.is_empty() || mount == "assets";
    let mount_name = if main { "" } else { mount };
    if !root.is_dir() {
        return Err(PlannerError::MissingMountRoot {
            name: mount.to_string(),
            path: root.to_path_buf(),
        });
    }
    let mut assets = Vec::new();
    let mut by_logical = BTreeMap::<String, PathBuf>::new();
    let mut by_module = BTreeMap::<String, PathBuf>::new();
    let mut root_artwork = RootArtwork::default();
    collect_mount_assets(
        mount_name,
        root,
        &mut assets,
        &mut by_logical,
        &mut by_module,
        &mut root_artwork,
    )?;
    assets.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    Ok(assets)
}

fn collect_mount_assets(
    mount_name: &str,
    root: &Path,
    assets: &mut Vec<PlannedAsset>,
    by_logical: &mut BTreeMap<String, PathBuf>,
    by_module: &mut BTreeMap<String, PathBuf>,
    root_artwork: &mut RootArtwork,
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

        let role = infer_role(mount_name, &relative_path, path, root_artwork)?;
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
    root_artwork: &mut RootArtwork,
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
    let role = AssetRole::for_root_stem(stem);
    let Some(stem) = role.root_stem() else {
        return Ok(AssetRole::Regular);
    };
    if !matches!(infer_kind(absolute_path), AssetKind::Image) {
        return Err(PlannerError::InvalidArtworkSource {
            stem,
            path: absolute_path.to_path_buf(),
        });
    }
    root_artwork.claim(role, absolute_path)?;
    Ok(role)
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
    fn plan_mount_marks_root_icon() {
        let temp = tempdir().expect("tempdir");
        let assets = temp.path().join("assets");
        fs::create_dir_all(&assets).expect("create assets");
        fs::write(assets.join("Icon.png"), b"png").expect("write icon");

        let planned = plan_mount(&assets, "").expect("plan mount");
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].role, AssetRole::AppIcon);
        assert_eq!(planned[0].logical_path, Path::new("Icon.png"));
    }

    #[test]
    fn plan_mount_marks_root_launch_artwork_and_rejects_a_second_one() {
        let temp = tempdir().expect("tempdir");
        let assets = temp.path().join("assets");
        fs::create_dir_all(assets.join("nested")).expect("create assets");
        fs::write(assets.join("Launch.svg"), b"<svg/>").expect("write launch");
        // A nested Launch.* is a regular asset, not a second claim.
        fs::write(assets.join("nested/Launch.png"), b"png").expect("write nested");

        let planned = plan_mount(&assets, "").expect("plan mount");
        let launch = planned
            .iter()
            .find(|asset| asset.role == AssetRole::LaunchImage)
            .expect("root Launch.svg is the launch image");
        assert!(launch.source_path.ends_with("Launch.svg"));
        assert!(!planned.iter().any(|asset| asset.role == AssetRole::AppIcon));

        fs::write(assets.join("Launch.png"), b"png").expect("write second launch");
        let error = plan_mount(&assets, "").expect_err("two root Launch.* files");
        assert!(matches!(
            error,
            PlannerError::DuplicateArtwork { stem: "Launch", .. }
        ));
    }

    #[test]
    fn plan_mount_prefixes_non_main_mounts() {
        let temp = tempdir().expect("tempdir");
        let dist = temp.path().join("dist");
        fs::create_dir_all(&dist).expect("create dist");
        fs::write(dist.join("app.js"), b"console.log(1)").expect("write asset");
        fs::write(dist.join("Icon.png"), b"png").expect("write icon");

        let planned = plan_mount(&dist, "web").expect("plan mount");
        assert_eq!(planned.len(), 2);
        for asset in &planned {
            assert_eq!(asset.mount, "web");
            assert_eq!(asset.role, AssetRole::Regular);
            assert_eq!(
                asset.logical_path,
                Path::new("web").join(&asset.relative_path)
            );
        }
    }

    #[test]
    fn plan_mount_rejects_missing_root() {
        let temp = tempdir().expect("tempdir");
        let error =
            plan_mount(&temp.path().join("missing"), "web").expect_err("missing root must error");
        assert!(matches!(error, PlannerError::MissingMountRoot { .. }));
    }

    #[test]
    fn bundle_mount_meta_payload_round_trips() {
        let meta = BundleMountMeta {
            mount: "web".to_string(),
            path: PathBuf::from("/abs/path/dist"),
            project: Some(PathBuf::from("/abs/path")),
        };
        assert_eq!(meta.symbol_leaf(), "waterui_meta_bundle_web");
        let payload = meta.to_payload();
        assert_eq!(payload.last(), Some(&0));
        let decoded = BundleMountMeta::from_payload(&payload).expect("decode payload");
        assert_eq!(decoded, meta);
        // The artifact reader already cuts at the first NUL; decoding without
        // the terminator must still work.
        let decoded = BundleMountMeta::from_payload(&payload[..payload.len() - 1])
            .expect("decode without terminator");
        assert_eq!(decoded, meta);
    }
}
