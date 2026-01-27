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
use serde::Deserialize;
use smol::fs;
use smol::stream::StreamExt;
use tracing::{debug, info, warn};

use crate::project::Project;

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
                // Local path - resolve relative to crate root
                let crate_root = package
                    .manifest_path
                    .parent()
                    .ok_or_eyre("Package has no parent directory")?
                    .as_std_path()
                    .to_path_buf();

                FontSource::Local {
                    crate_root,
                    relative_path: PathBuf::from(local_path),
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
            } => {
                let full_path = crate_root.join(relative_path);
                if !full_path.exists() {
                    warn!(
                        "Font file not found: {} (declared by {})",
                        full_path.display(),
                        decl.crate_name
                    );
                    continue;
                }
                full_path
            }
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
    if cache_file.exists() {
        debug!("Font '{}' already cached at {}", name, cache_file.display());

        // For zip files, we need to find the actual font file
        if extension == "zip" {
            return find_font_in_extracted_zip(&cache_file, name).await;
        }
        return Ok(cache_file);
    }

    info!("Downloading font '{}' from {}", name, url);

    // Download the font using zenwave with redirect following
    use zenwave::{Client, Method, redirect::FollowRedirect};
    let mut client = FollowRedirect::new(zenwave::client());
    let response = client
        .method(Method::GET, url)
        .await
        .wrap_err_with(|| format!("Failed to download font from {url}"))?;

    if !response.status().is_success() {
        eyre::bail!(
            "Failed to download font: HTTP {} from {}",
            response.status(),
            url
        );
    }

    let bytes = response
        .into_body()
        .into_bytes()
        .await
        .wrap_err("Failed to read font data")?;

    fs::write(&cache_file, &bytes).await?;

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

        smol::unblock(move || {
            let file = std::fs::File::open(&zip_path)?;
            let mut archive = zip::ZipArchive::new(file)?;
            archive.extract(&extract_dir_clone)?;
            Ok::<_, eyre::Report>(())
        })
        .await?;

        // For Font Awesome archives, also copy icons.json to the fontawesome cache
        copy_fontawesome_icons_json(&extract_dir).await.ok();
    }

    // Find a font file (.ttf or .otf)
    let font_file = find_font_file(&extract_dir, name).await?;
    Ok(font_file)
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
    let mut entries = fs::read_dir(dir).await?;

    while let Some(entry) = entries.next().await {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            if let Ok(found) = Box::pin(find_file_recursive(&path, filename)).await {
                return Ok(found);
            }
        } else if path.file_name().is_some_and(|n| n == filename) {
            return Ok(path);
        }
    }

    eyre::bail!("File '{}' not found in {}", filename, dir.display())
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
    let mut entries = fs::read_dir(dir).await?;

    let mut candidates = Vec::new();
    let name_lower = name.to_lowercase();

    // Extract style keywords for Font Awesome-style names (e.g., "Font Awesome 7 Free Solid" -> "solid")
    // If no style found, default to "regular" for matching
    let style_keyword = extract_style_keyword(&name_lower).unwrap_or_else(|| "regular".to_string());

    while let Some(entry) = entries.next().await {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            if let Ok(found) = Box::pin(find_font_file(&path, name)).await {
                return Ok(found);
            }
        } else if let Some(ext) = path.extension() {
            let ext = ext.to_string_lossy().to_lowercase();
            // Support TTF and OTF formats
            if ext == "ttf" || ext == "otf" {
                let file_name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();

                // Match by style keyword (e.g., "solid" matches "Font Awesome 7 Free-Solid-900")
                // This is more precise than just matching the base name which could hit multiple files
                if file_name.contains(&style_keyword) {
                    // Also verify the base name matches to avoid false positives
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
        }
    }

    // Return any font file found
    candidates
        .into_iter()
        .next()
        .ok_or_else(|| eyre::eyre!("No font file found in zip for '{}'", name))
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

/// Recursively copies a directory.
async fn copy_dir_recursive(src: &Path, dest: &Path) -> eyre::Result<()> {
    fs::create_dir_all(dest).await?;

    let mut entries = fs::read_dir(src).await?;
    while let Some(entry) = entries.next().await {
        let entry = entry?;
        let path = entry.path();
        let file_name = path.file_name().unwrap();
        let dest_path = dest.join(file_name);

        if path.is_dir() {
            Box::pin(copy_dir_recursive(&path, &dest_path)).await?;
        } else {
            fs::copy(&path, &dest_path).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
