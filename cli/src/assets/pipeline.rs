use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use color_eyre::eyre::{self, Context, OptionExt};
use image::ImageEncoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use smol::fs;
use smol::stream::StreamExt;
use tracing::warn;

use crate::project::Project;
use crate::utils::{command, copy_file, which};

const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "avif", "bmp", "gif", "tif", "tiff", "svg", "pdf",
];
const GENERATED_APPLE_XCASSETS: &str = "WaterUIAssets.xcassets";
const APPLE_RESERVED_ASSET_NAMES: &[&str] = &["AppIcon", "AccentColor"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OptimizationProfile {
    Lossless,
    Lossy,
}

impl OptimizationProfile {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Lossless => "lossless",
            Self::Lossy => "lossy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ImageRules {
    lossy: bool,
}

#[derive(Debug, Clone)]
struct RawAsset {
    source: PathBuf,
    relative: PathBuf,
}

#[derive(Debug, Clone)]
struct AppleImageVariant {
    source: PathBuf,
    output_name: String,
    scale: String,
    dark: bool,
    profile: OptimizationProfile,
}

#[derive(Debug, Clone)]
struct AppleImageSet {
    name: String,
    variants: Vec<AppleImageVariant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppleCatalogSetKind {
    AppIconSet,
    ColorSet,
}

#[derive(Debug, Clone)]
struct AppleCatalogSet {
    source_dir: PathBuf,
    output_dir_name: String,
    kind: AppleCatalogSetKind,
    profile: OptimizationProfile,
}

#[derive(Debug, Clone)]
struct AndroidDrawable {
    source: PathBuf,
    qualifier: String,
    output_name: String,
    profile: OptimizationProfile,
}

#[derive(Debug, Clone)]
struct GtkImage {
    source: PathBuf,
    relative: PathBuf,
    profile: OptimizationProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AndroidGeneratedManifest {
    version: u8,
    files: Vec<String>,
}

/// A single validation issue found during asset scanning.
#[derive(Debug, Clone)]
struct ValidationIssue {
    path: PathBuf,
    message: String,
    suggestion: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ScannedAssets {
    raw_assets: Vec<RawAsset>,
    apple_imagesets: Vec<AppleImageSet>,
    apple_catalog_sets: Vec<AppleCatalogSet>,
    android_drawables: Vec<AndroidDrawable>,
    gtk_images: Vec<GtkImage>,
    errors: Vec<ValidationIssue>,
    warnings: Vec<ValidationIssue>,
}

impl ScannedAssets {
    fn ensure_valid(self) -> eyre::Result<Self> {
        if self.errors.is_empty() {
            return Ok(self);
        }

        let mut message = String::from("Asset layout validation failed:\n");
        for issue in &self.errors {
            let _ = writeln!(message, "- {}: {}", issue.path.display(), issue.message);
            if let Some(suggestion) = &issue.suggestion {
                let _ = writeln!(message, "  suggestion: {suggestion}");
            }
        }
        eyre::bail!(message);
    }
}

pub async fn stage_for_apple(project: &Project, dest_dir: &Path) -> eyre::Result<()> {
    let scanned = scan_assets(&project.assets_dir()).await?.ensure_valid()?;

    // Recreate generated resource roots to avoid stale entries.
    let raw_dest = dest_dir.join("raw");
    let xcassets_dest = dest_dir.join(GENERATED_APPLE_XCASSETS);
    if raw_dest.exists() {
        fs::remove_dir_all(&raw_dest).await?;
    }
    if xcassets_dest.exists() {
        fs::remove_dir_all(&xcassets_dest).await?;
    }

    for raw in &scanned.raw_assets {
        let dest = raw_dest.join(&raw.relative);
        copy_with_optimization(
            &raw.source,
            &dest,
            OptimizationProfile::Lossless,
            "apple",
            raw.relative.to_string_lossy().as_ref(),
        )
        .await?;
    }

    if !scanned.apple_imagesets.is_empty() || !scanned.apple_catalog_sets.is_empty() {
        let has_user_app_icon = scanned
            .apple_catalog_sets
            .iter()
            .any(|set| set.output_dir_name == "AppIcon.appiconset")
            || has_named_directory_under(dest_dir, "AppIcon.appiconset").await?;
        let has_user_accent_color = scanned
            .apple_catalog_sets
            .iter()
            .any(|set| set.output_dir_name == "AccentColor.colorset")
            || has_named_directory_under(dest_dir, "AccentColor.colorset").await?;

        fs::create_dir_all(&xcassets_dest).await?;
        write_apple_root_contents(&xcassets_dest).await?;

        if !has_user_app_icon {
            warn!(
                "No AppIcon.appiconset found. Generating placeholder icon set in {}",
                xcassets_dest.display()
            );
            write_apple_placeholder_app_icon(&xcassets_dest).await?;
        }
        if !has_user_accent_color {
            warn!(
                "No AccentColor.colorset found. Generating placeholder accent color in {}",
                xcassets_dest.display()
            );
            write_apple_placeholder_accent_color(&xcassets_dest).await?;
        }
    }

    for catalog_set in &scanned.apple_catalog_sets {
        let set_dest = xcassets_dest.join(&catalog_set.output_dir_name);
        copy_apple_catalog_set(catalog_set, &set_dest).await?;
    }

    for set in &scanned.apple_imagesets {
        let set_dest = xcassets_dest.join(format!("{}.imageset", set.name));
        fs::create_dir_all(&set_dest).await?;

        for variant in &set.variants {
            let dest = set_dest.join(&variant.output_name);
            let meta = format!("{}:{}:{}", set.name, variant.scale, variant.dark);
            copy_with_optimization(&variant.source, &dest, variant.profile, "apple", &meta).await?;
        }

        write_apple_imageset_contents(&set_dest, set).await?;
    }

    Ok(())
}

pub async fn stage_for_android(project: &Project, backend_path: &Path) -> eyre::Result<()> {
    let scanned = scan_assets(&project.assets_dir()).await?.ensure_valid()?;

    let assets_raw_dest = backend_path.join("app/src/main/assets/raw");
    let android_res_root = backend_path.join("app/src/main/res");
    let android_manifest = android_res_root.join(".water-assets-manifest.json");

    cleanup_android_generated_drawables(&android_res_root, &android_manifest).await?;

    if assets_raw_dest.exists() {
        fs::remove_dir_all(&assets_raw_dest).await?;
    }

    for raw in &scanned.raw_assets {
        let dest = assets_raw_dest.join(&raw.relative);
        copy_with_optimization(
            &raw.source,
            &dest,
            OptimizationProfile::Lossless,
            "android",
            raw.relative.to_string_lossy().as_ref(),
        )
        .await?;
    }

    let mut duplicate_guard = BTreeSet::new();
    let mut generated_files = Vec::new();
    for drawable in &scanned.android_drawables {
        let key = format!("{}/{}", drawable.qualifier, drawable.output_name);
        if !duplicate_guard.insert(key.clone()) {
            eyre::bail!("Android drawable name collision after normalization: {key}");
        }

        let dest = backend_path
            .join("app/src/main/res")
            .join(&drawable.qualifier)
            .join(&drawable.output_name);
        copy_with_optimization(
            &drawable.source,
            &dest,
            drawable.profile,
            "android",
            &drawable.qualifier,
        )
        .await?;
        generated_files.push(format!("{}/{}", drawable.qualifier, drawable.output_name));
    }

    write_android_generated_manifest(&android_manifest, &generated_files).await?;

    Ok(())
}

pub async fn stage_for_gtk(project: &Project, resources_dir: &Path) -> eyre::Result<()> {
    let scanned = scan_assets(&project.assets_dir()).await?.ensure_valid()?;

    let raw_dest = resources_dir.join("raw");
    let images_dest = resources_dir.join("images");
    if raw_dest.exists() {
        fs::remove_dir_all(&raw_dest).await?;
    }
    if images_dest.exists() {
        fs::remove_dir_all(&images_dest).await?;
    }

    let mut resource_entries = Vec::new();

    for raw in &scanned.raw_assets {
        let dest = raw_dest.join(&raw.relative);
        copy_with_optimization(
            &raw.source,
            &dest,
            OptimizationProfile::Lossless,
            "gtk4",
            raw.relative.to_string_lossy().as_ref(),
        )
        .await?;
        resource_entries.push(path_for_gresource(&dest, resources_dir)?);
    }

    for image in &scanned.gtk_images {
        let dest = images_dest.join(&image.relative);
        copy_with_optimization(
            &image.source,
            &dest,
            image.profile,
            "gtk4",
            image.relative.to_string_lossy().as_ref(),
        )
        .await?;
        resource_entries.push(path_for_gresource(&dest, resources_dir)?);
    }

    if !resource_entries.is_empty() {
        write_gresource_manifest(resources_dir, &resource_entries).await?;
        compile_gresource(resources_dir).await?;
    } else {
        remove_file_if_exists(resources_dir.join("resources.gresource.xml")).await?;
        remove_file_if_exists(resources_dir.join("resources.gresource")).await?;
    }

    Ok(())
}

pub async fn stage_for_web(project: &Project, site_root: &Path) -> eyre::Result<()> {
    let source = project.assets_dir();
    if !source.exists() {
        return Ok(());
    }

    let destination = site_root.join(project.assets_path());
    if destination.exists() {
        fs::remove_dir_all(&destination).await?;
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::create_dir_all(&destination).await?;

    smol::unblock(move || {
        let mut options = fs_extra::dir::CopyOptions::new();
        options.copy_inside = true;
        options.overwrite = true;
        fs_extra::dir::copy(&source, &destination, &options)
            .map(|_| ())
            .map_err(|error| {
                eyre::eyre!(
                    "Failed to copy web assets from {} to {}: {error}",
                    source.display(),
                    destination.display()
                )
            })
    })
    .await
}

async fn scan_assets(assets_dir: &Path) -> eyre::Result<ScannedAssets> {
    let mut scanned = ScannedAssets::default();

    if !assets_dir.exists() {
        return Ok(scanned);
    }

    let mut entries = fs::read_dir(assets_dir).await?;
    while let Some(entry) = entries.next().await {
        let entry = entry?;
        let path = entry.path();
        let file_name = file_name(&path)?;
        if is_symlink(&path).await? {
            scanned.errors.push(ValidationIssue {
                path: path.clone(),
                message: "Symlinks are not allowed in assets root".to_string(),
                suggestion: Some("Replace symlink with regular files/directories".to_string()),
            });
            continue;
        }

        if is_placeholder_file_name(file_name.as_str()) {
            continue;
        }

        if path.is_file() {
            if is_placeholder_file_name(file_name.as_str()) {
                continue;
            }

            scanned.errors.push(ValidationIssue {
                path: path.clone(),
                message: "Top-level files are not allowed in assets root".to_string(),
                suggestion: Some(
                    "Move non-image files to assets/raw/ and images to assets/images/*.imageset/ (or AppIcon.appiconset / AccentColor.colorset)"
                        .to_string(),
                ),
            });
            continue;
        }

        match file_name.as_str() {
            "raw" => collect_raw_assets(&path, Path::new(""), &mut scanned, false).await?,
            "fonts" => collect_raw_assets(&path, Path::new("fonts"), &mut scanned, false).await?,
            "images" => collect_images(&path, ImageRules { lossy: false }, &mut scanned).await?,
            "images-lossy" => {
                collect_images(&path, ImageRules { lossy: true }, &mut scanned).await?
            }
            _ => {
                scanned.errors.push(ValidationIssue {
                    path: path.clone(),
                    message: format!("Unknown assets top-level directory '{file_name}'"),
                    suggestion: Some(
                        "Allowed top-level directories: raw/, fonts/, images/, images-lossy/"
                            .to_string(),
                    ),
                });
            }
        }
    }

    Ok(scanned)
}

async fn collect_raw_assets(
    root: &Path,
    prefix: &Path,
    scanned: &mut ScannedAssets,
    lossy: bool,
) -> eyre::Result<()> {
    if is_symlink(root).await? {
        scanned.errors.push(ValidationIssue {
            path: root.to_path_buf(),
            message: "Symlinks are not allowed for raw/fonts asset directories".to_string(),
            suggestion: Some("Replace symlink with a regular directory".to_string()),
        });
        return Ok(());
    }

    let files = list_files_recursive(root).await?;
    for file in files {
        if is_image_file(&file) {
            scanned.errors.push(ValidationIssue {
                path: file.clone(),
                message: "Image files are not allowed in raw/fonts directories".to_string(),
                suggestion: Some(
                    "Move images to assets/images/*.imageset/ (Apple), assets/images/AppIcon.appiconset/, assets/images/AccentColor.colorset/, assets/images/android/drawable-*/ (Android), or assets/images/gtk/"
                        .to_string(),
                ),
            });
            continue;
        }
        let rel = file
            .strip_prefix(root)
            .wrap_err_with(|| format!("Failed to relativize {}", file.display()))?;
        let rel = prefix.join(rel);
        scanned.raw_assets.push(RawAsset {
            source: file,
            relative: rel,
        });
    }

    if lossy {
        scanned.warnings.push(ValidationIssue {
            path: root.to_path_buf(),
            message: "lossy optimization flag ignored for raw resources".to_string(),
            suggestion: None,
        });
    }

    Ok(())
}

async fn collect_images(
    root: &Path,
    rules: ImageRules,
    scanned: &mut ScannedAssets,
) -> eyre::Result<()> {
    if is_symlink(root).await? {
        scanned.errors.push(ValidationIssue {
            path: root.to_path_buf(),
            message: "Symlinks are not allowed for images directories".to_string(),
            suggestion: Some("Replace symlink with a regular directory".to_string()),
        });
        return Ok(());
    }

    let mut entries = fs::read_dir(root).await?;
    while let Some(entry) = entries.next().await {
        let entry = entry?;
        let path = entry.path();
        let name = file_name(&path)?;
        if is_symlink(&path).await? {
            scanned.errors.push(ValidationIssue {
                path: path.clone(),
                message: "Symlinks are not allowed under assets/images".to_string(),
                suggestion: Some("Replace symlink with a regular directory or file".to_string()),
            });
            continue;
        }

        if path.is_file() {
            if is_placeholder_file_name(name.as_str()) {
                continue;
            }

            scanned.errors.push(ValidationIssue {
                path: path.clone(),
                message: "Image files cannot live directly under assets/images/".to_string(),
                suggestion: Some(
                    "Use assets/images/<name>.imageset/, AppIcon.appiconset, AccentColor.colorset, assets/images/android/drawable-*/, or assets/images/gtk/"
                        .to_string(),
                ),
            });
            continue;
        }

        if name.ends_with(".imageset") {
            collect_apple_imageset(&path, rules, scanned).await?;
            continue;
        }
        if name.ends_with(".appiconset") {
            collect_apple_catalog_set(&path, AppleCatalogSetKind::AppIconSet, rules, scanned)
                .await?;
            continue;
        }
        if name.ends_with(".colorset") {
            collect_apple_catalog_set(&path, AppleCatalogSetKind::ColorSet, rules, scanned).await?;
            continue;
        }

        match name.as_str() {
            "android" => collect_android_drawables(&path, rules, scanned).await?,
            "gtk" => collect_gtk_images(&path, rules, scanned).await?,
            _ => scanned.errors.push(ValidationIssue {
                path: path.clone(),
                message: format!("Unknown images directory '{name}'"),
                suggestion: Some(
                    "Allowed under images/: *.imageset, AppIcon.appiconset, AccentColor.colorset, android/, gtk/"
                        .to_string(),
                ),
            }),
        }
    }
    Ok(())
}

async fn collect_apple_catalog_set(
    set_dir: &Path,
    kind: AppleCatalogSetKind,
    rules: ImageRules,
    scanned: &mut ScannedAssets,
) -> eyre::Result<()> {
    if is_symlink(set_dir).await? {
        scanned.errors.push(ValidationIssue {
            path: set_dir.to_path_buf(),
            message: "Symlinks are not allowed for Apple catalog set directories".to_string(),
            suggestion: Some("Replace symlink with a regular directory".to_string()),
        });
        return Ok(());
    }

    let output_dir_name = file_name(set_dir)?;
    let expected_name = match kind {
        AppleCatalogSetKind::AppIconSet => "AppIcon.appiconset",
        AppleCatalogSetKind::ColorSet => "AccentColor.colorset",
    };
    if output_dir_name != expected_name {
        scanned.errors.push(ValidationIssue {
            path: set_dir.to_path_buf(),
            message: format!(
                "Unsupported Apple catalog set name '{output_dir_name}' for this asset type"
            ),
            suggestion: Some(format!(
                "Rename to '{expected_name}' to match Xcode build settings"
            )),
        });
        return Ok(());
    }

    let files = list_files_recursive(set_dir).await?;
    if files.is_empty() {
        scanned.errors.push(ValidationIssue {
            path: set_dir.to_path_buf(),
            message: "Empty Apple catalog set directory".to_string(),
            suggestion: Some("Add Contents.json and required catalog files".to_string()),
        });
        return Ok(());
    }

    let mut has_contents_json = false;
    for file in files {
        if file
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(is_placeholder_file_name)
        {
            continue;
        }

        let rel = file
            .strip_prefix(set_dir)
            .wrap_err_with(|| format!("Failed to relativize {}", file.display()))?;
        if rel.components().count() != 1 {
            scanned.errors.push(ValidationIssue {
                path: file.clone(),
                message: "Nested subdirectories are not supported in Apple catalog sets"
                    .to_string(),
                suggestion: Some(
                    "Flatten files directly under the *.appiconset/*.colorset directory"
                        .to_string(),
                ),
            });
            continue;
        }

        if rel
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name == "Contents.json")
        {
            has_contents_json = true;
        }
    }

    if !has_contents_json {
        scanned.errors.push(ValidationIssue {
            path: set_dir.to_path_buf(),
            message: "Apple catalog set must include Contents.json".to_string(),
            suggestion: Some("Add Contents.json generated by Xcode".to_string()),
        });
        return Ok(());
    }

    scanned.apple_catalog_sets.push(AppleCatalogSet {
        source_dir: set_dir.to_path_buf(),
        output_dir_name,
        kind,
        profile: if rules.lossy {
            OptimizationProfile::Lossy
        } else {
            OptimizationProfile::Lossless
        },
    });
    Ok(())
}

async fn collect_apple_imageset(
    imageset_dir: &Path,
    rules: ImageRules,
    scanned: &mut ScannedAssets,
) -> eyre::Result<()> {
    if is_symlink(imageset_dir).await? {
        scanned.errors.push(ValidationIssue {
            path: imageset_dir.to_path_buf(),
            message: "Symlinks are not allowed for .imageset directories".to_string(),
            suggestion: Some("Replace symlink with a regular directory".to_string()),
        });
        return Ok(());
    }

    let set_name = file_name(imageset_dir)?
        .trim_end_matches(".imageset")
        .to_string();
    if set_name.is_empty() {
        scanned.errors.push(ValidationIssue {
            path: imageset_dir.to_path_buf(),
            message: "Invalid imageset name".to_string(),
            suggestion: Some("Use a non-empty directory name like icon.imageset".to_string()),
        });
        return Ok(());
    }
    if APPLE_RESERVED_ASSET_NAMES.contains(&set_name.as_str()) {
        scanned.errors.push(ValidationIssue {
            path: imageset_dir.to_path_buf(),
            message: format!("Imageset name '{set_name}' is reserved for Apple asset catalogs"),
            suggestion: Some(
                "Use another imageset name (reserved: AppIcon, AccentColor)".to_string(),
            ),
        });
        return Ok(());
    }

    let files = list_files_recursive(imageset_dir).await?;
    if files.is_empty() {
        scanned.errors.push(ValidationIssue {
            path: imageset_dir.to_path_buf(),
            message: "Empty imageset".to_string(),
            suggestion: Some("Add files like icon@1x.png, icon@2x.png, icon@3x.png".to_string()),
        });
        return Ok(());
    }

    let mut variants = Vec::new();
    let mut slot_guard = BTreeSet::new();
    let mut has_1x = false;
    for file in files {
        if file
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(is_placeholder_file_name)
        {
            continue;
        }
        let rel = file
            .strip_prefix(imageset_dir)
            .wrap_err_with(|| format!("Failed to relativize {}", file.display()))?;
        if rel
            .components()
            .next()
            .is_some_and(|c| matches!(c, Component::Normal(name) if name == OsStr::new("dark")))
            && rel.components().count() > 2
        {
            scanned.errors.push(ValidationIssue {
                path: file.clone(),
                message: "Nested paths under dark/ are not supported".to_string(),
                suggestion: Some("Use dark/<name>@2x.png".to_string()),
            });
            continue;
        }

        if !is_image_file(&file) {
            scanned.errors.push(ValidationIssue {
                path: file.clone(),
                message: "Non-image file inside imageset".to_string(),
                suggestion: Some("Keep only image files inside .imageset directories".to_string()),
            });
            continue;
        }

        let dark = rel
            .components()
            .next()
            .is_some_and(|c| matches!(c, Component::Normal(name) if name == OsStr::new("dark")));
        let file_name = file
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_eyre("Image file has no valid UTF-8 name")?;
        let (output_name, scale) = match parse_imageset_file_name(file_name) {
            Ok(parsed) => parsed,
            Err(error) => {
                scanned.errors.push(ValidationIssue {
                    path: file.clone(),
                    message: format!("Invalid imageset filename: {error}"),
                    suggestion: Some(
                        "Use <name>.png or <name>@1x|2x|3x.<ext> (for example logo@2x.png)"
                            .to_string(),
                    ),
                });
                continue;
            }
        };
        if scale == "1x" {
            has_1x = true;
        }

        let slot = format!("{}:{}", if dark { "dark" } else { "any" }, scale);
        if !slot_guard.insert(slot.clone()) {
            scanned.errors.push(ValidationIssue {
                path: file.clone(),
                message: format!("Duplicate imageset variant slot '{slot}'"),
                suggestion: Some("Keep a single file per appearance+scale slot".to_string()),
            });
            continue;
        }

        variants.push(AppleImageVariant {
            source: file,
            output_name,
            scale,
            dark,
            profile: if rules.lossy {
                OptimizationProfile::Lossy
            } else {
                OptimizationProfile::Lossless
            },
        });
    }

    if !has_1x {
        scanned.warnings.push(ValidationIssue {
            path: imageset_dir.to_path_buf(),
            message: "No 1x image variant found in imageset".to_string(),
            suggestion: Some("Add at least one @1x image for broad platform coverage".to_string()),
        });
    }

    if !variants.is_empty() {
        scanned.apple_imagesets.push(AppleImageSet {
            name: set_name,
            variants,
        });
    }
    Ok(())
}

async fn collect_android_drawables(
    android_root: &Path,
    rules: ImageRules,
    scanned: &mut ScannedAssets,
) -> eyre::Result<()> {
    if is_symlink(android_root).await? {
        scanned.errors.push(ValidationIssue {
            path: android_root.to_path_buf(),
            message: "Symlinks are not allowed for images/android".to_string(),
            suggestion: Some("Replace symlink with a regular directory".to_string()),
        });
        return Ok(());
    }

    let mut entries = fs::read_dir(android_root).await?;
    while let Some(entry) = entries.next().await {
        let entry = entry?;
        let qualifier_path = entry.path();
        let qualifier = file_name(&qualifier_path)?;
        if is_symlink(&qualifier_path).await? {
            scanned.errors.push(ValidationIssue {
                path: qualifier_path.clone(),
                message: "Symlinks are not allowed under images/android".to_string(),
                suggestion: Some("Replace symlink with a regular directory".to_string()),
            });
            continue;
        }

        if qualifier_path.is_file() {
            if is_placeholder_file_name(qualifier.as_str()) {
                continue;
            }
            scanned.errors.push(ValidationIssue {
                path: qualifier_path.clone(),
                message: "Files are not allowed directly under images/android/".to_string(),
                suggestion: Some("Use images/android/drawable-*/<name>.png".to_string()),
            });
            continue;
        }

        if !qualifier.starts_with("drawable") {
            scanned.errors.push(ValidationIssue {
                path: qualifier_path.clone(),
                message: format!("Invalid Android qualifier '{qualifier}'"),
                suggestion: Some(
                    "Use drawable-<qualifier> directories (e.g., drawable-xhdpi)".to_string(),
                ),
            });
            continue;
        }

        let files = list_files_recursive(&qualifier_path).await?;
        for file in files {
            if file
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(is_placeholder_file_name)
            {
                continue;
            }
            if file
                .strip_prefix(&qualifier_path)
                .ok()
                .is_some_and(|rel| rel.components().count() != 1)
            {
                scanned.errors.push(ValidationIssue {
                    path: file.clone(),
                    message: "Nested subdirectories are not allowed inside drawable-*".to_string(),
                    suggestion: Some("Flatten files directly under drawable-*".to_string()),
                });
                continue;
            }

            if !is_android_drawable_file(&file) {
                scanned.errors.push(ValidationIssue {
                    path: file.clone(),
                    message: "Unsupported file inside Android drawable directory".to_string(),
                    suggestion: Some(
                        "Keep only image files or XML vector drawables inside drawable-*"
                            .to_string(),
                    ),
                });
                continue;
            }

            let stem = file
                .file_stem()
                .and_then(OsStr::to_str)
                .ok_or_eyre("Drawable file has no UTF-8 stem")?;
            let ext = file
                .extension()
                .and_then(OsStr::to_str)
                .ok_or_eyre("Drawable file has no extension")?;
            let normalized_stem = normalize_android_resource_name(stem);
            let output_name = format!("{normalized_stem}.{}", ext.to_ascii_lowercase());

            scanned.android_drawables.push(AndroidDrawable {
                source: file,
                qualifier: qualifier.clone(),
                output_name,
                profile: if rules.lossy {
                    OptimizationProfile::Lossy
                } else {
                    OptimizationProfile::Lossless
                },
            });
        }
    }
    Ok(())
}

async fn collect_gtk_images(
    gtk_root: &Path,
    rules: ImageRules,
    scanned: &mut ScannedAssets,
) -> eyre::Result<()> {
    if is_symlink(gtk_root).await? {
        scanned.errors.push(ValidationIssue {
            path: gtk_root.to_path_buf(),
            message: "Symlinks are not allowed for images/gtk".to_string(),
            suggestion: Some("Replace symlink with a regular directory".to_string()),
        });
        return Ok(());
    }

    let files = list_files_recursive(gtk_root).await?;
    for file in files {
        if file
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(is_placeholder_file_name)
        {
            continue;
        }
        if !is_image_file(&file) {
            scanned.errors.push(ValidationIssue {
                path: file.clone(),
                message: "Non-image file inside images/gtk".to_string(),
                suggestion: Some("Keep only image files inside images/gtk".to_string()),
            });
            continue;
        }

        let relative = file
            .strip_prefix(gtk_root)
            .wrap_err_with(|| format!("Failed to relativize {}", file.display()))?
            .to_path_buf();
        scanned.gtk_images.push(GtkImage {
            source: file,
            relative,
            profile: if rules.lossy {
                OptimizationProfile::Lossy
            } else {
                OptimizationProfile::Lossless
            },
        });
    }
    Ok(())
}

fn is_placeholder_file_name(name: &str) -> bool {
    // Ignore editor/OS metadata files so asset scanning remains robust
    // across different environments and VCS conventions.
    name.starts_with('.')
        || name.eq_ignore_ascii_case("thumbs.db")
        || name.eq_ignore_ascii_case("desktop.ini")
}

fn parse_imageset_file_name(file_name: &str) -> eyre::Result<(String, String)> {
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(OsStr::to_str)
        .ok_or_eyre("Image filename has invalid UTF-8")?;
    let ext = path
        .extension()
        .and_then(OsStr::to_str)
        .ok_or_eyre("Image filename missing extension")?
        .to_ascii_lowercase();

    let (base, scale) = if let Some((base, suffix)) = stem.rsplit_once('@') {
        if suffix.ends_with('x') {
            let scale = suffix.to_string();
            (base.to_string(), scale)
        } else {
            (stem.to_string(), "1x".to_string())
        }
    } else {
        (stem.to_string(), "1x".to_string())
    };

    if base.is_empty() {
        eyre::bail!("Invalid imageset filename: {file_name}");
    }
    if !matches!(scale.as_str(), "1x" | "2x" | "3x") {
        eyre::bail!("unsupported scale '{scale}' in '{file_name}' (allowed: 1x, 2x, 3x)");
    }

    Ok((format!("{base}@{scale}.{ext}"), scale))
}

fn normalize_android_resource_name(stem: &str) -> String {
    let mut out = String::with_capacity(stem.len());
    for ch in stem.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out = out.trim_matches('_').to_string();
    if out.is_empty() {
        out = "asset".to_string();
    }
    if out
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_digit())
    {
        out.insert_str(0, "img_");
    }
    out
}

fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .map(|ext| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn is_android_drawable_file(path: &Path) -> bool {
    if is_image_file(path) {
        return true;
    }
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
}

fn file_name(path: &Path) -> eyre::Result<String> {
    Ok(path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_eyre(format!("Path has no UTF-8 filename: {}", path.display()))?
        .to_string())
}

async fn write_apple_placeholder_app_icon(xcassets_dest: &Path) -> eyre::Result<()> {
    #[derive(Serialize)]
    struct ImageItem<'a> {
        idiom: &'a str,
        size: &'a str,
        scale: &'a str,
        filename: &'a str,
    }

    #[derive(Serialize)]
    struct Info<'a> {
        version: u8,
        author: &'a str,
    }

    #[derive(Serialize)]
    struct Contents<'a> {
        images: Vec<ImageItem<'a>>,
        info: Info<'a>,
    }

    let appicon_dir = xcassets_dest.join("AppIcon.appiconset");
    fs::create_dir_all(&appicon_dir).await?;

    let icon_name = "AppIcon-1024.png";
    let image = image::RgbaImage::from_pixel(1024, 1024, image::Rgba([0, 0, 0, 0]));
    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        &mut png,
        image::codecs::png::CompressionType::Best,
        image::codecs::png::FilterType::Adaptive,
    );
    encoder.write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::Rgba8,
    )?;
    fs::write(appicon_dir.join(icon_name), png).await?;

    let json = serde_json::to_vec_pretty(&Contents {
        images: vec![
            ImageItem {
                idiom: "ios-marketing",
                size: "1024x1024",
                scale: "1x",
                filename: icon_name,
            },
            ImageItem {
                idiom: "mac",
                size: "512x512",
                scale: "2x",
                filename: icon_name,
            },
        ],
        info: Info {
            version: 1,
            author: "water",
        },
    })?;
    fs::write(appicon_dir.join("Contents.json"), json).await?;
    Ok(())
}

async fn write_apple_placeholder_accent_color(xcassets_dest: &Path) -> eyre::Result<()> {
    #[derive(Serialize)]
    struct Components<'a> {
        red: &'a str,
        green: &'a str,
        blue: &'a str,
        alpha: &'a str,
    }

    #[derive(Serialize)]
    struct Color<'a> {
        #[serde(rename = "color-space")]
        color_space: &'a str,
        components: Components<'a>,
    }

    #[derive(Serialize)]
    struct ColorItem<'a> {
        idiom: &'a str,
        color: Color<'a>,
    }

    #[derive(Serialize)]
    struct Info<'a> {
        version: u8,
        author: &'a str,
    }

    #[derive(Serialize)]
    struct Contents<'a> {
        colors: Vec<ColorItem<'a>>,
        info: Info<'a>,
    }

    let accent_dir = xcassets_dest.join("AccentColor.colorset");
    fs::create_dir_all(&accent_dir).await?;

    let json = serde_json::to_vec_pretty(&Contents {
        colors: vec![ColorItem {
            idiom: "universal",
            color: Color {
                color_space: "srgb",
                components: Components {
                    red: "0.000000",
                    green: "0.478431",
                    blue: "1.000000",
                    alpha: "1.000000",
                },
            },
        }],
        info: Info {
            version: 1,
            author: "water",
        },
    })?;
    fs::write(accent_dir.join("Contents.json"), json).await?;
    Ok(())
}

async fn write_apple_root_contents(xcassets_dest: &Path) -> eyre::Result<()> {
    #[derive(Serialize)]
    struct Info<'a> {
        version: u8,
        author: &'a str,
    }

    #[derive(Serialize)]
    struct Root<'a> {
        info: Info<'a>,
    }

    let json = serde_json::to_vec_pretty(&Root {
        info: Info {
            version: 1,
            author: "water",
        },
    })?;
    fs::write(xcassets_dest.join("Contents.json"), json).await?;
    Ok(())
}

async fn copy_apple_catalog_set(set: &AppleCatalogSet, dest_dir: &Path) -> eyre::Result<()> {
    if dest_dir.exists() {
        fs::remove_dir_all(dest_dir).await?;
    }
    fs::create_dir_all(dest_dir).await?;

    let files = list_files_recursive(&set.source_dir).await?;
    for source in files {
        let rel = source
            .strip_prefix(&set.source_dir)
            .wrap_err_with(|| format!("Failed to relativize {}", source.display()))?;
        let dest = dest_dir.join(rel);

        if let Some(file_name) = rel.file_name().and_then(OsStr::to_str) {
            if file_name == "Contents.json" {
                copy_file(&source, &dest).await?;
                continue;
            }
        }

        if is_image_file(&source) {
            let meta = format!("{}:{}", set.output_dir_name, rel.to_string_lossy());
            copy_with_optimization(&source, &dest, set.profile, "apple", &meta).await?;
            continue;
        }

        if set.kind == AppleCatalogSetKind::AppIconSet {
            warn!(
                "Unexpected non-image file in {}: {}",
                set.output_dir_name,
                source.display()
            );
        }
        copy_file(&source, &dest).await?;
    }

    Ok(())
}

async fn has_named_directory_under(root: &Path, expected_dir_name: &str) -> eyre::Result<bool> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .await
                .wrap_err_with(|| format!("Failed to read metadata for {}", path.display()))?;
            if metadata.file_type().is_symlink() {
                continue;
            }

            if metadata.is_dir() {
                if path
                    .file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|name| name == expected_dir_name)
                {
                    return Ok(true);
                }
                stack.push(path);
            }
        }
    }
    Ok(false)
}

async fn write_apple_imageset_contents(dest: &Path, set: &AppleImageSet) -> eyre::Result<()> {
    #[derive(Serialize)]
    struct Appearance<'a> {
        appearance: &'a str,
        value: &'a str,
    }

    #[derive(Serialize)]
    struct ImageItem<'a> {
        idiom: &'a str,
        filename: &'a str,
        scale: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        appearances: Option<Vec<Appearance<'a>>>,
    }

    #[derive(Serialize)]
    struct Info<'a> {
        version: u8,
        author: &'a str,
    }

    #[derive(Serialize)]
    struct Contents<'a> {
        images: Vec<ImageItem<'a>>,
        info: Info<'a>,
    }

    let images: Vec<ImageItem<'_>> = set
        .variants
        .iter()
        .map(|variant| ImageItem {
            idiom: "universal",
            filename: &variant.output_name,
            scale: &variant.scale,
            appearances: variant.dark.then(|| {
                vec![Appearance {
                    appearance: "luminosity",
                    value: "dark",
                }]
            }),
        })
        .collect();

    let json = serde_json::to_vec_pretty(&Contents {
        images,
        info: Info {
            version: 1,
            author: "water",
        },
    })?;
    fs::write(dest.join("Contents.json"), json).await?;
    Ok(())
}

async fn write_gresource_manifest(resources_dir: &Path, entries: &[String]) -> eyre::Result<()> {
    let mut unique = BTreeSet::new();
    unique.extend(entries.iter().cloned());

    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gresources>\n  <gresource prefix=\"/dev/waterui/assets\">\n",
    );
    for entry in unique {
        let escaped = xml_escape(&entry);
        let _ = writeln!(xml, "    <file compressed=\"true\">{escaped}</file>");
    }
    xml.push_str("  </gresource>\n</gresources>\n");

    fs::write(resources_dir.join("resources.gresource.xml"), xml).await?;
    Ok(())
}

async fn compile_gresource(resources_dir: &Path) -> eyre::Result<()> {
    which("glib-compile-resources")
        .await
        .wrap_err("`glib-compile-resources` is required for GTK resource bundling")?;

    let mut cmd = smol::process::Command::new("glib-compile-resources");
    let output = command(
        cmd.current_dir(resources_dir)
            .arg("resources.gresource.xml")
            .arg("--target=resources.gresource")
            .arg("--sourcedir=."),
    )
    .output()
    .await?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    eyre::bail!("Failed to compile GTK resources: {stderr}");
}

fn path_for_gresource(path: &Path, resources_dir: &Path) -> eyre::Result<String> {
    let rel = path
        .strip_prefix(resources_dir)
        .wrap_err_with(|| format!("Failed to relativize gresource path {}", path.display()))?;
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

async fn cleanup_android_generated_drawables(
    res_root: &Path,
    manifest_path: &Path,
) -> eyre::Result<()> {
    if !manifest_path.exists() {
        return Ok(());
    }

    let bytes = fs::read(manifest_path).await?;
    let manifest: AndroidGeneratedManifest = match serde_json::from_slice(&bytes) {
        Ok(manifest) => manifest,
        Err(error) => {
            warn!(
                "Invalid Android assets manifest at {}: {error}",
                manifest_path.display()
            );
            remove_file_if_exists(manifest_path.to_path_buf()).await?;
            return Ok(());
        }
    };

    for rel in manifest.files {
        let file_path = normalize_relative_under(res_root, &rel)?;
        if file_path.exists() && file_path.is_file() {
            fs::remove_file(&file_path).await?;
        }
    }

    cleanup_empty_dirs(res_root).await?;
    remove_file_if_exists(manifest_path.to_path_buf()).await?;
    Ok(())
}

async fn write_android_generated_manifest(
    manifest_path: &Path,
    files: &[String],
) -> eyre::Result<()> {
    if let Some(parent) = manifest_path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let mut unique = BTreeSet::new();
    unique.extend(files.iter().cloned());
    let manifest = AndroidGeneratedManifest {
        version: 1,
        files: unique.into_iter().collect(),
    };
    let payload = serde_json::to_vec_pretty(&manifest)?;
    fs::write(manifest_path, payload).await?;
    Ok(())
}

fn normalize_relative_under(root: &Path, rel: &str) -> eyre::Result<PathBuf> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        eyre::bail!("manifest path must be relative: {rel}");
    }
    if rel_path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        eyre::bail!("manifest path escapes res root: {rel}");
    }
    Ok(root.join(rel_path))
}

async fn copy_with_optimization(
    source: &Path,
    destination: &Path,
    profile: OptimizationProfile,
    target: &str,
    variant_meta: &str,
) -> eyre::Result<()> {
    let cache_path = optimized_cache_path(source, profile, target, variant_meta).await?;

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).await?;
    }
    copy_file(&cache_path, destination).await?;
    Ok(())
}

async fn optimized_cache_path(
    source: &Path,
    profile: OptimizationProfile,
    target: &str,
    variant_meta: &str,
) -> eyre::Result<PathBuf> {
    let bytes = fs::read(source).await?;
    let ext = source
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "bin".to_string());

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    hasher.update(target.as_bytes());
    hasher.update(profile.as_str().as_bytes());
    hasher.update(variant_meta.as_bytes());
    let digest = hex::encode(hasher.finalize());

    let cache_dir = dirs::home_dir()
        .ok_or_eyre("Could not determine home directory for assets cache")?
        .join(".water")
        .join("cache")
        .join("assets")
        .join("optimized");
    fs::create_dir_all(&cache_dir).await?;
    let cache_path = cache_dir.join(format!("{digest}.{ext}"));

    if let Ok(metadata) = fs::metadata(&cache_path).await {
        if metadata.len() > 0 {
            return Ok(cache_path);
        }
    }

    let optimized = optimize_image_bytes(&bytes, &ext, profile)?;
    let temp = cache_dir.join(format!("{digest}.{ext}.tmp-{}", std::process::id()));
    fs::write(&temp, optimized).await?;
    if let Err(e) = fs::rename(&temp, &cache_path).await {
        let _ = fs::remove_file(&temp).await;
        if !cache_path.exists() {
            return Err(e).wrap_err_with(|| {
                format!(
                    "Failed to finalize optimized cache file {}",
                    cache_path.display()
                )
            });
        }
    }

    Ok(cache_path)
}

fn optimize_image_bytes(
    source: &[u8],
    ext: &str,
    profile: OptimizationProfile,
) -> eyre::Result<Vec<u8>> {
    match (ext, profile) {
        ("png", OptimizationProfile::Lossless) | ("png", OptimizationProfile::Lossy) => {
            let image = image::load_from_memory(source)
                .map_err(|e| eyre::eyre!("Failed to decode PNG for optimization: {e}"))?
                .to_rgba8();
            let mut out = Vec::new();
            let encoder = image::codecs::png::PngEncoder::new_with_quality(
                &mut out,
                image::codecs::png::CompressionType::Best,
                image::codecs::png::FilterType::Adaptive,
            );
            encoder.write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgba8,
            )?;
            Ok(out)
        }
        ("jpg" | "jpeg", OptimizationProfile::Lossy) => {
            let image = image::load_from_memory(source)
                .map_err(|e| eyre::eyre!("Failed to decode JPEG for optimization: {e}"))?
                .to_rgb8();
            let mut out = Vec::new();
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85);
            encoder.encode(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgb8,
            )?;
            Ok(out)
        }
        _ => Ok(source.to_vec()),
    }
}

async fn list_files_recursive(root: &Path) -> eyre::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .await
                .wrap_err_with(|| format!("Failed to read metadata for {}", path.display()))?;
            if metadata.file_type().is_symlink() {
                warn!("Skipping symlink in asset pipeline: {}", path.display());
                continue;
            }
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

async fn cleanup_empty_dirs(root: &Path) -> eyre::Result<()> {
    let mut dirs = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        dirs.push(dir.clone());
        let mut entries = fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            let path = entry.path();
            if is_symlink(&path).await? {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            }
        }
    }

    // Remove deepest first.
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for dir in dirs {
        if dir == root {
            continue;
        }
        let mut entries = fs::read_dir(&dir).await?;
        if entries.next().await.is_none() {
            let _ = fs::remove_dir(&dir).await;
        }
    }
    Ok(())
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
        .replace('\'', "&apos;")
}

async fn is_symlink(path: &Path) -> eyre::Result<bool> {
    Ok(fs::symlink_metadata(path)
        .await
        .wrap_err_with(|| format!("Failed to read metadata for {}", path.display()))?
        .file_type()
        .is_symlink())
}

async fn remove_file_if_exists(path: PathBuf) -> eyre::Result<()> {
    if fs::metadata(&path).await.is_ok() {
        fs::remove_file(path).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn normalizes_android_resource_names() {
        assert_eq!(normalize_android_resource_name("Logo@2x"), "logo_2x");
        assert_eq!(normalize_android_resource_name("123abc"), "img_123abc");
        assert_eq!(normalize_android_resource_name("----"), "asset");
    }

    #[test]
    fn parses_imageset_file_names() {
        let (name, scale) = parse_imageset_file_name("logo@2x.png").expect("parse");
        assert_eq!(name, "logo@2x.png");
        assert_eq!(scale, "2x");

        let (name, scale) = parse_imageset_file_name("logo.png").expect("parse");
        assert_eq!(name, "logo@1x.png");
        assert_eq!(scale, "1x");
    }

    #[test]
    fn rejects_invalid_imageset_scale() {
        let error = parse_imageset_file_name("logo@4x.png").expect_err("4x should not be accepted");
        assert!(error.to_string().contains("allowed: 1x, 2x, 3x"));
    }

    #[test]
    fn writes_apple_placeholder_assets() {
        let tmp = tempdir().expect("tempdir");
        smol::block_on(async {
            write_apple_placeholder_app_icon(tmp.path())
                .await
                .expect("write app icon");
            write_apple_placeholder_accent_color(tmp.path())
                .await
                .expect("write accent");
        });

        assert!(
            tmp.path()
                .join("AppIcon.appiconset/AppIcon-1024.png")
                .exists()
        );
        assert!(tmp.path().join("AppIcon.appiconset/Contents.json").exists());
        assert!(
            tmp.path()
                .join("AccentColor.colorset/Contents.json")
                .exists()
        );
    }

    #[test]
    fn escapes_xml_entries() {
        let escaped = xml_escape("a&b<'\">");
        assert_eq!(escaped, "a&amp;b&lt;&apos;&quot;&gt;");
    }

    #[cfg(unix)]
    #[test]
    fn doctor_rejects_symlink_in_assets_root() {
        use std::os::unix::fs::symlink;

        let tmp = tempdir().expect("tempdir");
        let assets_root = tmp.path().join("assets");
        fs::create_dir_all(&assets_root).expect("create assets root");

        let real_dir = tmp.path().join("real-assets");
        fs::create_dir_all(&real_dir).expect("create real dir");
        let link_path = assets_root.join("images");
        symlink(&real_dir, &link_path).expect("create symlink");

        let report = smol::block_on(scan_assets(&assets_root)).expect("scan");
        assert_eq!(report.errors.len(), 1);
        assert!(
            report.errors[0]
                .message
                .contains("Symlinks are not allowed")
        );
    }

    #[test]
    fn detects_existing_named_directory_under_tree() {
        let tmp = tempdir().expect("tempdir");
        fs::create_dir_all(
            tmp.path()
                .join("foo.xcassets")
                .join("nested")
                .join("AppIcon.appiconset"),
        )
        .expect("create nested appicon");

        let found = smol::block_on(has_named_directory_under(tmp.path(), "AppIcon.appiconset"))
            .expect("scan");
        assert!(found);
    }

    #[test]
    fn android_drawable_accepts_xml_files() {
        let tmp = tempdir().expect("tempdir");
        let xml = tmp.path().join("drawable/vector_icon.xml");
        fs::create_dir_all(xml.parent().expect("parent")).expect("mkdir");
        fs::write(&xml, "<vector/>").expect("write");
        assert!(is_android_drawable_file(&xml));
    }

    #[test]
    fn image_extensions_include_svg_and_pdf() {
        let tmp = tempdir().expect("tempdir");
        let svg = tmp.path().join("icon.svg");
        let pdf = tmp.path().join("icon.pdf");
        fs::write(&svg, "<svg/>").expect("write svg");
        fs::write(&pdf, "%PDF").expect("write pdf");
        assert!(is_image_file(&svg));
        assert!(is_image_file(&pdf));
    }

    #[test]
    fn doctor_accepts_apple_catalog_sets() {
        let tmp = tempdir().expect("tempdir");
        let assets_root = tmp.path().join("assets/images");
        let appicon = assets_root.join("AppIcon.appiconset");
        let accent = assets_root.join("AccentColor.colorset");
        fs::create_dir_all(&appicon).expect("mkdir appicon");
        fs::create_dir_all(&accent).expect("mkdir accent");
        fs::write(appicon.join("Contents.json"), "{}").expect("write appicon contents");
        fs::write(accent.join("Contents.json"), "{}").expect("write accent contents");

        let report = smol::block_on(scan_assets(&tmp.path().join("assets"))).expect("scan");
        assert!(
            report.errors.is_empty(),
            "expected no errors, got: {:#?}",
            report.errors
        );
    }

    #[test]
    fn doctor_rejects_invalid_apple_catalog_set_name() {
        let tmp = tempdir().expect("tempdir");
        let assets_root = tmp.path().join("assets/images/Foo.appiconset");
        fs::create_dir_all(&assets_root).expect("mkdir");
        fs::write(assets_root.join("Contents.json"), "{}").expect("write contents");

        let report = smol::block_on(scan_assets(&tmp.path().join("assets"))).expect("scan");
        assert_eq!(report.errors.len(), 1);
        assert!(
            report.errors[0]
                .message
                .contains("Unsupported Apple catalog set name")
        );
    }

    #[test]
    fn doctor_rejects_apple_catalog_set_without_contents_json() {
        let tmp = tempdir().expect("tempdir");
        let assets_root = tmp.path().join("assets/images/AppIcon.appiconset");
        fs::create_dir_all(&assets_root).expect("mkdir");
        fs::write(assets_root.join("icon.png"), b"fake").expect("write icon");

        let report = smol::block_on(scan_assets(&tmp.path().join("assets"))).expect("scan");
        assert_eq!(report.errors.len(), 1);
        assert!(
            report.errors[0]
                .message
                .contains("must include Contents.json")
        );
    }

    #[test]
    fn doctor_ignores_gitkeep_under_images_root() {
        let tmp = tempdir().expect("tempdir");
        let assets_root = tmp.path().join("assets/images");
        fs::create_dir_all(&assets_root).expect("mkdir");
        fs::write(assets_root.join(".gitkeep"), b"").expect("write gitkeep");

        let report = smol::block_on(scan_assets(&tmp.path().join("assets"))).expect("scan");
        assert!(
            report.errors.is_empty(),
            "expected no errors, got: {:#?}",
            report.errors
        );
    }

    #[test]
    fn doctor_ignores_os_metadata_under_images_root() {
        let tmp = tempdir().expect("tempdir");
        let assets_root = tmp.path().join("assets/images");
        fs::create_dir_all(&assets_root).expect("mkdir");
        fs::write(assets_root.join(".DS_Store"), b"").expect("write ds store");
        fs::write(assets_root.join("Thumbs.db"), b"").expect("write thumbs");
        fs::write(assets_root.join("desktop.ini"), b"").expect("write desktop ini");

        let report = smol::block_on(scan_assets(&tmp.path().join("assets"))).expect("scan");
        assert!(
            report.errors.is_empty(),
            "expected no errors, got: {:#?}",
            report.errors
        );
    }

    #[test]
    fn normalize_relative_under_rejects_escape() {
        let tmp = tempdir().expect("tempdir");
        let err = normalize_relative_under(tmp.path(), "../evil.png").expect_err("must reject");
        assert!(err.to_string().contains("escapes res root"));
    }

    #[test]
    fn cleanup_android_generated_drawables_only_removes_manifest_files() {
        let tmp = tempdir().expect("tempdir");
        let res_root = tmp.path().join("res");
        let gen_dir = res_root.join("drawable-xhdpi");
        let keep_dir = res_root.join("drawable");
        fs::create_dir_all(&gen_dir).expect("create generated dir");
        fs::create_dir_all(&keep_dir).expect("create keep dir");

        let generated = gen_dir.join("hero.png");
        let keep = keep_dir.join("ic_launcher_foreground.xml");
        fs::write(&generated, b"generated").expect("write generated");
        fs::write(&keep, b"<vector/>").expect("write keep");

        let manifest = res_root.join(".water-assets-manifest.json");
        let payload = serde_json::to_vec_pretty(&AndroidGeneratedManifest {
            version: 1,
            files: vec!["drawable-xhdpi/hero.png".to_string()],
        })
        .expect("serialize manifest");
        fs::write(&manifest, payload).expect("write manifest");

        smol::block_on(cleanup_android_generated_drawables(&res_root, &manifest))
            .expect("cleanup generated drawables");

        assert!(!generated.exists(), "generated file should be removed");
        assert!(keep.exists(), "non-generated file must be preserved");
        assert!(
            !manifest.exists(),
            "manifest should be removed after cleanup"
        );
    }

    #[test]
    fn write_android_generated_manifest_deduplicates_files() {
        let tmp = tempdir().expect("tempdir");
        let manifest = tmp.path().join(".water-assets-manifest.json");

        smol::block_on(write_android_generated_manifest(
            &manifest,
            &[
                "drawable-xhdpi/logo.png".to_string(),
                "drawable-xhdpi/logo.png".to_string(),
            ],
        ))
        .expect("write manifest");

        let bytes = fs::read(&manifest).expect("read manifest");
        let parsed: AndroidGeneratedManifest =
            serde_json::from_slice(&bytes).expect("parse manifest");
        assert_eq!(parsed.files.len(), 1);
        assert_eq!(parsed.files[0], "drawable-xhdpi/logo.png");
    }
}
