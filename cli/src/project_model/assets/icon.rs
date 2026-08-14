//! App-icon rendering shared by every staged platform.
//!
//! One square source image (SVG or raster) is turned into each platform's
//! launcher format: full-bleed squares where the OS applies its own mask
//! (iOS, iPadOS), the inset rounded-rectangle shape macOS expects, and
//! adaptive-icon layers that survive Android's circular mask. SVG sources are
//! re-rendered from vector data at every requested pixel size so the largest
//! formats stay sharp.

use std::ffi::OsStr;
use std::path::Path;

use color_eyre::eyre::{self, Context};
use resvg::{tiny_skia, usvg};

/// The official `WaterUI` logo, used as the app icon when a project does not
/// provide an `Icon.*` asset of its own.
const DEFAULT_APP_ICON_SVG: &str = include_str!("../../templates/icon.svg");

/// Side length of the Android adaptive-icon foreground drawable in pixels
/// (108 dp canvas at xxxhdpi density).
const ANDROID_FOREGROUND_CANVAS: u32 = 432;

/// Fraction of the adaptive-icon canvas guaranteed to survive every launcher
/// mask shape: the 66 dp safe-zone circle on the 108 dp canvas.
const ANDROID_SAFE_ZONE_RATIO: f64 = 66.0 / 108.0;

/// Fraction of the adaptive-icon canvas inside the 72 dp visible square.
/// Used when the icon's own background cannot be derived and the whole
/// artwork has to be placed as-is.
const ANDROID_VISIBLE_RATIO: f64 = 72.0 / 108.0;

/// macOS icon-grid geometry from Apple's design template: on a 1024 px
/// canvas the icon shape is an 824 px rounded rectangle with a 185.4 px
/// corner radius.
const MACOS_SHAPE_RATIO: f64 = 824.0 / 1024.0;
const MACOS_CORNER_RADIUS_RATIO: f64 = 185.4 / 1024.0;

/// Resolution used to inspect an icon's edges and content extent.
const PROBE_SIZE: u32 = 512;

/// Maximum per-channel difference for edge pixels to count as one uniform
/// background color.
const EDGE_COLOR_TOLERANCE: u8 = 8;

/// Minimum per-channel difference from the background for a pixel to count
/// as icon content.
const CONTENT_TOLERANCE: u8 = 10;

/// Upper bound for intermediate render sizes when scaling small artwork up
/// into the Android safe zone.
const MAX_RENDER_SIZE: u32 = 4096;

/// A loaded app-icon source that can be rendered at any pixel size.
pub enum IconSource {
    /// Vector source, re-rendered per size.
    Svg(Box<usvg::Tree>),
    /// Raster source, resampled per size.
    Raster(image::DynamicImage),
}

impl IconSource {
    /// Loads an icon source from a project asset path.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or decoded, or when the image is
    /// not square.
    pub fn load(path: &Path) -> eyre::Result<Self> {
        if path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"))
        {
            let data = std::fs::read(path)
                .wrap_err_with(|| format!("Failed to read SVG app icon '{}'", path.display()))?;
            let mut options = usvg::Options {
                resources_dir: std::fs::canonicalize(path)
                    .ok()
                    .and_then(|absolute| absolute.parent().map(Path::to_path_buf)),
                ..usvg::Options::default()
            };
            options.fontdb_mut().load_system_fonts();
            let tree = usvg::Tree::from_data(&data, &options).map_err(|error| {
                eyre::eyre!("Failed to parse SVG app icon '{}': {error}", path.display())
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
            Ok(Self::Svg(Box::new(tree)))
        } else {
            let image = image::open(path).wrap_err_with(|| {
                format!("Failed to decode app icon source '{}'", path.display())
            })?;
            if image.width() != image.height() {
                eyre::bail!(
                    "App icon source '{}' must be square, got {}x{}",
                    path.display(),
                    image.width(),
                    image.height()
                );
            }
            Ok(Self::Raster(image))
        }
    }

    /// The bundled `WaterUI` logo.
    ///
    /// The SVG is a compile-time asset, so a parse failure is a build defect
    /// and panics immediately.
    #[must_use]
    pub fn default_logo() -> Self {
        let tree =
            usvg::Tree::from_data(DEFAULT_APP_ICON_SVG.as_bytes(), &usvg::Options::default())
                .expect("bundled app-icon SVG must parse");
        Self::Svg(Box::new(tree))
    }

    /// Renders the source as a square RGBA image with the given side length.
    ///
    /// # Errors
    ///
    /// Fails when a raster surface of the requested size cannot be
    /// allocated.
    pub fn render(&self, size: u32) -> eyre::Result<image::RgbaImage> {
        match self {
            Self::Svg(tree) => {
                let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or_else(|| {
                    eyre::eyre!("Failed to allocate {size}x{size} surface for app icon")
                })?;
                let scale = to_f32(f64::from(size) / f64::from(tree.size().width()));
                resvg::render(
                    tree,
                    tiny_skia::Transform::from_scale(scale, scale),
                    &mut pixmap.as_mut(),
                );
                let mut image = image::RgbaImage::new(size, size);
                for (source, target) in pixmap.pixels().iter().zip(image.pixels_mut()) {
                    let color = source.demultiply();
                    *target =
                        image::Rgba([color.red(), color.green(), color.blue(), color.alpha()]);
                }
                Ok(image)
            }
            Self::Raster(source) => Ok(source
                .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
                .to_rgba8()),
        }
    }

    /// Detects the icon's own background color by probing its edges.
    ///
    /// Returns the edge color when every border pixel is opaque and within
    /// [`EDGE_COLOR_TOLERANCE`] of the others; otherwise the icon has no
    /// derivable background (a photograph, a gradient, or pre-masked
    /// transparent artwork).
    ///
    /// # Errors
    ///
    /// Fails when the probe render fails.
    pub fn edge_color(&self) -> eyre::Result<Option<[u8; 3]>> {
        let probe = self.render(PROBE_SIZE)?;
        Ok(uniform_edge_color(&probe))
    }
}

/// Renders one Apple icon slot.
///
/// The `mac` idiom receives the inset rounded-rectangle shape from Apple's
/// icon-grid template; iOS and iPadOS idioms are full-bleed squares that the
/// OS masks itself.
///
/// # Errors
///
/// Fails when rendering fails.
pub fn render_apple_icon(
    source: &IconSource,
    idiom: &str,
    pixels: u32,
) -> eyre::Result<image::RgbaImage> {
    if idiom != "mac" {
        return source.render(pixels);
    }

    let shape_size = round_to_u32(f64::from(pixels) * MACOS_SHAPE_RATIO);
    let radius = f64::from(pixels) * MACOS_CORNER_RADIUS_RATIO;
    let mut shape = source.render(shape_size)?;
    apply_rounded_rect_mask(&mut shape, radius);

    let mut canvas = image::RgbaImage::from_pixel(pixels, pixels, image::Rgba([0, 0, 0, 0]));
    let inset = i64::from((pixels - shape_size) / 2);
    image::imageops::overlay(&mut canvas, &shape, inset, inset);
    Ok(canvas)
}

/// Pixel sizes carried by a macOS `.icns` icon family.
const MACOS_ICNS_SIZES: &[u32] = &[16, 32, 64, 128, 256, 512, 1024];

/// Encodes the macOS icon family (`.icns`) for hand-assembled app bundles,
/// using the same inset rounded-rectangle shape as the asset-catalog `mac`
/// idiom.
///
/// # Errors
///
/// Fails when rendering fails or the icon family cannot be encoded.
pub fn encode_macos_icns(source: &IconSource) -> eyre::Result<Vec<u8>> {
    let mut family = icns::IconFamily::new();
    for &size in MACOS_ICNS_SIZES {
        let rendered = render_apple_icon(source, "mac", size)?;
        let image =
            icns::Image::from_data(icns::PixelFormat::RGBA, size, size, rendered.into_raw())
                .map_err(|error| {
                    eyre::eyre!("Failed to build {size}x{size} icns image: {error}")
                })?;
        family.add_icon(&image).map_err(|error| {
            eyre::eyre!("Failed to add {size}x{size} icon to icns family: {error}")
        })?;
    }
    let mut encoded = Vec::new();
    family
        .write(&mut encoded)
        .map_err(|error| eyre::eyre!("Failed to encode icns icon family: {error}"))?;
    Ok(encoded)
}

/// Pixel sizes embedded in a Windows `.ico` resource. ICO stores dimensions
/// in a byte, so 256 is the largest representable size.
const WINDOWS_ICO_SIZES: &[u32] = &[16, 24, 32, 48, 64, 128, 256];

/// Pixel sizes installed into the freedesktop hicolor icon theme.
pub const LINUX_HICOLOR_SIZES: &[u32] = &[16, 24, 32, 48, 64, 128, 256, 512];

/// Side length of the staged runtime window icon.
pub const WINDOW_ICON_SIZE: u32 = 256;

/// Encodes a multi-size Windows `.ico` for embedding into the packaged
/// executable's resources.
///
/// # Errors
///
/// Fails when rendering or ICO encoding fails.
pub fn encode_windows_ico(source: &IconSource) -> eyre::Result<Vec<u8>> {
    let mut frames = Vec::new();
    for &size in WINDOWS_ICO_SIZES {
        let rendered = source.render(size)?;
        frames.push(
            image::codecs::ico::IcoFrame::as_png(
                rendered.as_raw(),
                size,
                size,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|error| eyre::eyre!("Failed to build {size}px ICO frame: {error}"))?,
        );
    }
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::codecs::ico::IcoEncoder::new(&mut encoded)
        .encode_images(&frames)
        .map_err(|error| eyre::eyre!("Failed to encode ICO resource: {error}"))?;
    Ok(encoded.into_inner())
}

/// Encodes an image as PNG bytes.
///
/// # Errors
///
/// Fails when PNG encoding fails.
pub fn encode_png(image: &image::RgbaImage) -> eyre::Result<Vec<u8>> {
    use image::ImageEncoder as _;
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
    Ok(png)
}

/// Renders the Android adaptive-icon foreground layer.
///
/// When the icon's background color is known, the icon content (everything
/// that differs from that background) is scaled so its bounding box fits the
/// 66 dp safe-zone circle and centered on a transparent canvas; the matching
/// background color goes into the adaptive icon's background layer, so the
/// seam between the two layers is invisible and the glyph survives every
/// launcher mask. Without a derivable background the whole artwork is placed
/// inside the guaranteed-visible 72 dp square, as-is.
///
/// # Errors
///
/// Fails when rendering fails or when the icon has no content
/// distinguishable from its background.
pub fn render_android_foreground(
    source: &IconSource,
    edge_color: Option<[u8; 3]>,
) -> eyre::Result<image::RgbaImage> {
    let canvas_size = ANDROID_FOREGROUND_CANVAS;
    let mut canvas =
        image::RgbaImage::from_pixel(canvas_size, canvas_size, image::Rgba([0, 0, 0, 0]));

    let Some(background) = edge_color else {
        let visible = round_to_u32(f64::from(canvas_size) * ANDROID_VISIBLE_RATIO);
        let artwork = source.render(visible)?;
        let inset = i64::from((canvas_size - visible) / 2);
        image::imageops::overlay(&mut canvas, &artwork, inset, inset);
        return Ok(canvas);
    };

    let probe = flatten_over(&source.render(PROBE_SIZE)?, background);
    let bbox = content_bbox(&probe, background).ok_or_else(|| {
        eyre::eyre!("App icon has no content distinguishable from its background")
    })?;

    // Re-render the source at the size that makes the content's bounding box
    // fit the safe-zone circle exactly, so vector sources keep full
    // sharpness instead of being cropped out of the probe render.
    let safe_diameter = f64::from(canvas_size) * ANDROID_SAFE_ZONE_RATIO;
    let probe_diagonal = bbox.diagonal();
    let render_size = round_to_u32(f64::from(PROBE_SIZE) * safe_diameter / probe_diagonal)
        .clamp(1, MAX_RENDER_SIZE);
    let full = flatten_over(&source.render(render_size)?, background);
    let bbox = content_bbox(&full, background).ok_or_else(|| {
        eyre::eyre!("App icon has no content distinguishable from its background")
    })?;

    let crop = image::imageops::crop_imm(&full, bbox.x, bbox.y, bbox.width, bbox.height);
    let left = i64::from(canvas_size / 2) - i64::from(bbox.width / 2);
    let top = i64::from(canvas_size / 2) - i64::from(bbox.height / 2);
    image::imageops::overlay(&mut canvas, &crop.to_image(), left, top);
    Ok(canvas)
}

/// Formats a derived background color for Android `colors.xml`.
#[must_use]
pub fn hex_color(color: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2])
}

/// Axis-aligned bounding box of icon content, in pixels.
struct ContentBox {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl ContentBox {
    fn diagonal(&self) -> f64 {
        f64::from(self.width).hypot(f64::from(self.height))
    }
}

fn uniform_edge_color(image: &image::RgbaImage) -> Option<[u8; 3]> {
    let mut reference: Option<[u8; 3]> = None;
    let (width, height) = image.dimensions();
    for (x, y, pixel) in image.enumerate_pixels() {
        if x != 0 && y != 0 && x != width - 1 && y != height - 1 {
            continue;
        }
        let image::Rgba([red, green, blue, alpha]) = *pixel;
        if alpha != u8::MAX {
            return None;
        }
        match reference {
            None => reference = Some([red, green, blue]),
            Some(color) => {
                if color
                    .iter()
                    .zip([red, green, blue])
                    .any(|(a, b)| a.abs_diff(b) > EDGE_COLOR_TOLERANCE)
                {
                    return None;
                }
            }
        }
    }
    reference
}

/// Composites the image over an opaque background color.
fn flatten_over(image: &image::RgbaImage, background: [u8; 3]) -> image::RgbaImage {
    let mut out = image.clone();
    for pixel in out.pixels_mut() {
        let image::Rgba([red, green, blue, alpha]) = *pixel;
        let blend = |fg: u8, bg: u8| -> u8 {
            let alpha = u16::from(alpha);
            let value = u16::from(fg) * alpha + u16::from(bg) * (255 - alpha);
            u8::try_from((value + 127) / 255).expect("8-bit blend stays in range")
        };
        *pixel = image::Rgba([
            blend(red, background[0]),
            blend(green, background[1]),
            blend(blue, background[2]),
            u8::MAX,
        ]);
    }
    out
}

/// Bounding box of pixels that differ from the background color. Expects a
/// flattened (opaque) image.
fn content_bbox(image: &image::RgbaImage, background: [u8; 3]) -> Option<ContentBox> {
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0_u32;
    let mut max_y = 0_u32;
    let mut found = false;
    for (x, y, pixel) in image.enumerate_pixels() {
        let image::Rgba([red, green, blue, _]) = *pixel;
        if background
            .iter()
            .zip([red, green, blue])
            .all(|(a, b)| a.abs_diff(b) <= CONTENT_TOLERANCE)
        {
            continue;
        }
        found = true;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    found.then(|| ContentBox {
        x: min_x,
        y: min_y,
        width: max_x - min_x + 1,
        height: max_y - min_y + 1,
    })
}

/// Multiplies the image's alpha by an antialiased rounded-rectangle mask
/// covering the whole image.
fn apply_rounded_rect_mask(image: &mut image::RgbaImage, radius: f64) {
    let (width, height) = image.dimensions();
    let half_width = f64::from(width) / 2.0;
    let half_height = f64::from(height) / 2.0;
    let radius = radius.min(half_width).min(half_height);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let center_x = f64::from(x) + 0.5 - half_width;
        let center_y = f64::from(y) + 0.5 - half_height;
        let distance_x = center_x.abs() - (half_width - radius);
        let distance_y = center_y.abs() - (half_height - radius);
        let outside = distance_x.max(0.0).hypot(distance_y.max(0.0));
        let inside = distance_x.max(distance_y).min(0.0);
        let signed_distance = outside + inside - radius;
        let coverage = (0.5 - signed_distance).clamp(0.0, 1.0);
        pixel.0[3] = scale_alpha(pixel.0[3], coverage);
    }
}

fn scale_alpha(alpha: u8, coverage: f64) -> u8 {
    let scaled = (f64::from(alpha) * coverage).round().clamp(0.0, 255.0);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "value is clamped to the u8 range above"
    )]
    {
        scaled as u8
    }
}

fn round_to_u32(value: f64) -> u32 {
    let rounded = value.round().clamp(0.0, f64::from(u32::MAX));
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "value is clamped to the u32 range above"
    )]
    {
        rounded as u32
    }
}

const fn to_f32(value: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "icon scale factors are far inside f32 range"
    )]
    {
        value as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn center_distance(x: u32, y: u32, size: u32) -> f64 {
        let half = f64::from(size) / 2.0;
        (f64::from(x) + 0.5 - half).hypot(f64::from(y) + 0.5 - half)
    }

    #[test]
    fn default_logo_has_uniform_white_edges() {
        let logo = IconSource::default_logo();
        assert_eq!(
            logo.edge_color().expect("probe render must succeed"),
            Some([255, 255, 255]),
            "the bundled logo is black artwork on a white background"
        );
    }

    #[test]
    fn ios_icon_is_full_bleed() {
        let logo = IconSource::default_logo();
        let icon = render_apple_icon(&logo, "iphone", 180).expect("render must succeed");
        assert_eq!(icon.dimensions(), (180, 180));
        assert_eq!(
            *icon.get_pixel(0, 0),
            image::Rgba([255, 255, 255, 255]),
            "iOS icons are masked by the OS, so the artwork covers the corner"
        );
    }

    #[test]
    fn mac_icon_is_inset_and_rounded() {
        let logo = IconSource::default_logo();
        let size = 512;
        let icon = render_apple_icon(&logo, "mac", size).expect("render must succeed");
        assert_eq!(icon.dimensions(), (size, size));
        assert_eq!(
            icon.get_pixel(0, 0).0[3],
            0,
            "the icon-grid margin must stay transparent"
        );
        let shape_inset = round_to_u32(f64::from(size) * (1.0 - MACOS_SHAPE_RATIO) / 2.0);
        assert_eq!(
            icon.get_pixel(shape_inset + 1, shape_inset + 1).0[3],
            0,
            "the rounded-rectangle corner must be masked off"
        );
        // The logo's minus stroke crosses the exact center, so probe a
        // point in the lower half where the artwork is pure background.
        assert_eq!(
            *icon.get_pixel(size / 2, size * 3 / 4),
            image::Rgba([255, 255, 255, 255]),
            "the shape interior keeps the icon's background"
        );
    }

    #[test]
    fn android_foreground_content_fits_the_safe_zone() {
        let logo = IconSource::default_logo();
        let edge = logo.edge_color().expect("probe render must succeed");
        assert!(edge.is_some());
        let foreground =
            render_android_foreground(&logo, edge).expect("foreground render must succeed");
        let size = foreground.width();
        // Half a safe-zone diameter, plus one pixel of slack for the
        // bounding-box rounding.
        let safe_radius = f64::from(size) * ANDROID_SAFE_ZONE_RATIO / 2.0 + 1.0;
        for (x, y, pixel) in foreground.enumerate_pixels() {
            if pixel.0[3] == 0 {
                continue;
            }
            assert!(
                center_distance(x, y, size) <= safe_radius,
                "opaque pixel at ({x}, {y}) escapes the launcher-mask safe zone"
            );
        }
    }

    #[test]
    fn android_foreground_without_background_uses_visible_square() {
        let logo = IconSource::default_logo();
        let foreground =
            render_android_foreground(&logo, None).expect("foreground render must succeed");
        let size = foreground.width();
        let visible = round_to_u32(f64::from(size) * ANDROID_VISIBLE_RATIO);
        let inset = (size - visible) / 2;
        assert_eq!(
            foreground.get_pixel(inset / 2, size / 2).0[3],
            0,
            "outside the visible square the layer stays transparent"
        );
        assert_eq!(
            *foreground.get_pixel(size / 2, inset + 1),
            image::Rgba([255, 255, 255, 255]),
            "the whole artwork, background included, sits in the visible square"
        );
    }

    #[test]
    fn windows_ico_encodes_multi_size_frames() {
        let logo = IconSource::default_logo();
        let encoded = encode_windows_ico(&logo).expect("ico must encode");
        let decoded = image::load_from_memory_with_format(&encoded, image::ImageFormat::Ico)
            .expect("encoded ico must decode");
        // The image crate surfaces the largest frame; 256 is the ICO ceiling.
        assert_eq!(decoded.width(), 256);
        assert_eq!(decoded.height(), 256);
    }

    #[test]
    fn macos_icns_encodes_all_family_sizes() {
        let logo = IconSource::default_logo();
        let encoded = encode_macos_icns(&logo).expect("icns must encode");
        let family = icns::IconFamily::read(std::io::Cursor::new(&encoded))
            .expect("encoded icns must parse back");
        for &size in MACOS_ICNS_SIZES {
            assert!(
                family
                    .available_icons()
                    .iter()
                    .any(|icon| icon.pixel_width() == size),
                "icns family is missing the {size}px entry"
            );
        }
    }

    /// Exports the icon formats generated from the bundled logo so they can
    /// be reviewed by eye, mirroring the filter gallery export:
    /// `cargo nextest run -p waterui-cli -E 'test(export_app_icon_gallery_images)'`
    /// writes to `<tempdir>/waterui_app_icon_gallery/`.
    #[test]
    fn export_app_icon_gallery_images() {
        let out_dir = std::env::temp_dir().join("waterui_app_icon_gallery");
        std::fs::create_dir_all(&out_dir).expect("gallery dir must be creatable");
        let logo = IconSource::default_logo();

        let ios = render_apple_icon(&logo, "iphone", 1024).expect("ios render");
        ios.save(out_dir.join("apple-ios-1024.png"))
            .expect("save ios");

        let mac = render_apple_icon(&logo, "mac", 1024).expect("mac render");
        mac.save(out_dir.join("apple-mac-1024.png"))
            .expect("save mac");

        let edge = logo.edge_color().expect("edge probe");
        let foreground = render_android_foreground(&logo, edge).expect("android foreground");
        foreground
            .save(out_dir.join("android-foreground-432.png"))
            .expect("save android foreground");

        // What a circular-mask launcher shows: background layer, foreground
        // layer, then the mask applied to the visible 72dp region.
        let background = edge.expect("logo has a uniform background");
        let size = ANDROID_FOREGROUND_CANVAS;
        let mut launcher = image::RgbaImage::from_pixel(
            size,
            size,
            image::Rgba([background[0], background[1], background[2], 255]),
        );
        image::imageops::overlay(&mut launcher, &foreground, 0, 0);
        let visible = round_to_u32(f64::from(size) * ANDROID_VISIBLE_RATIO);
        let inset = (size - visible) / 2;
        let mut launcher =
            image::imageops::crop_imm(&launcher, inset, inset, visible, visible).to_image();
        apply_rounded_rect_mask(&mut launcher, f64::from(visible) / 2.0);
        launcher
            .save(out_dir.join("android-launcher-circle-preview.png"))
            .expect("save android launcher preview");
    }
}
