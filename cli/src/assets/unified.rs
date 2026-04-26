use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, Context};
use image::ImageEncoder;
use resvg::{self, tiny_skia, usvg};
use serde::Serialize;
use sha2::{Digest, Sha256};
use smol::fs;
use waterui_assets::AssetKind;
use waterui_assets_planner::{AssetRole, BundleManifest, PlannedAsset, ThemeConfig, plan_bundle};

use crate::project::Project;

const ASSET_ROOT_DIR: &str = "waterui_assets";
const ANDROID_VALUES_DIR: &str = "app/src/main/res/values";
const ANDROID_VALUES_NIGHT_DIR: &str = "app/src/main/res/values-night";
const ANDROID_DRAWABLE_DIR: &str = "app/src/main/res/drawable";
const ANDROID_DEFAULT_LAUNCHER_FOREGROUND_XML: &str =
    include_str!("../templates/android/app/src/main/res/drawable/ic_launcher_foreground.xml.tpl");
const ANDROID_MIPMAP_DIRS: &[(&str, u32)] = &[
    ("mipmap-mdpi", 48),
    ("mipmap-hdpi", 72),
    ("mipmap-xhdpi", 96),
    ("mipmap-xxhdpi", 144),
    ("mipmap-xxxhdpi", 192),
];

pub async fn stage_for_apple(project: &Project, dest_dir: &Path) -> eyre::Result<()> {
    let manifest = build_manifest(project)?;
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
        .and_then(|theme| theme.accent.as_deref());

    if let Some(icon) = manifest
        .assets
        .iter()
        .find(|asset| asset.role == AssetRole::AppIcon)
    {
        write_apple_app_icon(icon, &xcassets_dest).await?;
    } else {
        write_generated_apple_app_icon(accent, &xcassets_dest).await?;
    }

    if let Some(theme) = project.manifest().theme.as_ref() {
        if !theme.is_empty() {
            write_apple_theme_json(theme, dest_dir).await?;
        }
        write_apple_accent_color(accent, &xcassets_dest).await?;
    } else {
        write_apple_accent_color(None, &xcassets_dest).await?;
    }

    Ok(())
}

pub async fn stage_for_android(project: &Project, backend_path: &Path) -> eyre::Result<()> {
    let manifest = build_manifest(project)?;
    let assets_dest = backend_path
        .join("app/src/main/assets")
        .join(ASSET_ROOT_DIR);
    reset_dir(&assets_dest).await?;
    copy_manifest_assets(&manifest, &assets_dest).await?;
    write_manifest_stamp(&manifest, &assets_dest).await?;

    let res_root = backend_path.join("app/src/main/res");
    fs::create_dir_all(&res_root).await?;

    let theme = project.manifest().theme.as_ref();
    write_android_theme_files(theme, backend_path).await?;

    if let Some(icon) = manifest
        .assets
        .iter()
        .find(|asset| asset.role == AssetRole::AppIcon)
    {
        remove_file_if_exists(res_root.join("drawable/ic_launcher_foreground.xml")).await?;
        write_android_icon_resources(icon, theme, backend_path).await?;
    } else {
        write_default_android_icon_resources(backend_path).await?;
    }

    Ok(())
}

pub async fn stage_for_gtk(project: &Project, resources_dir: &Path) -> eyre::Result<()> {
    let manifest = build_manifest(project)?;
    let assets_dest = resources_dir.join(ASSET_ROOT_DIR);
    reset_dir(&assets_dest).await?;
    copy_manifest_assets(&manifest, &assets_dest).await?;
    write_manifest_stamp(&manifest, &assets_dest).await?;
    remove_file_if_exists(resources_dir.join("resources.gresource")).await?;
    remove_file_if_exists(resources_dir.join("resources.gresource.xml")).await?;
    Ok(())
}

pub fn scan_project_fonts(project: &Project) -> eyre::Result<Vec<super::ResolvedFont>> {
    let manifest = build_manifest(project)?;
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

fn build_manifest(project: &Project) -> eyre::Result<BundleManifest> {
    plan_bundle(project.root(), project.assets_path()).map_err(Into::into)
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

async fn write_apple_theme_json(theme: &ThemeConfig, dest_dir: &Path) -> eyre::Result<()> {
    for value in [
        theme.background.as_deref(),
        theme.surface.as_deref(),
        theme.surface_variant.as_deref(),
        theme.border.as_deref(),
        theme.foreground.as_deref(),
        theme.muted_foreground.as_deref(),
        theme.accent.as_deref(),
        theme.accent_foreground.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        validate_hex_color(value)?;
    }
    let bytes = serde_json::to_vec_pretty(theme)?;
    fs::write(dest_dir.join("WaterUITheme.json"), bytes).await?;
    Ok(())
}

async fn write_apple_accent_color(accent: Option<&str>, xcassets_dest: &Path) -> eyre::Result<()> {
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

    let accent = accent.unwrap_or("#0A84FF");
    let [red, green, blue] = parse_rgb(accent)?;
    let accent_dir = xcassets_dest.join("AccentColor.colorset");
    fs::create_dir_all(&accent_dir).await?;
    let red = component_string(red);
    let green = component_string(green);
    let blue = component_string(blue);
    let json = serde_json::to_vec_pretty(&Contents {
        colors: vec![ColorItem {
            idiom: "universal",
            color: Color {
                color_space: "srgb",
                components: Components {
                    red: &red,
                    green: &green,
                    blue: &blue,
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

async fn write_apple_app_icon(icon: &PlannedAsset, xcassets_dest: &Path) -> eyre::Result<()> {
    let source = load_icon_image(&icon.source_path)?;
    write_apple_app_icon_from_source(&source, xcassets_dest).await
}

async fn write_generated_apple_app_icon(
    accent: Option<&str>,
    xcassets_dest: &Path,
) -> eyre::Result<()> {
    let source = generate_default_app_icon(accent)?;
    write_apple_app_icon_from_source(&source, xcassets_dest).await
}

async fn write_apple_app_icon_from_source(
    source: &image::DynamicImage,
    xcassets_dest: &Path,
) -> eyre::Result<()> {
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
        let file_name = format!("AppIcon-{size}@{scale}.png");
        write_png(
            &source.resize_exact(pixels, pixels, image::imageops::FilterType::Lanczos3),
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
    icon: &PlannedAsset,
    _theme: Option<&ThemeConfig>,
    backend_path: &Path,
) -> eyre::Result<()> {
    let source = load_icon_image(&icon.source_path)?;
    let drawable_dir = backend_path.join(ANDROID_DRAWABLE_DIR);
    fs::create_dir_all(&drawable_dir).await?;

    let foreground = render_android_foreground(&source);
    write_png(
        &foreground,
        &drawable_dir.join("ic_launcher_foreground.png"),
    )
    .await?;

    for (dir, size) in ANDROID_MIPMAP_DIRS {
        let target_dir = backend_path.join("app/src/main/res").join(dir);
        fs::create_dir_all(&target_dir).await?;
        let icon = source.resize_exact(*size, *size, image::imageops::FilterType::Lanczos3);
        write_png(&icon, &target_dir.join("ic_launcher.png")).await?;
        write_png(&icon, &target_dir.join("ic_launcher_round.png")).await?;
    }

    Ok(())
}

async fn write_default_android_icon_resources(backend_path: &Path) -> eyre::Result<()> {
    let drawable_dir = backend_path.join(ANDROID_DRAWABLE_DIR);
    fs::create_dir_all(&drawable_dir).await?;
    remove_file_if_exists(drawable_dir.join("ic_launcher_foreground.png")).await?;
    fs::write(
        drawable_dir.join("ic_launcher_foreground.xml"),
        ANDROID_DEFAULT_LAUNCHER_FOREGROUND_XML,
    )
    .await?;

    for (dir, _) in ANDROID_MIPMAP_DIRS {
        let target_dir = backend_path.join("app/src/main/res").join(dir);
        remove_file_if_exists(target_dir.join("ic_launcher.png")).await?;
        remove_file_if_exists(target_dir.join("ic_launcher_round.png")).await?;
    }

    Ok(())
}

fn generate_default_app_icon(accent: Option<&str>) -> eyre::Result<image::DynamicImage> {
    let [red, green, blue] = parse_rgb(accent.unwrap_or("#0A84FF"))?;
    let mut image = image::RgbaImage::from_pixel(1024, 1024, image::Rgba([red, green, blue, 255]));

    let circle_radius = 320_i32;
    let center = 512_i32;
    for y in 0..1024_i32 {
        for x in 0..1024_i32 {
            let dx = x - center;
            let dy = y - center;
            if dx * dx + dy * dy <= circle_radius * circle_radius {
                image.put_pixel(
                    x.cast_unsigned(),
                    y.cast_unsigned(),
                    image::Rgba([255, 255, 255, 235]),
                );
            }
        }
    }

    Ok(image::DynamicImage::ImageRgba8(image))
}

fn render_android_foreground(source: &image::DynamicImage) -> image::DynamicImage {
    let canvas_size = 432_u32;
    let icon_size = 312_u32;
    let mut canvas =
        image::RgbaImage::from_pixel(canvas_size, canvas_size, image::Rgba([0, 0, 0, 0]));
    let resized = source
        .resize_exact(icon_size, icon_size, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let inset = (canvas_size - icon_size) / 2;
    image::imageops::overlay(&mut canvas, &resized, i64::from(inset), i64::from(inset));
    image::DynamicImage::ImageRgba8(canvas)
}

async fn write_android_theme_files(
    theme: Option<&ThemeConfig>,
    backend_path: &Path,
) -> eyre::Result<()> {
    let values_dir = backend_path.join(ANDROID_VALUES_DIR);
    let values_night_dir = backend_path.join(ANDROID_VALUES_NIGHT_DIR);
    fs::create_dir_all(&values_dir).await?;
    fs::create_dir_all(&values_night_dir).await?;

    let colors_xml = build_android_colors_xml(theme)?;
    let themes_xml = build_android_themes_xml(theme);
    fs::write(values_dir.join("colors.xml"), &colors_xml).await?;
    fs::write(values_dir.join("themes.xml"), &themes_xml).await?;
    fs::write(values_night_dir.join("themes.xml"), &themes_xml).await?;
    Ok(())
}

fn build_android_colors_xml(theme: Option<&ThemeConfig>) -> eyre::Result<String> {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<resources>\n");
    let background = theme
        .and_then(|value| value.accent.as_deref())
        .unwrap_or("#0A84FF");
    validate_hex_color(background)?;
    let _ = writeln!(
        &mut xml,
        "    <color name=\"ic_launcher_background\">{background}</color>"
    );
    if let Some(theme) = theme {
        write_android_color(&mut xml, "waterui_background", theme.background.as_deref())?;
        write_android_color(&mut xml, "waterui_surface", theme.surface.as_deref())?;
        write_android_color(
            &mut xml,
            "waterui_surface_variant",
            theme.surface_variant.as_deref(),
        )?;
        write_android_color(&mut xml, "waterui_border", theme.border.as_deref())?;
        write_android_color(&mut xml, "waterui_foreground", theme.foreground.as_deref())?;
        write_android_color(
            &mut xml,
            "waterui_muted_foreground",
            theme.muted_foreground.as_deref(),
        )?;
        write_android_color(&mut xml, "waterui_accent", theme.accent.as_deref())?;
        write_android_color(
            &mut xml,
            "waterui_accent_foreground",
            theme.accent_foreground.as_deref(),
        )?;
    }
    xml.push_str("</resources>\n");
    Ok(xml)
}

fn write_android_color(xml: &mut String, name: &str, value: Option<&str>) -> eyre::Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    validate_hex_color(value)?;
    let _ = writeln!(xml, "    <color name=\"{name}\">{value}</color>");
    Ok(())
}

fn build_android_themes_xml(theme: Option<&ThemeConfig>) -> String {
    let mut xml = String::from(
        "<resources>\n    <style name=\"Theme.WaterUIApp\" parent=\"Theme.Material3.DayNight.NoActionBar\">\n",
    );
    if let Some(theme) = theme {
        maybe_theme_item(
            &mut xml,
            "android:colorBackground",
            theme.background.as_deref(),
            "waterui_background",
        );
        maybe_theme_item(
            &mut xml,
            "colorSurface",
            theme.surface.as_deref(),
            "waterui_surface",
        );
        maybe_theme_item(
            &mut xml,
            "colorSurfaceVariant",
            theme.surface_variant.as_deref(),
            "waterui_surface_variant",
        );
        maybe_theme_item(
            &mut xml,
            "colorOutline",
            theme.border.as_deref(),
            "waterui_border",
        );
        maybe_theme_item(
            &mut xml,
            "colorOnSurface",
            theme.foreground.as_deref(),
            "waterui_foreground",
        );
        maybe_theme_item(
            &mut xml,
            "colorOnSurfaceVariant",
            theme.muted_foreground.as_deref(),
            "waterui_muted_foreground",
        );
        maybe_theme_item(
            &mut xml,
            "colorPrimary",
            theme.accent.as_deref(),
            "waterui_accent",
        );
        maybe_theme_item(
            &mut xml,
            "colorOnPrimary",
            theme.accent_foreground.as_deref(),
            "waterui_accent_foreground",
        );
    }
    xml.push_str("    </style>\n</resources>\n");
    xml
}

fn maybe_theme_item(xml: &mut String, attr: &str, value: Option<&str>, color_name: &str) {
    if value.is_some() {
        let _ = writeln!(
            xml,
            "        <item name=\"{attr}\">@color/{color_name}</item>"
        );
    }
}

fn validate_hex_color(value: &str) -> eyre::Result<()> {
    let raw = value.strip_prefix('#').unwrap_or(value);
    if raw.len() != 6 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        eyre::bail!("Invalid theme color '{value}'; expected #RRGGBB");
    }
    Ok(())
}

fn parse_rgb(value: &str) -> eyre::Result<[u8; 3]> {
    validate_hex_color(value)?;
    let raw = value.strip_prefix('#').unwrap_or(value);
    Ok([
        u8::from_str_radix(&raw[0..2], 16)?,
        u8::from_str_radix(&raw[2..4], 16)?,
        u8::from_str_radix(&raw[4..6], 16)?,
    ])
}

fn component_string(value: u8) -> String {
    format!("{:.6}", f32::from(value) / 255.0)
}

fn load_icon_image(path: &Path) -> eyre::Result<image::DynamicImage> {
    if path
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
    {
        return load_svg_icon_image(path);
    }

    let image = image::open(path)
        .wrap_err_with(|| format!("Failed to decode app icon source '{}'", path.display()))?;
    ensure_square_icon_dimensions(path, image.width(), image.height())?;
    Ok(image)
}

fn load_svg_icon_image(path: &Path) -> eyre::Result<image::DynamicImage> {
    let svg_data = std::fs::read(path)
        .wrap_err_with(|| format!("Failed to read SVG app icon source '{}'", path.display()))?;
    let mut options = usvg::Options {
        resources_dir: std::fs::canonicalize(path)
            .ok()
            .and_then(|absolute| absolute.parent().map(Path::to_path_buf)),
        ..usvg::Options::default()
    };
    options.fontdb_mut().load_system_fonts();

    let tree = usvg::Tree::from_data(&svg_data, &options).map_err(|error| {
        eyre::eyre!(
            "Failed to parse SVG app icon source '{}': {error}",
            path.display()
        )
    })?;
    let size = tree.size();
    if (size.width() - size.height()).abs() > f32::EPSILON {
        eyre::bail!(
            "App icon source '{}' must be square, got {}x{}",
            path.display(),
            size.width(),
            size.height()
        );
    }

    let raster_size = size.to_int_size();
    let mut pixmap =
        tiny_skia::Pixmap::new(raster_size.width(), raster_size.height()).ok_or_else(|| {
            eyre::eyre!(
                "Failed to allocate raster surface for SVG app icon '{}'",
                path.display()
            )
        })?;
    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());
    let png = pixmap.encode_png().map_err(|error| {
        eyre::eyre!(
            "Failed to encode rasterized SVG app icon '{}': {error}",
            path.display()
        )
    })?;
    let image = image::load_from_memory(&png).map_err(|error| {
        eyre::eyre!(
            "Failed to decode rasterized SVG app icon '{}': {error}",
            path.display()
        )
    })?;
    ensure_square_icon_dimensions(path, image.width(), image.height())?;
    Ok(image)
}

fn ensure_square_icon_dimensions(path: &Path, width: u32, height: u32) -> eyre::Result<()> {
    if width != height {
        eyre::bail!(
            "App icon source '{}' must be square, got {}x{}",
            path.display(),
            width,
            height
        );
    }
    Ok(())
}

async fn write_png(image: &image::DynamicImage, path: &Path) -> eyre::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let rgba = image.to_rgba8();
    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        &mut png,
        image::codecs::png::CompressionType::Best,
        image::codecs::png::FilterType::Adaptive,
    );
    encoder.write_image(
        rgba.as_raw(),
        rgba.width(),
        rgba.height(),
        image::ExtendedColorType::Rgba8,
    )?;
    fs::write(path, png).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_default_android_icon_resources_restores_vector_foreground() {
        smol::block_on(async {
            let tempdir = tempfile::tempdir().expect("failed to create tempdir");
            let backend_path = tempdir.path();
            let drawable_dir = backend_path.join(ANDROID_DRAWABLE_DIR);
            fs::create_dir_all(&drawable_dir)
                .await
                .expect("failed to create drawable dir");
            fs::write(drawable_dir.join("ic_launcher_foreground.png"), b"stale")
                .await
                .expect("failed to create stale foreground png");

            for (dir, _) in ANDROID_MIPMAP_DIRS {
                let target_dir = backend_path.join("app/src/main/res").join(dir);
                fs::create_dir_all(&target_dir)
                    .await
                    .expect("failed to create mipmap dir");
                fs::write(target_dir.join("ic_launcher.png"), b"stale")
                    .await
                    .expect("failed to create stale launcher png");
                fs::write(target_dir.join("ic_launcher_round.png"), b"stale")
                    .await
                    .expect("failed to create stale round launcher png");
            }

            write_default_android_icon_resources(backend_path)
                .await
                .expect("failed to write default android icon resources");

            assert_eq!(
                fs::read_to_string(drawable_dir.join("ic_launcher_foreground.xml"))
                    .await
                    .expect("failed to read foreground xml"),
                ANDROID_DEFAULT_LAUNCHER_FOREGROUND_XML
            );
            assert!(
                fs::metadata(drawable_dir.join("ic_launcher_foreground.png"))
                    .await
                    .is_err(),
                "stale foreground png should be removed"
            );

            for (dir, _) in ANDROID_MIPMAP_DIRS {
                let target_dir = backend_path.join("app/src/main/res").join(dir);
                assert!(
                    fs::metadata(target_dir.join("ic_launcher.png"))
                        .await
                        .is_err(),
                    "stale launcher png should be removed from {dir}"
                );
                assert!(
                    fs::metadata(target_dir.join("ic_launcher_round.png"))
                        .await
                        .is_err(),
                    "stale round launcher png should be removed from {dir}"
                );
            }
        });
    }
}
