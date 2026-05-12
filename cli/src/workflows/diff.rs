//! Image comparison and diff generation for visual testing.
//!
//! Provides pixel-level comparison of before/after screenshots to detect
//! UI changes after gestures.

use std::path::Path;

use color_eyre::eyre::{self, eyre};
use image::{ImageBuffer, Rgba, RgbaImage};

/// Bounding box of changed region.
#[derive(Debug, Clone, Copy)]
pub struct BoundingBox {
    /// Minimum X coordinate.
    pub min_x: u32,
    /// Minimum Y coordinate.
    pub min_y: u32,
    /// Maximum X coordinate.
    pub max_x: u32,
    /// Maximum Y coordinate.
    pub max_y: u32,
}

impl BoundingBox {
    /// Width of the bounding box.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.max_x - self.min_x + 1
    }

    /// Height of the bounding box.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.max_y - self.min_y + 1
    }
}

/// Result of comparing two images.
#[derive(Debug)]
pub struct DiffResult {
    /// Whether any pixels changed.
    pub changed: bool,
    /// Bounding box of the changed region, if any.
    pub bbox: Option<BoundingBox>,
    /// Percentage of pixels that changed (0.0 to 100.0).
    pub percentage: f32,
    /// Total number of changed pixels.
    pub changed_pixels: u32,
    /// Image dimensions (width, height).
    pub dimensions: (u32, u32),
}

impl std::fmt::Display for DiffResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.changed {
            writeln!(f, "Changed: yes")?;
            if let Some(bbox) = &self.bbox {
                writeln!(
                    f,
                    "Bounding box: ({}, {}) to ({}, {})",
                    bbox.min_x, bbox.min_y, bbox.max_x, bbox.max_y
                )?;
                writeln!(f, "Dimensions: {}x{} pixels", bbox.width(), bbox.height())?;
            }
            writeln!(f, "Screen percentage: {:.1}%", self.percentage)?;
        } else {
            writeln!(f, "Changed: no")?;
        }
        Ok(())
    }
}

/// Tolerance for pixel comparison to handle compression artifacts.
const PIXEL_TOLERANCE: u8 = 10;

/// Check if two pixels are different (with tolerance).
fn pixels_differ(a: Rgba<u8>, b: Rgba<u8>) -> bool {
    a.0.iter()
        .zip(b.0.iter())
        .any(|(a_val, b_val)| a_val.abs_diff(*b_val) > PIXEL_TOLERANCE)
}

/// Compare two images and return the diff result.
///
/// # Arguments
///
/// * `before` - PNG bytes of the before image
/// * `after` - PNG bytes of the after image
///
/// # Errors
///
/// Returns an error if the images cannot be loaded or have different dimensions.
pub fn compare_images(before: &[u8], after: &[u8]) -> eyre::Result<DiffResult> {
    let before_img = image::load_from_memory(before)
        .map_err(|e| eyre!("Failed to load before image: {e}"))?
        .to_rgba8();

    let after_img = image::load_from_memory(after)
        .map_err(|e| eyre!("Failed to load after image: {e}"))?
        .to_rgba8();

    let (width, height) = before_img.dimensions();
    let (after_width, after_height) = after_img.dimensions();

    if width != after_width || height != after_height {
        eyre::bail!(
            "Image dimensions don't match: before={}x{}, after={}x{}",
            width,
            height,
            after_width,
            after_height
        );
    }

    let total_pixels = width * height;
    let mut changed_pixels: u32 = 0;
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x: u32 = 0;
    let mut max_y: u32 = 0;

    for y in 0..height {
        for x in 0..width {
            let before_pixel = before_img.get_pixel(x, y);
            let after_pixel = after_img.get_pixel(x, y);

            if pixels_differ(*before_pixel, *after_pixel) {
                changed_pixels += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    let changed = changed_pixels > 0;
    #[allow(clippy::cast_possible_truncation)]
    let percentage = (f64::from(changed_pixels) / f64::from(total_pixels) * 100.0) as f32;

    let bbox = if changed {
        Some(BoundingBox {
            min_x,
            min_y,
            max_x,
            max_y,
        })
    } else {
        None
    };

    Ok(DiffResult {
        changed,
        bbox,
        percentage,
        changed_pixels,
        dimensions: (width, height),
    })
}

/// Save a diff image highlighting changed pixels.
///
/// Changed pixels are overlaid with semi-transparent red.
///
/// # Arguments
///
/// * `before` - PNG bytes of the before image
/// * `after` - PNG bytes of the after image
/// * `output` - Path to save the diff image
///
/// # Errors
///
/// Returns an error if the images cannot be loaded or the output cannot be written.
pub fn save_diff_image(before: &[u8], after: &[u8], output: &Path) -> eyre::Result<()> {
    let before_img = image::load_from_memory(before)
        .map_err(|e| eyre!("Failed to load before image: {e}"))?
        .to_rgba8();

    let after_img = image::load_from_memory(after)
        .map_err(|e| eyre!("Failed to load after image: {e}"))?
        .to_rgba8();

    let (width, height) = after_img.dimensions();
    let (before_width, before_height) = before_img.dimensions();

    if width != before_width || height != before_height {
        eyre::bail!(
            "Image dimensions don't match: before={}x{}, after={}x{}",
            before_width,
            before_height,
            width,
            height
        );
    }

    let mut diff_img: RgbaImage = ImageBuffer::new(width, height);

    // Semi-transparent red overlay for changed pixels
    let overlay_r: u8 = 255;
    let overlay_g: u8 = 0;
    let overlay_b: u8 = 0;

    for y in 0..height {
        for x in 0..width {
            let before_pixel = before_img.get_pixel(x, y);
            let after_pixel = after_img.get_pixel(x, y);

            if pixels_differ(*before_pixel, *after_pixel) {
                // Blend the after pixel with red overlay
                let after = after_pixel.0;
                #[allow(clippy::cast_possible_truncation)]
                let blended = Rgba([
                    u16::midpoint(u16::from(after[0]), u16::from(overlay_r)) as u8,
                    u16::midpoint(u16::from(after[1]), u16::from(overlay_g)) as u8,
                    u16::midpoint(u16::from(after[2]), u16::from(overlay_b)) as u8,
                    255,
                ]);
                diff_img.put_pixel(x, y, blended);
            } else {
                diff_img.put_pixel(x, y, *after_pixel);
            }
        }
    }

    diff_img
        .save(output)
        .map_err(|e| eyre!("Failed to save diff image: {e}"))?;

    Ok(())
}
