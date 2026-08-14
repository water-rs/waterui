use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, Context};
use image::ImageEncoder;
use serde::Serialize;
use sha2::{Digest, Sha256};
use smol::fs;
use waterui_assets::AssetKind;
use waterui_assets_planner::{AssetRole, BundleManifest, PlannedAsset, ThemeConfig, plan_bundle};

use super::icon::{
    IconSource, encode_macos_icns, hex_color, render_android_foreground, render_apple_icon,
};
use crate::project::Project;

const ASSET_ROOT_DIR: &str = "waterui_assets";
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

    let icon = load_project_icon(&manifest)?;
    write_apple_app_icon(&icon, &xcassets_dest).await?;
    write_apple_accent_color(accent, &xcassets_dest).await?;

    Ok(())
}

/// Loads the project's `Icon.*` asset, or the bundled `WaterUI` logo when the
/// project does not provide one.
fn load_project_icon(manifest: &BundleManifest) -> eyre::Result<IconSource> {
    manifest
        .assets
        .iter()
        .find(|asset| asset.role == AssetRole::AppIcon)
        .map_or_else(
            || Ok(IconSource::default_logo()),
            |icon| IconSource::load(&icon.source_path),
        )
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

    let icon = load_project_icon(&manifest)?;
    let icon_background = icon.edge_color()?;

    let theme = project.manifest().theme.as_ref();
    write_android_theme_files(theme, icon_background, backend_path).await?;

    // Older CLI versions staged the foreground as a vector drawable; a PNG
    // and an XML with the same resource name cannot coexist.
    remove_file_if_exists(res_root.join("drawable/ic_launcher_foreground.xml")).await?;
    write_android_icon_resources(&icon, icon_background, backend_path).await?;

    Ok(())
}

/// Renders the project's macOS `.icns` app icon for hand-assembled bundles
/// (self-drawn backends that do not go through an Xcode asset catalog).
///
/// # Errors
///
/// Fails when the icon asset cannot be loaded or rendered.
pub fn macos_icns(project: &Project) -> eyre::Result<Vec<u8>> {
    let manifest = build_manifest(project)?;
    let icon = load_project_icon(&manifest)?;
    encode_macos_icns(&icon)
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

async fn write_android_theme_files(
    theme: Option<&ThemeConfig>,
    icon_background: Option<[u8; 3]>,
    backend_path: &Path,
) -> eyre::Result<()> {
    let values_dir = backend_path.join(ANDROID_VALUES_DIR);
    let values_night_dir = backend_path.join(ANDROID_VALUES_NIGHT_DIR);
    fs::create_dir_all(&values_dir).await?;
    fs::create_dir_all(&values_night_dir).await?;

    let colors_xml = build_android_colors_xml(theme, icon_background)?;
    let themes_xml = build_android_themes_xml(theme);
    fs::write(values_dir.join("colors.xml"), &colors_xml).await?;
    fs::write(values_dir.join("themes.xml"), &themes_xml).await?;
    fs::write(values_night_dir.join("themes.xml"), &themes_xml).await?;
    Ok(())
}

fn build_android_colors_xml(
    theme: Option<&ThemeConfig>,
    icon_background: Option<[u8; 3]>,
) -> eyre::Result<String> {
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<resources>\n");
    // The adaptive-icon background must match the icon's own background so
    // the two layers join seamlessly; the theme accent only stands in when
    // no background color can be derived from the icon.
    let derived = icon_background.map(hex_color);
    let background = derived.as_deref().unwrap_or_else(|| {
        theme
            .and_then(|value| value.accent.as_deref())
            .unwrap_or("#0A84FF")
    });
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

async fn write_png(image: &image::RgbaImage, path: &Path) -> eyre::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
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
    fs::write(path, png).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn colors_xml_uses_derived_icon_background_over_accent() {
        let theme = ThemeConfig {
            accent: Some("#0A84FF".to_string()),
            ..ThemeConfig::default()
        };
        let xml = build_android_colors_xml(Some(&theme), Some([255, 255, 255]))
            .expect("colors xml must build");
        assert!(
            xml.contains("<color name=\"ic_launcher_background\">#FFFFFF</color>"),
            "derived icon background must win over the accent color:\n{xml}"
        );

        let xml = build_android_colors_xml(Some(&theme), None).expect("colors xml must build");
        assert!(
            xml.contains("<color name=\"ic_launcher_background\">#0A84FF</color>"),
            "accent must remain the launcher background when none can be derived:\n{xml}"
        );
    }
}
