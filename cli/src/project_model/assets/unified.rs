use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use askama::Template;
use eyre::{Context, bail};
use image::ImageEncoder;
use serde::Serialize;
use sha2::{Digest, Sha256};
use smol::fs;
use waterui_assets_core::AssetKind;
use waterui_assets_planner::{
    AssetRole, BUNDLE_META_PREFIX, BundleManifest, BundleMount, BundleMountMeta, ColorScheme,
    HexColor, LaunchPlan, PlannedAsset, ThemeConfig, plan_mount,
};

#[cfg(target_os = "macos")]
use super::icon::encode_macos_icns;
use super::icon::{
    IconSource, LINUX_HICOLOR_SIZES, WINDOW_ICON_SIZE, encode_png, render_android_foreground,
    render_apple_icon,
};
use crate::artifact_symbols::{ArtifactSymbols, build_host_rlib};
use crate::project::Project;

const ASSET_ROOT_DIR: &str = "waterui_assets";
/// The accent every platform falls back to when `[theme]` names none.
const DEFAULT_ACCENT: HexColor = HexColor::from_rgb([0x0A, 0x84, 0xFF]);
/// Point size of the iOS launch image, rendered at 1x, 2x and 3x.
const APPLE_LAUNCH_IMAGE_POINTS: u32 = 128;
/// Asset-catalog names the generated Xcode project refers to.
const APPLE_LAUNCH_BACKGROUND_SET: &str = "LaunchBackground";
const APPLE_LAUNCH_IMAGE_SET: &str = "LaunchImage";
const ANDROID_VALUES_DIR: &str = "app/src/main/res/values";
const ANDROID_VALUES_NIGHT_DIR: &str = "app/src/main/res/values-night";
const ANDROID_DRAWABLE_DIR: &str = "app/src/main/res/drawable";
const ANDROID_MIPMAP_DIRS: &[(&str, u32)] = &[
    ("mipmap-mdpi", 48),
    ("mipmap-hdpi", 72),
    ("mipmap-xhdpi", 96),
    ("mipmap-xxhdpi", 144),
    ("mipmap-xxxhdpi", 192),
];

pub async fn stage_for_apple(
    project: &Project,
    dest_dir: &Path,
    sccache_path: Option<&Path>,
) -> eyre::Result<BundleManifest> {
    let manifest = build_manifest(project, sccache_path).await?;
    let assets_dest = dest_dir.join(ASSET_ROOT_DIR);
    reset_dir(&assets_dest).await?;
    copy_manifest_assets(&manifest, &assets_dest).await?;
    write_manifest_stamp(&manifest, &assets_dest).await?;

    let xcassets_dest = dest_dir.join("WaterUIAssets.xcassets");
    reset_dir(&xcassets_dest).await?;
    write_apple_root_contents(&xcassets_dest).await?;
    let accent = project
        .manifest()
        .theme
        .as_ref()
        .and_then(|theme| theme.accent);

    let icon = load_project_icon(&manifest)?;
    write_apple_app_icon(&icon, &xcassets_dest).await?;
    write_apple_color_set(
        "AccentColor",
        accent.unwrap_or(DEFAULT_ACCENT),
        None,
        &xcassets_dest,
    )
    .await?;

    // The launch screen: a color set with a dark appearance when one differs,
    // and the artwork as a universal image set. Neither exists when nothing
    // is configured, and the generated project then names neither.
    let launch = launch_assets_from(project, &manifest)?;
    let plan = launch.plan();
    if let Some(background) = plan.background(ColorScheme::Light) {
        let dark = plan
            .has_distinct_dark_background()
            .then(|| plan.background(ColorScheme::Dark).copied())
            .flatten();
        write_apple_color_set(
            APPLE_LAUNCH_BACKGROUND_SET,
            *background,
            dark,
            &xcassets_dest,
        )
        .await?;
    }
    if let Some(artwork) = launch.artwork() {
        write_apple_launch_image(artwork, &xcassets_dest).await?;
    }

    Ok(manifest)
}

/// The project's launch screen, resolved, with the artwork it shows.
pub struct LaunchAssets {
    plan: LaunchPlan,
    artwork: Option<IconSource>,
    app_icon: IconSource,
}

impl LaunchAssets {
    /// The resolved `[launch]` colors and artwork path.
    #[must_use]
    pub const fn plan(&self) -> &LaunchPlan {
        &self.plan
    }

    /// Whether a `Launch.*` artwork exists.
    #[must_use]
    pub const fn has_artwork(&self) -> bool {
        self.artwork.is_some()
    }

    /// The `Launch.*` artwork, for platforms whose default is no artwork
    /// (iOS) or the OS's own (Android).
    #[must_use]
    pub(super) const fn artwork(&self) -> Option<&IconSource> {
        self.artwork.as_ref()
    }

    /// The `Launch.*` artwork, or the app icon where that is the platform's
    /// default (the web).
    #[must_use]
    pub(super) const fn artwork_or_app_icon(&self) -> &IconSource {
        match &self.artwork {
            Some(artwork) => artwork,
            None => &self.app_icon,
        }
    }

    /// The `Launch.*` artwork or the app icon, rendered `size` pixels square
    /// as PNG bytes.
    ///
    /// # Errors
    ///
    /// Fails when the artwork cannot be rendered or encoded.
    pub fn artwork_or_app_icon_png(&self, size: u32) -> eyre::Result<Vec<u8>> {
        encode_png(&self.artwork_or_app_icon().render(size)?)
    }
}

/// Resolves the project's launch screen: `[launch]` over `[theme]`, and the
/// root `Launch.*` artwork.
///
/// # Errors
///
/// Fails when the assets cannot be planned or an artwork file cannot be
/// decoded.
pub fn launch_assets(project: &Project) -> eyre::Result<LaunchAssets> {
    let manifest = build_main_manifest(project)?;
    launch_assets_from(project, &manifest)
}

fn launch_assets_from(project: &Project, manifest: &BundleManifest) -> eyre::Result<LaunchAssets> {
    let water = project.manifest();
    let plan = LaunchPlan::resolve(water.launch.as_ref(), water.theme.as_ref(), manifest);
    let artwork = plan
        .image()
        .map(|path| IconSource::load(path))
        .transpose()?;
    let app_icon = load_project_icon(manifest)?;
    Ok(LaunchAssets {
        plan,
        artwork,
        app_icon,
    })
}

/// Loads the project's `Icon.*` asset, or the bundled `WaterUI` logo when the
/// project does not provide one.
fn load_project_icon(manifest: &BundleManifest) -> eyre::Result<IconSource> {
    manifest.root_artwork(AssetRole::AppIcon).map_or_else(
        || Ok(IconSource::default_logo()),
        |icon| IconSource::load(&icon.source_path),
    )
}

pub async fn stage_for_android(
    project: &Project,
    backend_path: &Path,
    sccache_path: Option<&Path>,
) -> eyre::Result<BundleManifest> {
    let manifest = build_manifest(project, sccache_path).await?;
    let assets_dest = backend_path
        .join("app/src/main/assets")
        .join(ASSET_ROOT_DIR);
    reset_dir(&assets_dest).await?;
    copy_manifest_assets(&manifest, &assets_dest).await?;
    write_manifest_stamp(&manifest, &assets_dest).await?;

    let res_root = backend_path.join("app/src/main/res");
    fs::create_dir_all(&res_root).await?;

    let icon = load_project_icon(&manifest)?;
    let icon_background = icon.edge_color()?;
    let launch = launch_assets_from(project, &manifest)?;

    let theme = project.manifest().theme.as_ref();
    write_android_theme_files(theme, icon_background, &launch, backend_path).await?;

    // Older CLI versions staged the foreground as a vector drawable; a PNG
    // and an XML with the same resource name cannot coexist.
    remove_file_if_exists(res_root.join("drawable/ic_launcher_foreground.xml")).await?;
    write_android_icon_resources(&icon, icon_background, backend_path).await?;
    write_android_launch_artwork(&launch, backend_path).await?;

    Ok(manifest)
}

/// Renders the project's macOS `.icns` app icon for hand-assembled bundles
/// (self-drawn backends that do not go through an Xcode asset catalog).
///
/// Gated to macOS because the only caller — the macOS packaging path in
/// `hydrolysis::platform` — is gated the same way.
///
/// # Errors
///
/// Fails when the icon asset cannot be loaded or rendered.
#[cfg(target_os = "macos")]
pub fn macos_icns(project: &Project) -> eyre::Result<Vec<u8>> {
    // Only the main mount can claim the AppIcon role, so enumerating
    // `include_bundle!` mounts is unnecessary here.
    let manifest = build_main_manifest(project)?;
    let icon = load_project_icon(&manifest)?;
    encode_macos_icns(&icon)
}

/// Renders the project's Windows `.ico` app icon for embedding into the
/// packaged executable.
///
/// # Errors
///
/// Fails when the icon asset cannot be loaded or rendered.
pub fn windows_ico(project: &Project) -> eyre::Result<Vec<u8>> {
    let manifest = build_main_manifest(project)?;
    let icon = load_project_icon(&manifest)?;
    super::icon::encode_windows_ico(&icon)
}

pub async fn stage_for_gtk(
    project: &Project,
    resources_dir: &Path,
    sccache_path: Option<&Path>,
) -> eyre::Result<BundleManifest> {
    let manifest = build_manifest(project, sccache_path).await?;
    let assets_dest = resources_dir.join(ASSET_ROOT_DIR);
    reset_dir(&assets_dest).await?;
    copy_manifest_assets(&manifest, &assets_dest).await?;
    write_manifest_stamp(&manifest, &assets_dest).await?;

    // Self-drawn desktop backends read this at startup to set the runtime
    // window icon (taskbars on X11 and Windows show it; macOS uses the
    // bundle's icns instead).
    let icon = load_project_icon(&manifest)?;
    write_png(
        &icon.render(WINDOW_ICON_SIZE)?,
        &assets_dest.join(waterui_assets_core::WINDOW_ICON_FILE),
    )
    .await?;

    remove_file_if_exists(resources_dir.join("resources.gresource")).await?;
    remove_file_if_exists(resources_dir.join("resources.gresource.xml")).await?;
    Ok(manifest)
}

/// Installs the app icon into a freedesktop hicolor icon-theme tree rooted at
/// `icons_root`, named after the bundle identifier so desktop entries and
/// GTK icon-name lookup resolve it.
pub async fn stage_hicolor_icons(project: &Project, icons_root: &Path) -> eyre::Result<()> {
    let manifest = build_main_manifest(project)?;
    let icon = load_project_icon(&manifest)?;
    let name = format!("{}.png", project.bundle_identifier());
    for &size in LINUX_HICOLOR_SIZES {
        let dir = icons_root.join(format!("hicolor/{size}x{size}/apps"));
        write_png(&icon.render(size)?, &dir.join(&name)).await?;
    }
    Ok(())
}

/// Resolves the fonts inside an already-staged manifest; the staging call
/// that produced it owns the artifact enumeration, so this stays pure.
pub fn scan_project_fonts(manifest: &BundleManifest) -> eyre::Result<Vec<super::ResolvedFont>> {
    manifest
        .assets
        .iter()
        .filter(|asset| asset.kind == AssetKind::Font)
        .map(|asset| {
            let name = detect_font_family(&asset.source_path)?;
            Ok(super::ResolvedFont {
                name,
                path: asset.source_path.clone(),
            })
        })
        .collect()
}

/// Plans only the main application asset root.
///
/// Enough for the icon paths: `include_bundle!` mounts are sibling namespaces
/// that can never claim the `AppIcon` role.
fn build_main_manifest(project: &Project) -> eyre::Result<BundleManifest> {
    Ok(BundleManifest {
        crate_root: project.root().to_path_buf(),
        assets_root: project.assets_dir(),
        mounts: Vec::new(),
        assets: plan_main_assets(project)?,
    })
}

fn plan_main_assets(project: &Project) -> eyre::Result<Vec<PlannedAsset>> {
    let assets_dir = project.assets_dir();
    if assets_dir.is_dir() {
        Ok(plan_mount(&assets_dir, "")?)
    } else {
        Ok(Vec::new())
    }
}

/// Plans the full manifest: the main asset root plus every `include_bundle!`
/// mount enumerated from the compiled library's `waterui_meta_bundle_*`
/// statics.
async fn build_manifest(
    project: &Project,
    sccache_path: Option<&Path>,
) -> eyre::Result<BundleManifest> {
    let mut assets = plan_main_assets(project)?;

    let rlib = build_host_rlib(project.root(), sccache_path).await?;
    let symbols = ArtifactSymbols::read(&rlib)?;
    // The declared frontend toolchain is a manifest concern: `[web]` absent
    // means bun, and the declared manager is never substituted.
    let package_manager = project
        .manifest()
        .web
        .as_ref()
        .map_or_else(crate::web::PackageManager::default, |web| {
            web.package_manager
        });
    // The main root is always planned; a second `assets` declaration is a
    // duplicate mount.
    let mut seen = BTreeSet::new();
    let mut mounts = Vec::new();
    for leaf in symbols.leaves_with_prefix(BUNDLE_META_PREFIX) {
        let meta = BundleMountMeta::from_payload(&symbols.static_bytes(&leaf)?)?;
        // The `assets` mount is the main root, already planned above.
        if meta.mount == "assets" {
            continue;
        }
        if !seen.insert(meta.mount.clone()) {
            bail!(
                "include_bundle mount '{}' is declared more than once",
                meta.mount
            );
        }
        // An `include_web!` mount carries the toolchain project that produces
        // its output directory; build it before staging. Staging runs once
        // per `water` invocation — callers reuse the returned manifest — so
        // the frontend build rides that same exactly-once guarantee.
        if meta.project.is_some() {
            crate::web::build_frontend(package_manager, &meta).await?;
        }
        assets.extend(plan_mount(&meta.path, &meta.mount)?);
        mounts.push(BundleMount {
            name: meta.mount,
            root: meta.path,
        });
    }

    assets.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    Ok(BundleManifest {
        crate_root: project.root().to_path_buf(),
        assets_root: project.assets_dir(),
        mounts,
        assets,
    })
}

async fn copy_manifest_assets(manifest: &BundleManifest, dest_root: &Path) -> eyre::Result<()> {
    for asset in &manifest.assets {
        let dest = dest_root.join(&asset.logical_path);
        copy_asset(asset, &dest).await?;
    }
    Ok(())
}

async fn copy_asset(asset: &PlannedAsset, dest: &Path) -> eyre::Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).await?;
    }
    match asset.kind {
        AssetKind::Image => {
            let bytes = fs::read(&asset.source_path).await?;
            let optimized = optimize_image(&bytes, &asset.source_path)?;
            fs::write(dest, optimized).await?;
        }
        _ => {
            fs::copy(&asset.source_path, dest).await?;
        }
    }
    Ok(())
}

fn optimize_image(bytes: &[u8], source: &Path) -> eyre::Result<Vec<u8>> {
    let ext = source
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "png" => {
            let image = image::load_from_memory(bytes)
                .map_err(|error| {
                    eyre::eyre!("Failed to decode PNG '{}': {error}", source.display())
                })?
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
        "jpg" | "jpeg" => {
            let image = image::load_from_memory(bytes)
                .map_err(|error| {
                    eyre::eyre!("Failed to decode JPEG '{}': {error}", source.display())
                })?
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
        _ => Ok(bytes.to_vec()),
    }
}

async fn write_manifest_stamp(manifest: &BundleManifest, dest_root: &Path) -> eyre::Result<()> {
    let mut hasher = Sha256::new();
    for asset in &manifest.assets {
        hasher.update(asset.logical_path.to_string_lossy().as_bytes());
        let bytes = std::fs::read(&asset.source_path).wrap_err_with(|| {
            format!(
                "Failed to read '{}' for asset stamp",
                asset.source_path.display()
            )
        })?;
        hasher.update(&bytes);
    }
    let stamp = hex::encode(hasher.finalize());
    fs::write(dest_root.join(".waterui-sync-stamp"), stamp).await?;
    Ok(())
}

async fn reset_dir(path: &Path) -> eyre::Result<()> {
    if path.exists() {
        fs::remove_dir_all(path).await?;
    }
    fs::create_dir_all(path).await?;
    Ok(())
}

async fn remove_file_if_exists(path: PathBuf) -> eyre::Result<()> {
    match fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn detect_font_family(path: &Path) -> eyre::Result<String> {
    let bytes = std::fs::read(path)
        .wrap_err_with(|| format!("Failed to read font '{}'", path.display()))?;
    let face = ttf_parser::Face::parse(&bytes, 0)
        .map_err(|_| eyre::eyre!("Failed to parse font family from '{}'", path.display()))?;
    let family = face
        .names()
        .into_iter()
        .find(|name| name.name_id == ttf_parser::name_id::FULL_NAME)
        .or_else(|| {
            face.names()
                .into_iter()
                .find(|name| name.name_id == ttf_parser::name_id::TYPOGRAPHIC_FAMILY)
        })
        .or_else(|| {
            face.names()
                .into_iter()
                .find(|name| name.name_id == ttf_parser::name_id::FAMILY)
        })
        .and_then(|name| name.to_string())
        .ok_or_else(|| {
            eyre::eyre!(
                "Font '{}' does not contain a readable family name",
                path.display()
            )
        })?;
    Ok(family)
}

/// Writes a named color set with a universal color and, when given, a dark
/// appearance.
async fn write_apple_color_set(
    name: &str,
    light: HexColor,
    dark: Option<HexColor>,
    xcassets_dest: &Path,
) -> eyre::Result<()> {
    #[derive(Serialize)]
    struct Components {
        red: String,
        green: String,
        blue: String,
        alpha: &'static str,
    }

    #[derive(Serialize)]
    struct Color {
        #[serde(rename = "color-space")]
        color_space: &'static str,
        components: Components,
    }

    #[derive(Serialize)]
    struct Appearance {
        appearance: &'static str,
        value: &'static str,
    }

    #[derive(Serialize)]
    struct ColorItem {
        #[serde(skip_serializing_if = "Vec::is_empty")]
        appearances: Vec<Appearance>,
        idiom: &'static str,
        color: Color,
    }

    #[derive(Serialize)]
    struct Info {
        version: u8,
        author: &'static str,
    }

    #[derive(Serialize)]
    struct Contents {
        colors: Vec<ColorItem>,
        info: Info,
    }

    fn item(color: HexColor, appearances: Vec<Appearance>) -> ColorItem {
        let [red, green, blue] = color.rgb();
        ColorItem {
            appearances,
            idiom: "universal",
            color: Color {
                color_space: "srgb",
                components: Components {
                    red: component_string(red),
                    green: component_string(green),
                    blue: component_string(blue),
                    alpha: "1.000000",
                },
            },
        }
    }

    let mut colors = vec![item(light, Vec::new())];
    if let Some(dark) = dark {
        colors.push(item(
            dark,
            vec![Appearance {
                appearance: "luminosity",
                value: "dark",
            }],
        ));
    }
    let set_dir = xcassets_dest.join(format!("{name}.colorset"));
    fs::create_dir_all(&set_dir).await?;
    let json = serde_json::to_vec_pretty(&Contents {
        colors,
        info: Info {
            version: 1,
            author: "water",
        },
    })?;
    fs::write(set_dir.join("Contents.json"), json).await?;
    Ok(())
}

/// Writes the launch artwork as a universal image set at 1x, 2x and 3x of
/// its point size; iOS centers it at that size inside the safe area.
async fn write_apple_launch_image(source: &IconSource, xcassets_dest: &Path) -> eyre::Result<()> {
    #[derive(Serialize)]
    struct ImageItem {
        idiom: &'static str,
        scale: &'static str,
        filename: String,
    }

    #[derive(Serialize)]
    struct Info {
        version: u8,
        author: &'static str,
    }

    #[derive(Serialize)]
    struct Contents {
        images: Vec<ImageItem>,
        info: Info,
    }

    let set_dir = xcassets_dest.join(format!("{APPLE_LAUNCH_IMAGE_SET}.imageset"));
    reset_dir(&set_dir).await?;
    let mut images = Vec::new();
    for (scale, factor) in [("1x", 1_u32), ("2x", 2), ("3x", 3)] {
        let filename = format!("{APPLE_LAUNCH_IMAGE_SET}@{scale}.png");
        write_png(
            &source.render(APPLE_LAUNCH_IMAGE_POINTS * factor)?,
            &set_dir.join(&filename),
        )
        .await?;
        images.push(ImageItem {
            idiom: "universal",
            scale,
            filename,
        });
    }
    let json = serde_json::to_vec_pretty(&Contents {
        images,
        info: Info {
            version: 1,
            author: "water",
        },
    })?;
    fs::write(set_dir.join("Contents.json"), json).await?;
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

async fn write_apple_app_icon(source: &IconSource, xcassets_dest: &Path) -> eyre::Result<()> {
    #[derive(Serialize)]
    struct ImageItem<'a> {
        idiom: &'a str,
        size: &'a str,
        scale: &'a str,
        filename: String,
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
    reset_dir(&appicon_dir).await?;

    let specs = [
        ("iphone", "20x20", "2x", 40_u32),
        ("iphone", "20x20", "3x", 60),
        ("iphone", "29x29", "2x", 58),
        ("iphone", "29x29", "3x", 87),
        ("iphone", "40x40", "2x", 80),
        ("iphone", "40x40", "3x", 120),
        ("iphone", "60x60", "2x", 120),
        ("iphone", "60x60", "3x", 180),
        ("ipad", "20x20", "1x", 20),
        ("ipad", "20x20", "2x", 40),
        ("ipad", "29x29", "1x", 29),
        ("ipad", "29x29", "2x", 58),
        ("ipad", "40x40", "1x", 40),
        ("ipad", "40x40", "2x", 80),
        ("ipad", "76x76", "1x", 76),
        ("ipad", "76x76", "2x", 152),
        ("ipad", "83.5x83.5", "2x", 167),
        ("mac", "16x16", "1x", 16),
        ("mac", "16x16", "2x", 32),
        ("mac", "32x32", "1x", 32),
        ("mac", "32x32", "2x", 64),
        ("mac", "128x128", "1x", 128),
        ("mac", "128x128", "2x", 256),
        ("mac", "256x256", "1x", 256),
        ("mac", "256x256", "2x", 512),
        ("mac", "512x512", "1x", 512),
        ("mac", "512x512", "2x", 1024),
        ("ios-marketing", "1024x1024", "1x", 1024),
    ];

    let mut images = Vec::new();
    for (idiom, size, scale, pixels) in specs {
        let file_name = format!("AppIcon-{idiom}-{size}@{scale}.png");
        write_png(
            &render_apple_icon(source, idiom, pixels)?,
            &appicon_dir.join(&file_name),
        )
        .await?;
        images.push(ImageItem {
            idiom,
            size,
            scale,
            filename: file_name,
        });
    }

    let json = serde_json::to_vec_pretty(&Contents {
        images,
        info: Info {
            version: 1,
            author: "water",
        },
    })?;
    fs::write(appicon_dir.join("Contents.json"), json).await?;
    Ok(())
}

async fn write_android_icon_resources(
    icon: &IconSource,
    icon_background: Option<[u8; 3]>,
    backend_path: &Path,
) -> eyre::Result<()> {
    let drawable_dir = backend_path.join(ANDROID_DRAWABLE_DIR);
    fs::create_dir_all(&drawable_dir).await?;

    let foreground = render_android_foreground(icon, icon_background)?;
    write_png(
        &foreground,
        &drawable_dir.join("ic_launcher_foreground.png"),
    )
    .await?;

    for (dir, size) in ANDROID_MIPMAP_DIRS {
        let target_dir = backend_path.join("app/src/main/res").join(dir);
        fs::create_dir_all(&target_dir).await?;
        let icon = icon.render(*size)?;
        write_png(&icon, &target_dir.join("ic_launcher.png")).await?;
        write_png(&icon, &target_dir.join("ic_launcher_round.png")).await?;
    }

    Ok(())
}

/// One `<color>` resource.
#[derive(Debug, PartialEq, Eq)]
struct AndroidColor {
    name: &'static str,
    value: HexColor,
}

/// One theme attribute bound to a color resource.
#[derive(Debug, PartialEq, Eq)]
struct AndroidThemeItem {
    attr: &'static str,
    color_name: &'static str,
}

#[derive(Template)]
#[template(path = "src/templates/android_res/colors.xml.tpl", escape = "xml")]
struct AndroidColorsTemplate {
    colors: Vec<AndroidColor>,
}

#[derive(Template)]
#[template(path = "src/templates/android_res/themes.xml.tpl", escape = "xml")]
struct AndroidThemesTemplate {
    theme_items: Vec<AndroidThemeItem>,
    launch_background: bool,
    launch_artwork: bool,
}

#[derive(Template)]
#[template(
    path = "src/templates/android_res/ic_launch_artwork.xml.tpl",
    escape = "xml"
)]
struct AndroidLaunchArtworkTemplate;

/// The theme slots and the resources they bind to, in the order the
/// generated `colors.xml` and `themes.xml` list them.
const fn android_theme_slots(
    theme: &ThemeConfig,
) -> [(&'static str, &'static str, Option<HexColor>); 8] {
    [
        (
            "android:colorBackground",
            "waterui_background",
            theme.background,
        ),
        ("colorSurface", "waterui_surface", theme.surface),
        (
            "colorSurfaceVariant",
            "waterui_surface_variant",
            theme.surface_variant,
        ),
        ("colorOutline", "waterui_border", theme.border),
        ("colorOnSurface", "waterui_foreground", theme.foreground),
        (
            "colorOnSurfaceVariant",
            "waterui_muted_foreground",
            theme.muted_foreground,
        ),
        ("colorPrimary", "waterui_accent", theme.accent),
        (
            "colorOnPrimary",
            "waterui_accent_foreground",
            theme.accent_foreground,
        ),
    ]
}

/// The color behind an adaptive icon's foreground: the artwork's own edge
/// color so the two layers join seamlessly, else `fallback`.
fn android_adaptive_background(edge: Option<[u8; 3]>, fallback: HexColor) -> HexColor {
    edge.map_or(fallback, HexColor::from_rgb)
}

async fn write_android_theme_files(
    theme: Option<&ThemeConfig>,
    icon_background: Option<[u8; 3]>,
    launch: &LaunchAssets,
    backend_path: &Path,
) -> eyre::Result<()> {
    let values_dir = backend_path.join(ANDROID_VALUES_DIR);
    let values_night_dir = backend_path.join(ANDROID_VALUES_NIGHT_DIR);
    fs::create_dir_all(&values_dir).await?;
    fs::create_dir_all(&values_night_dir).await?;

    let colors = android_colors(theme, icon_background, launch)?;
    fs::write(values_dir.join("colors.xml"), render_android(&colors.day)?).await?;
    match colors.night {
        Some(night) => {
            fs::write(values_night_dir.join("colors.xml"), render_android(&night)?).await?;
        }
        None => remove_file_if_exists(values_night_dir.join("colors.xml")).await?,
    }

    let plan = launch.plan();
    let themes = AndroidThemesTemplate {
        theme_items: theme.map_or_else(Vec::new, |theme| {
            android_theme_slots(theme)
                .into_iter()
                .filter(|(_, _, value)| value.is_some())
                .map(|(attr, color_name, _)| AndroidThemeItem { attr, color_name })
                .collect()
        }),
        launch_background: plan.background(ColorScheme::Light).is_some(),
        launch_artwork: launch.has_artwork(),
    };
    fs::write(values_dir.join("themes.xml"), render_android(&themes)?).await?;
    // The theme is appearance-neutral: every color it names resolves through
    // `values-night/colors.xml`, so a night copy of it would only be a
    // duplicate. Earlier CLIs wrote one; drop it.
    remove_file_if_exists(values_night_dir.join("themes.xml")).await?;
    Ok(())
}

fn render_android<T: Template>(template: &T) -> eyre::Result<String> {
    template
        .render()
        .map_err(|error| eyre::eyre!("Failed to render Android resource: {error}"))
}

/// The day color table and, when the dark launch background differs, the
/// night table that overrides it.
struct AndroidColorTables {
    day: AndroidColorsTemplate,
    night: Option<AndroidColorsTemplate>,
}

fn android_colors(
    theme: Option<&ThemeConfig>,
    icon_background: Option<[u8; 3]>,
    launch: &LaunchAssets,
) -> eyre::Result<AndroidColorTables> {
    let accent = theme
        .and_then(|theme| theme.accent)
        .unwrap_or(DEFAULT_ACCENT);
    let mut colors = vec![AndroidColor {
        name: "ic_launcher_background",
        value: android_adaptive_background(icon_background, accent),
    }];
    if let Some(theme) = theme {
        colors.extend(
            android_theme_slots(theme)
                .into_iter()
                .filter_map(|(_, name, value)| value.map(|value| AndroidColor { name, value })),
        );
    }

    let plan = launch.plan();
    let launch_background = plan.background(ColorScheme::Light).copied();
    if let Some(background) = launch_background {
        colors.push(AndroidColor {
            name: "waterui_launch_background",
            value: background,
        });
    }
    if let Some(artwork) = launch.artwork() {
        colors.push(AndroidColor {
            name: "ic_launch_artwork_background",
            value: android_adaptive_background(
                artwork.edge_color()?,
                launch_background.unwrap_or(accent),
            ),
        });
    }

    let night = plan
        .has_distinct_dark_background()
        .then(|| plan.background(ColorScheme::Dark).copied())
        .flatten()
        .map(|dark| AndroidColorsTemplate {
            colors: vec![AndroidColor {
                name: "waterui_launch_background",
                value: dark,
            }],
        });
    Ok(AndroidColorTables {
        day: AndroidColorsTemplate { colors },
        night,
    })
}

/// Stages `Launch.*` as the adaptive splash icon: Android masks the splash
/// icon to a circle, so the artwork goes through the same safe-zone
/// placement as the launcher icon.
async fn write_android_launch_artwork(
    launch: &LaunchAssets,
    backend_path: &Path,
) -> eyre::Result<()> {
    let drawable_dir = backend_path.join(ANDROID_DRAWABLE_DIR);
    let icon_xml = drawable_dir.join("ic_launch_artwork.xml");
    let foreground_png = drawable_dir.join("ic_launch_artwork_foreground.png");
    let Some(artwork) = launch.artwork() else {
        remove_file_if_exists(icon_xml).await?;
        remove_file_if_exists(foreground_png).await?;
        return Ok(());
    };
    fs::create_dir_all(&drawable_dir).await?;
    write_png(
        &render_android_foreground(artwork, artwork.edge_color()?)?,
        &foreground_png,
    )
    .await?;
    fs::write(icon_xml, render_android(&AndroidLaunchArtworkTemplate)?).await?;
    Ok(())
}

fn component_string(value: u8) -> String {
    format!("{:.6}", f32::from(value) / 255.0)
}

async fn write_png(image: &image::RgbaImage, path: &Path) -> eyre::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(path, encode_png(image)?).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_assets_planner::LaunchConfig;

    #[test]
    fn android_icon_resources_write_launcher_pngs() {
        smol::block_on(async {
            let tempdir = tempfile::tempdir().expect("failed to create tempdir");
            let backend_path = tempdir.path();

            let icon = IconSource::default_logo();
            let background = icon.edge_color().expect("edge probe must render");
            write_android_icon_resources(&icon, background, backend_path)
                .await
                .expect("failed to write android icon resources");

            let foreground = backend_path
                .join(ANDROID_DRAWABLE_DIR)
                .join("ic_launcher_foreground.png");
            let decoded = image::open(&foreground).expect("foreground must be a decodable png");
            assert_eq!(decoded.width(), decoded.height());

            for (dir, size) in ANDROID_MIPMAP_DIRS {
                let launcher = backend_path
                    .join("app/src/main/res")
                    .join(dir)
                    .join("ic_launcher.png");
                let decoded = image::open(&launcher).expect("launcher must be a decodable png");
                assert_eq!(decoded.width(), *size, "wrong launcher size in {dir}");
            }
        });
    }

    #[test]
    fn apple_color_set_carries_a_dark_appearance_only_when_given() {
        let temp = tempfile::tempdir().expect("tempdir");
        smol::block_on(async {
            let light = HexColor::from_rgb([0x0B, 0x1E, 0x3F]);
            let dark = HexColor::from_rgb([0, 0, 0]);
            write_apple_color_set("LaunchBackground", light, Some(dark), temp.path())
                .await
                .expect("color set with dark appearance");
            let json: serde_json::Value = serde_json::from_slice(
                &std::fs::read(temp.path().join("LaunchBackground.colorset/Contents.json"))
                    .expect("contents"),
            )
            .expect("valid json");
            let colors = json["colors"].as_array().expect("colors");
            assert_eq!(colors.len(), 2);
            assert!(colors[0].get("appearances").is_none());
            assert_eq!(colors[0]["color"]["components"]["red"], "0.043137");
            assert_eq!(colors[1]["appearances"][0]["value"], "dark");
            assert_eq!(colors[1]["color"]["components"]["red"], "0.000000");

            write_apple_color_set("AccentColor", light, None, temp.path())
                .await
                .expect("color set without dark appearance");
            let json: serde_json::Value = serde_json::from_slice(
                &std::fs::read(temp.path().join("AccentColor.colorset/Contents.json"))
                    .expect("contents"),
            )
            .expect("valid json");
            assert_eq!(json["colors"].as_array().expect("colors").len(), 1);
        });
    }

    #[test]
    fn apple_launch_image_is_written_at_three_scales() {
        let temp = tempfile::tempdir().expect("tempdir");
        smol::block_on(async {
            write_apple_launch_image(&IconSource::default_logo(), temp.path())
                .await
                .expect("launch image set");
            let set = temp.path().join("LaunchImage.imageset");
            for (scale, factor) in [("1x", 1), ("2x", 2), ("3x", 3)] {
                let decoded = image::open(set.join(format!("LaunchImage@{scale}.png")))
                    .expect("launch image must decode");
                assert_eq!(decoded.width(), APPLE_LAUNCH_IMAGE_POINTS * factor);
                assert_eq!(decoded.height(), decoded.width());
            }
            let json: serde_json::Value = serde_json::from_slice(
                &std::fs::read(set.join("Contents.json")).expect("contents"),
            )
            .expect("valid json");
            assert_eq!(json["images"].as_array().expect("images").len(), 3);
        });
    }

    #[test]
    fn android_colors_derive_the_launcher_background_and_carry_the_launch_colors() {
        let theme = ThemeConfig {
            accent: Some(DEFAULT_ACCENT),
            ..ThemeConfig::default()
        };
        let no_launch = LaunchAssets {
            plan: LaunchPlan::resolve(None, None, &empty_manifest()),
            artwork: None,
            app_icon: IconSource::default_logo(),
        };
        let colors = android_colors(Some(&theme), Some([255, 255, 255]), &no_launch)
            .expect("colors must build");
        assert_eq!(
            colors.day.colors[0],
            AndroidColor {
                name: "ic_launcher_background",
                value: HexColor::from_rgb([255, 255, 255])
            },
            "derived icon background must win over the accent color"
        );
        assert!(colors.night.is_none());
        let xml = colors.day.render().expect("colors.xml renders");
        assert!(
            xml.contains("<color name=\"waterui_accent\">#0A84FF</color>"),
            "{xml}"
        );
        assert!(!xml.contains("waterui_launch_background"));

        let colors = android_colors(Some(&theme), None, &no_launch).expect("colors must build");
        assert_eq!(colors.day.colors[0].value, DEFAULT_ACCENT);

        let launch = LaunchAssets {
            plan: LaunchPlan::resolve(
                Some(&LaunchConfig {
                    background: Some(HexColor::from_rgb([0x0B, 0x1E, 0x3F])),
                    background_dark: Some(HexColor::from_rgb([0, 0, 0])),
                }),
                None,
                &empty_manifest(),
            ),
            artwork: Some(IconSource::default_logo()),
            app_icon: IconSource::default_logo(),
        };
        let colors = android_colors(None, None, &launch).expect("colors must build");
        let day = colors.day.render().expect("colors.xml renders");
        assert!(day.contains("<color name=\"waterui_launch_background\">#0B1E3F</color>"));
        assert!(day.contains("<color name=\"ic_launch_artwork_background\">"));
        let night = colors
            .night
            .expect("distinct dark background needs a night table");
        assert_eq!(
            night.colors,
            vec![AndroidColor {
                name: "waterui_launch_background",
                value: HexColor::from_rgb([0, 0, 0])
            }]
        );
    }

    #[test]
    fn android_themes_bind_only_configured_slots_and_the_launch_theme() {
        let xml = AndroidThemesTemplate {
            theme_items: vec![AndroidThemeItem {
                attr: "colorPrimary",
                color_name: "waterui_accent",
            }],
            launch_background: true,
            launch_artwork: false,
        }
        .render()
        .expect("themes.xml renders");
        assert!(xml.contains("<item name=\"colorPrimary\">@color/waterui_accent</item>"));
        assert!(!xml.contains("colorSurface"));
        assert!(xml.contains("parent=\"Theme.SplashScreen\""));
        assert!(
            xml.contains("<item name=\"postSplashScreenTheme\">@style/Theme.WaterUIApp</item>")
        );
        assert!(xml.contains("windowSplashScreenBackground"));
        assert!(!xml.contains("windowSplashScreenAnimatedIcon"));

        let xml = AndroidThemesTemplate {
            theme_items: Vec::new(),
            launch_background: false,
            launch_artwork: true,
        }
        .render()
        .expect("themes.xml renders");
        assert!(!xml.contains("windowSplashScreenBackground"));
        assert!(xml.contains(
            "<item name=\"windowSplashScreenAnimatedIcon\">@drawable/ic_launch_artwork</item>"
        ));
    }

    fn empty_manifest() -> BundleManifest {
        BundleManifest {
            crate_root: PathBuf::new(),
            assets_root: PathBuf::new(),
            mounts: Vec::new(),
            assets: Vec::new(),
        }
    }
}
