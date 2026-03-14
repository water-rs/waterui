//! Asset and font management for WaterUI projects.
//!
//! This module provides functionality to:
//! - Scan dependency crates for font declarations in `[package.metadata.waterui.assets]`
//! - Resolve fonts from local paths, remote URLs, or built-in registry
//! - Download remote fonts with caching
//! - Copy assets to platform-specific locations

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use cargo_metadata::PackageId;
use color_eyre::eyre::{self, Context, OptionExt};
use serde::{Deserialize, Serialize};
use smol::fs;
use smol::stream::StreamExt;
use tracing::{debug, info, warn};
use walkdir::WalkDir;
use waterui_assets::{AtomicWriteOutcome, download_remote_bytes, write_bytes_atomically};

use crate::project::Project;

mod pipeline;
mod unified;

/// Built-in font registry mapping font names to download URLs.
///
/// These fonts can be declared in Cargo.toml with just a name:
/// ```toml
/// [[package.metadata.waterui.assets.font]]
/// name = "Inter"
/// ```
///
/// Note: Icon pack fonts (Font Awesome, Material Icons, Lucide, etc.) should
/// NOT be in this registry. They should declare their fonts in their own
/// Cargo.toml with `remote_path` and `required-feature` fields.
const FONT_REGISTRY: &[(&str, &str)] = &[
    // Popular text fonts
    (
        "Inter",
        "https://github.com/rsms/inter/releases/download/v4.0/Inter-4.0.zip",
    ),
    (
        "Roboto",
        "https://github.com/googlefonts/roboto/releases/download/v2.138/roboto-android.zip",
    ),
    (
        "JetBrainsMono",
        "https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip",
    ),
    (
        "FiraCode",
        "https://github.com/tonsky/FiraCode/releases/download/6.2/Fira_Code_v6.2.zip",
    ),
    (
        "SourceCodePro",
        "https://github.com/adobe-fonts/source-code-pro/releases/download/2.042R-u%2F1.062R-i%2F1.026R-vf/OTF-source-code-pro-2.042R-u_1.062R-i.zip",
    ),
];
const HYDROLYSIS_WEB_DEFAULT_FONT_FAMILY: &str = "Inter";
const HYDROLYSIS_WEB_FONT_MANIFEST_FILE_NAME: &str = "waterui-fonts.json";

/// Font declaration from a crate's Cargo.toml metadata.
#[derive(Debug, Clone)]
pub struct FontDeclaration {
    /// Font family name (used as `font_family` in Text).
    pub name: String,
    /// Source of the font file.
    pub source: FontSource,
    /// Crate that declared this font.
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
    /// Font to download from a URL.
    Remote {
        /// URL to download the font from.
        url: String,
    },
    /// Font from the built-in registry.
    BuiltIn,
}

/// A resolved font with its absolute path.
#[derive(Debug, Clone)]
pub struct ResolvedFont {
    /// Font family name.
    pub name: String,
    /// Absolute path to the font file.
    pub path: PathBuf,
}

#[derive(Debug, Serialize)]
struct HydrolysisWebFontManifest {
    default_family: String,
    fonts: Vec<HydrolysisWebFontManifestEntry>,
}

#[derive(Debug, Serialize)]
struct HydrolysisWebFontManifestEntry {
    name: String,
    file_name: String,
}

/// Font metadata from Cargo.toml `[package.metadata.waterui.assets]`.
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
    /// Optional feature that must be enabled for this font to be included.
    /// If specified, the font will only be downloaded/bundled if this feature
    /// is enabled for the declaring package.
    #[serde(default, rename = "required-feature")]
    required_feature: Option<String>,
}

/// Scans all dependencies for font declarations in their Cargo.toml metadata.
///
/// Uses `cargo metadata` to find all packages and parse their
/// `[package.metadata.waterui.assets.font]` sections.
///
/// Fonts with a `required-feature` field will only be included if that feature
/// is enabled for the declaring package (checked via cargo metadata's resolved graph).
pub async fn scan_fonts(project: &Project) -> eyre::Result<Vec<FontDeclaration>> {
    let manifest_path = project.root().join("Cargo.toml");

    debug!("Scanning fonts from dependencies via cargo metadata");

    // Run cargo metadata to get all packages
    let metadata = smol::unblock({
        let manifest_path = manifest_path.clone();
        move || {
            cargo_metadata::MetadataCommand::new()
                .manifest_path(&manifest_path)
                .exec()
        }
    })
    .await
    .wrap_err("Failed to run cargo metadata")?;

    // Build map of package_id -> enabled features from resolved graph
    let enabled_features_map: HashMap<&PackageId, HashSet<&str>> = metadata
        .resolve
        .as_ref()
        .map(|resolve| {
            resolve
                .nodes
                .iter()
                .map(|node| (&node.id, node.features.iter().map(|f| f.as_str()).collect()))
                .collect()
        })
        .unwrap_or_default();

    let mut fonts = Vec::new();

    for package in &metadata.packages {
        // Skip if no waterui metadata
        let Some(waterui) = package.metadata.get("waterui") else {
            continue;
        };

        // Parse the metadata
        let waterui_meta: WaterUIMetadata = match serde_json::from_value(waterui.clone()) {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    "Failed to parse waterui metadata for {}: {}",
                    package.name, e
                );
                continue;
            }
        };

        // Get enabled features for this package from resolve
        let enabled_features = enabled_features_map
            .get(&package.id)
            .cloned()
            .unwrap_or_default();

        // Process font declarations
        for font_meta in waterui_meta.assets.font {
            // Skip if required feature is not enabled
            if let Some(required) = &font_meta.required_feature {
                if !enabled_features.contains(required.as_str()) {
                    debug!(
                        "Skipping font '{}': feature '{}' not enabled for {}",
                        font_meta.name, required, package.name
                    );
                    continue;
                }
            }

            let source = if let Some(local_path) = font_meta.local_path {
                let local_path = PathBuf::from(local_path);
                if local_path.is_absolute() {
                    warn!(
                        "Skipping font '{}': local_path must be relative (crate: {})",
                        font_meta.name, package.name
                    );
                    continue;
                }

                // Local path - resolve relative to crate root
                let crate_root = package
                    .manifest_path
                    .parent()
                    .ok_or_eyre("Package has no parent directory")?
                    .as_std_path()
                    .to_path_buf();

                FontSource::Local {
                    crate_root,
                    relative_path: local_path,
                }
            } else if let Some(url) = font_meta.remote_path {
                FontSource::Remote { url }
            } else {
                // Just name - use built-in registry
                FontSource::BuiltIn
            };

            fonts.push(FontDeclaration {
                name: font_meta.name,
                source,
                crate_name: package.name.to_string(),
            });
        }
    }

    info!("Found {} font declarations from dependencies", fonts.len());
    Ok(fonts)
}

/// Resolves and deduplicates font declarations.
///
/// Rules:
/// - Same `name` → keep only one font
/// - Priority: local > remote > built-in
/// - Downloads remote/built-in fonts and caches them
pub async fn resolve_fonts(declarations: Vec<FontDeclaration>) -> eyre::Result<Vec<ResolvedFont>> {
    // Group by name
    let mut by_name: HashMap<String, Vec<FontDeclaration>> = HashMap::new();
    for decl in declarations {
        by_name.entry(decl.name.clone()).or_default().push(decl);
    }

    let mut resolved = Vec::new();
    let cache_dir = cache_dir()?;

    for (name, decls) in by_name {
        // Sort by priority: Local > Remote > BuiltIn
        let mut sorted = decls;
        sorted.sort_by_key(|d| match &d.source {
            FontSource::Local { .. } => 0,
            FontSource::Remote { .. } => 1,
            FontSource::BuiltIn => 2,
        });

        // Take the first (highest priority)
        let decl = sorted.into_iter().next().unwrap();

        let path = match &decl.source {
            FontSource::Local {
                crate_root,
                relative_path,
            } => match resolve_local_font_path(crate_root, relative_path) {
                Ok(Some(full_path)) => full_path,
                Ok(None) => {
                    warn!(
                        "Font file not found: {} (declared by {})",
                        crate_root.join(relative_path).display(),
                        decl.crate_name
                    );
                    continue;
                }
                Err(e) => {
                    warn!(
                        "Skipping font '{}': invalid local path '{}' (declared by {}): {e}",
                        name,
                        relative_path.display(),
                        decl.crate_name
                    );
                    continue;
                }
            },
            FontSource::Remote { url } => download_font(&name, url, &cache_dir).await?,
            FontSource::BuiltIn => {
                // Look up in registry
                let url = FONT_REGISTRY
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, url)| *url);

                match url {
                    Some(url) => download_font(&name, url, &cache_dir).await?,
                    None => {
                        warn!(
                            "Font '{}' not found in built-in registry (declared by {})",
                            name, decl.crate_name
                        );
                        continue;
                    }
                }
            }
        };

        debug!("Resolved font '{}' -> {}", name, path.display());
        resolved.push(ResolvedFont { name, path });
    }

    info!("Resolved {} fonts", resolved.len());
    Ok(resolved)
}

/// Gets the cache directory for downloaded fonts.
fn cache_dir() -> eyre::Result<PathBuf> {
    let cache = dirs::home_dir()
        .ok_or_eyre("Could not determine home directory")?
        .join(".water")
        .join("fonts");
    Ok(cache)
}

fn resolve_local_font_path(
    crate_root: &Path,
    relative_path: &Path,
) -> eyre::Result<Option<PathBuf>> {
    let full_path = crate_root.join(relative_path);
    if !full_path.exists() {
        return Ok(None);
    }

    let canonical_root = crate_root
        .canonicalize()
        .wrap_err_with(|| format!("Failed to canonicalize crate root {}", crate_root.display()))?;
    let canonical_path = full_path
        .canonicalize()
        .wrap_err_with(|| format!("Failed to canonicalize font path {}", full_path.display()))?;

    if !canonical_path.starts_with(&canonical_root) {
        eyre::bail!(
            "path escapes crate root ({} -> {})",
            full_path.display(),
            canonical_path.display()
        );
    }

    Ok(Some(canonical_path))
}

/// Downloads a font from a URL and caches it.
///
/// If the font is already cached, returns the cached path.
async fn download_font(name: &str, url: &str, cache_dir: &Path) -> eyre::Result<PathBuf> {
    // Create cache directory if needed
    fs::create_dir_all(cache_dir).await?;

    // Use URL hash as filename to avoid conflicts
    let hash = sha256_hex(url);
    let extension = if url.ends_with(".zip") { "zip" } else { "ttf" };
    let cache_file = cache_dir.join(format!("{hash}.{extension}"));

    // If already cached, return the path
    let cache_len = match fs::metadata(&cache_file).await {
        Ok(metadata) => Some(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).wrap_err_with(|| {
                format!(
                    "Failed to read cached font metadata for '{}' at {}",
                    name,
                    cache_file.display()
                )
            });
        }
    };
    if let Some(cache_len) = cache_len {
        if cache_len == 0 {
            warn!(
                "Ignoring empty cached font '{}' at {}",
                name,
                cache_file.display()
            );
            let _ = fs::remove_file(&cache_file).await;
        } else {
            debug!("Font '{}' already cached at {}", name, cache_file.display());

            // For zip files, we need to find the actual font file
            if extension == "zip" {
                return find_font_in_extracted_zip(&cache_file, name).await;
            }
            return Ok(cache_file);
        }
    }

    info!("Downloading font '{}' from {}", name, url);
    let bytes = download_remote_bytes(url)
        .await
        .map_err(eyre::Report::new)
        .wrap_err_with(|| format!("Failed to download font from {url}"))?;

    match write_bytes_atomically(&cache_file, &bytes)
        .await
        .map_err(eyre::Report::new)
        .wrap_err_with(|| {
            format!(
                "Failed to finalize cache file for '{}' at {}",
                name,
                cache_file.display()
            )
        })? {
        AtomicWriteOutcome::Written => {}
        AtomicWriteOutcome::ReusedExisting => {
            debug!(
                "Font cache race detected for '{}', reusing {}",
                name,
                cache_file.display()
            );
        }
    }

    // For zip files, extract and find the font
    if extension == "zip" {
        return find_font_in_extracted_zip(&cache_file, name).await;
    }

    Ok(cache_file)
}

/// Finds a font file in an extracted zip archive.
async fn find_font_in_extracted_zip(zip_path: &Path, name: &str) -> eyre::Result<PathBuf> {
    let extract_dir = zip_path.with_extension("");

    // Extract if not already done
    if !extract_dir.exists() {
        fs::create_dir_all(&extract_dir).await?;

        let zip_path = zip_path.to_path_buf();
        let extract_dir_clone = extract_dir.clone();
        let zip_path_for_extraction = zip_path.clone();

        smol::unblock(move || {
            let file = std::fs::File::open(&zip_path_for_extraction)?;
            let mut archive = zip::ZipArchive::new(file)?;
            archive.extract(&extract_dir_clone)?;
            Ok::<_, eyre::Report>(())
        })
        .await?;

        // For Font Awesome archives, also copy icons.json to the fontawesome cache
        if let Err(e) = copy_fontawesome_icons_json(&extract_dir).await {
            warn!(
                "Failed to copy Font Awesome icons.json from {}: {e}",
                extract_dir.display()
            );
        }
    }

    remove_extracted_font_archive(zip_path).await?;

    // Find a font file (.ttf or .otf)
    let font_file = find_font_file(&extract_dir, name).await?;
    Ok(font_file)
}

async fn remove_extracted_font_archive(zip_path: &Path) -> eyre::Result<()> {
    match fs::remove_file(zip_path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).wrap_err_with(|| {
            format!(
                "Failed to remove extracted font archive at {}",
                zip_path.display()
            )
        }),
    }
}

/// Copies Font Awesome icons.json to the fontawesome cache directory.
///
/// This is needed by the fontawesome7 crate's build.rs to generate icon definitions.
async fn copy_fontawesome_icons_json(extract_dir: &Path) -> eyre::Result<()> {
    // Look for metadata/icons.json in the extracted archive
    let icons_json = find_file_recursive(extract_dir, "icons.json").await?;

    // Determine version from parent directory name (e.g., "fontawesome-free-7.1.0-desktop")
    let version = extract_fontawesome_version(extract_dir);

    // Copy to fontawesome cache directory
    let fontawesome_cache = dirs::home_dir()
        .ok_or_eyre("Could not determine home directory")?
        .join(".water")
        .join("cache")
        .join("fontawesome");

    fs::create_dir_all(&fontawesome_cache).await?;

    let dest = fontawesome_cache.join(format!("fontawesome-{version}-icons.json"));
    fs::copy(&icons_json, &dest).await?;

    debug!("Copied icons.json to {}", dest.display());
    Ok(())
}

/// Recursively finds a file by name in a directory.
async fn find_file_recursive(dir: &Path, filename: &str) -> eyre::Result<PathBuf> {
    let dir = dir.to_path_buf();
    let filename = filename.to_string();
    smol::unblock(move || {
        for entry in WalkDir::new(&dir) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name == filename)
            {
                return Ok(entry.into_path());
            }
        }
        eyre::bail!("File '{}' not found in {}", filename, dir.display())
    })
    .await
}

/// Extracts Font Awesome version from directory structure.
fn extract_fontawesome_version(extract_dir: &Path) -> String {
    // Try to find version from directory names like "fontawesome-free-7.1.0-desktop"
    if let Ok(entries) = std::fs::read_dir(extract_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("fontawesome-") {
                // Extract version: "fontawesome-free-7.1.0-desktop" -> "7.1.0"
                let parts: Vec<&str> = name.split('-').collect();
                for (i, part) in parts.iter().enumerate() {
                    if part.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                        return parts[i..]
                            .join("-")
                            .split('-')
                            .next()
                            .unwrap_or("7.1.0")
                            .to_string();
                    }
                }
            }
        }
    }
    "7.1.0".to_string() // Default fallback
}

/// Recursively finds a font file in a directory.
///
/// Supports TTF and OTF formats.
async fn find_font_file(dir: &Path, name: &str) -> eyre::Result<PathBuf> {
    let dir = dir.to_path_buf();
    let name = name.to_string();
    smol::unblock(move || {
        let mut candidates = Vec::new();
        let name_lower = name.to_lowercase();

        let style_keyword =
            extract_style_keyword(&name_lower).unwrap_or_else(|| "regular".to_string());

        for entry in WalkDir::new(&dir) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.into_path();
            let Some(ext) = path.extension() else {
                continue;
            };
            let ext = ext.to_string_lossy().to_lowercase();
            if ext != "ttf" && ext != "otf" {
                continue;
            }

            let file_name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();

            if file_name.contains(&style_keyword) {
                let name_parts: Vec<&str> = name_lower.split_whitespace().collect();
                let matches_base = name_parts
                    .iter()
                    .take(3)
                    .all(|part| file_name.contains(part));
                if matches_base {
                    return Ok(path);
                }
            }

            candidates.push(path);
        }

        candidates
            .into_iter()
            .next()
            .ok_or_else(|| eyre::eyre!("No font file found in zip for '{}'", name))
    })
    .await
}

/// Extract style keyword from font family name for matching.
///
/// Handles patterns like:
/// - "FontAwesome7Free-Solid" -> "solid"
/// - "FontAwesome7Free-Regular" -> "regular"
/// - "FontAwesome7Free-Brands" -> "brands"
fn extract_style_keyword(name: &str) -> Option<String> {
    // Check for common style suffixes
    let styles = [
        "solid", "regular", "brands", "light", "thin", "bold", "medium",
    ];
    for style in styles {
        if name.ends_with(style) || name.contains(&format!("-{style}")) {
            return Some(style.to_string());
        }
    }
    None
}

/// Computes SHA256 hash of a string as hex.
fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

/// Stage project assets for Apple packaging (Asset Catalog + raw resources).
pub async fn stage_project_assets_for_apple(
    project: &Project,
    dest_dir: &Path,
) -> eyre::Result<()> {
    unified::stage_for_apple(project, dest_dir).await
}

/// Stage project assets for Android packaging (res + assets/raw).
pub async fn stage_project_assets_for_android(
    project: &Project,
    backend_path: &Path,
) -> eyre::Result<()> {
    unified::stage_for_android(project, backend_path).await
}

/// Stage project assets for GTK4 packaging (resources + gresource bundle).
pub async fn stage_project_assets_for_gtk(
    project: &Project,
    resources_dir: &Path,
) -> eyre::Result<()> {
    unified::stage_for_gtk(project, resources_dir).await
}

pub fn scan_project_font_assets(project: &Project) -> eyre::Result<Vec<ResolvedFont>> {
    unified::scan_project_fonts(project)
}

/// Stage project assets for web packaging.
pub async fn stage_project_assets_for_web(project: &Project, site_root: &Path) -> eyre::Result<()> {
    pipeline::stage_for_web(project, site_root).await
}

/// Copies project assets to a destination directory.
///
/// Preserves directory structure within the assets folder.
pub async fn copy_project_assets(project: &Project, dest: &Path) -> eyre::Result<()> {
    let assets_dir = project.assets_dir();

    if !assets_dir.exists() {
        debug!("Assets directory does not exist: {}", assets_dir.display());
        return Ok(());
    }

    info!("Copying project assets from {}", assets_dir.display());

    copy_dir_recursive(&assets_dir, dest).await?;

    Ok(())
}

/// Copies fonts to a destination directory.
pub async fn copy_fonts(fonts: &[ResolvedFont], dest: &Path) -> eyre::Result<()> {
    fs::create_dir_all(dest).await?;

    for font in fonts {
        let file_name = font
            .path
            .file_name()
            .ok_or_eyre("Font path has no filename")?;
        let dest_path = dest.join(file_name);

        debug!(
            "Copying font {} -> {}",
            font.path.display(),
            dest_path.display()
        );
        fs::copy(&font.path, &dest_path).await?;
    }

    Ok(())
}

/// Stage fonts for Hydrolysis web runtime using the existing CLI font pipeline.
///
/// This always provisions a default text font so Web/WASM builds have a usable
/// fallback even when the project does not declare any fonts.
pub async fn stage_hydrolysis_web_fonts(project: &Project, site_root: &Path) -> eyre::Result<()> {
    let mut declarations = scan_fonts(project).await?;
    declarations.push(FontDeclaration {
        name: HYDROLYSIS_WEB_DEFAULT_FONT_FAMILY.to_string(),
        source: FontSource::BuiltIn,
        crate_name: "waterui-cli".to_string(),
    });

    let resolved_fonts = resolve_fonts(declarations).await?;
    let fonts_dest = site_root.join("fonts");
    copy_fonts(&resolved_fonts, &fonts_dest).await?;
    write_hydrolysis_web_font_manifest(&resolved_fonts, &fonts_dest).await?;
    Ok(())
}

async fn write_hydrolysis_web_font_manifest(
    fonts: &[ResolvedFont],
    fonts_dest: &Path,
) -> eyre::Result<()> {
    let mut manifest_fonts = Vec::with_capacity(fonts.len());
    let mut has_default_family = false;

    for font in fonts {
        let file_name = font
            .path
            .file_name()
            .ok_or_eyre("Font path has no filename")?
            .to_string_lossy()
            .into_owned();
        if font.name == HYDROLYSIS_WEB_DEFAULT_FONT_FAMILY {
            has_default_family = true;
        }
        manifest_fonts.push(HydrolysisWebFontManifestEntry {
            name: font.name.clone(),
            file_name,
        });
    }

    assert!(
        has_default_family,
        "hydrolysis web font staging must include default family `{HYDROLYSIS_WEB_DEFAULT_FONT_FAMILY}`"
    );

    let manifest = HydrolysisWebFontManifest {
        default_family: HYDROLYSIS_WEB_DEFAULT_FONT_FAMILY.to_string(),
        fonts: manifest_fonts,
    };
    let payload = serde_json::to_vec_pretty(&manifest)?;
    fs::write(
        fonts_dest.join(HYDROLYSIS_WEB_FONT_MANIFEST_FILE_NAME),
        payload,
    )
    .await?;
    Ok(())
}

/// Recursively copies a directory.
async fn copy_dir_recursive(src: &Path, dest: &Path) -> eyre::Result<()> {
    fs::create_dir_all(dest).await?;

    let mut entries = fs::read_dir(src).await?;
    while let Some(entry) = entries.next().await {
        let entry = entry?;
        let path = entry.path();
        let Some(file_name) = path.file_name() else {
            warn!("Skipping path without file name: {}", path.display());
            continue;
        };
        let dest_path = dest.join(file_name);

        if is_symlink(&path)? {
            warn!("Skipping symlink while copying assets: {}", path.display());
            continue;
        }

        if path.is_dir() {
            Box::pin(copy_dir_recursive(&path, &dest_path)).await?;
        } else {
            fs::copy(&path, &dest_path).await?;
        }
    }

    Ok(())
}

fn is_symlink(path: &Path) -> eyre::Result<bool> {
    Ok(std::fs::symlink_metadata(path)
        .wrap_err_with(|| format!("Failed to read metadata for {}", path.display()))?
        .file_type()
        .is_symlink())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_font_registry_has_entries() {
        assert!(!FONT_REGISTRY.is_empty());
        assert!(FONT_REGISTRY.iter().any(|(name, _)| *name == "Inter"));
        // Icon pack fonts should NOT be in the built-in registry
        assert!(
            !FONT_REGISTRY
                .iter()
                .any(|(name, _)| name.contains("Font Awesome"))
        );
        assert!(
            !FONT_REGISTRY
                .iter()
                .any(|(name, _)| name.contains("Material Design"))
        );
    }

    #[test]
    fn test_sha256_hex() {
        let hash = sha256_hex("hello");
        assert_eq!(hash.len(), 64); // SHA256 = 32 bytes = 64 hex chars
    }

    #[test]
    fn test_http_allowlist() {
        for url in [
            "http://localhost/font.ttf",
            "http://127.0.0.1:8080/font.ttf",
            "http://[::1]/font.ttf",
            "https://example.com/font.ttf",
        ] {
            assert!(
                waterui_assets::ensure_http_allowed(url).is_ok(),
                "expected to allow {url}"
            );
        }
    }

    #[test]
    fn test_http_rejects_non_loopback_and_prefix_bypass() {
        for url in [
            "http://example.com/font.ttf",
            "http://localhost.evil.com/font.ttf",
            "http://127.0.0.1.evil.com/font.ttf",
        ] {
            assert!(
                waterui_assets::ensure_http_allowed(url).is_err(),
                "expected to reject {url}"
            );
        }
    }

    #[test]
    fn test_resolve_local_font_path_rejects_escape() {
        let root = tempdir().expect("temp root");
        let outside = tempdir().expect("temp outside");
        let outside_font = outside.path().join("outside.ttf");
        fs::write(&outside_font, b"font").expect("write outside font");

        let rel_escape = Path::new("..").join(outside.path().file_name().expect("outside name"));
        let rel_escape = rel_escape.join("outside.ttf");

        let result = resolve_local_font_path(root.path(), &rel_escape);
        assert!(result.is_err(), "expected path traversal to be rejected");
    }

    #[test]
    fn test_resolve_local_font_path_accepts_inside_root() {
        let root = tempdir().expect("temp root");
        let inside_dir = root.path().join("fonts");
        fs::create_dir_all(&inside_dir).expect("create fonts dir");
        let font_path = inside_dir.join("inside.ttf");
        fs::write(&font_path, b"font").expect("write inside font");

        let resolved = resolve_local_font_path(root.path(), Path::new("fonts/inside.ttf"))
            .expect("resolve should succeed")
            .expect("font should exist");
        assert_eq!(resolved, font_path.canonicalize().expect("canonical path"));
    }
}
