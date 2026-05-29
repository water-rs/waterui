use crate::color::Srgb;
use crate::image_generator::GeneratedImage;
use num_traits::ToPrimitive;

/// Per-channel histogram data for an RGBA8 image.
#[derive(Debug, Clone)]
pub struct Histogram {
    /// Histogram counts for the red channel.
    pub red: [u32; 256],
    /// Histogram counts for the green channel.
    pub green: [u32; 256],
    /// Histogram counts for the blue channel.
    pub blue: [u32; 256],
    /// Histogram counts for the alpha channel.
    pub alpha: [u32; 256],
}

/// Minimum and maximum luma found in an image.
#[derive(Debug, Clone, Copy)]
pub struct MinMaxLuma {
    /// Minimum luma value.
    pub min: f32,
    /// Maximum luma value.
    pub max: f32,
}

/// Dominant quantized color found in an image.
#[derive(Debug, Clone, Copy)]
pub struct DominantColor {
    /// Representative color.
    pub color: Srgb,
    /// Number of pixels in the winning bucket.
    pub count: u32,
}

/// Analysis helpers for generated images.
#[derive(Debug)]
pub struct ImageAnalysis<'a> {
    image: &'a GeneratedImage,
}

impl<'a> ImageAnalysis<'a> {
    /// Creates an analyzer for a generated image.
    #[must_use]
    pub const fn new(image: &'a GeneratedImage) -> Self {
        Self { image }
    }

    /// Computes a per-channel histogram.
    #[must_use]
    pub fn histogram(&self) -> Histogram {
        let mut histogram = Histogram {
            red: [0; 256],
            green: [0; 256],
            blue: [0; 256],
            alpha: [0; 256],
        };
        for px in self.image.rgba8().chunks_exact(4) {
            histogram.red[px[0] as usize] += 1;
            histogram.green[px[1] as usize] += 1;
            histogram.blue[px[2] as usize] += 1;
            histogram.alpha[px[3] as usize] += 1;
        }
        histogram
    }

    /// Computes the area-average color.
    #[must_use]
    pub fn area_average(&self) -> Srgb {
        let mut red = 0u64;
        let mut green = 0u64;
        let mut blue = 0u64;
        let pixel_count = u64::from((self.image.width() * self.image.height()).max(1));
        for px in self.image.rgba8().chunks_exact(4) {
            red += u64::from(px[0]);
            green += u64::from(px[1]);
            blue += u64::from(px[2]);
        }
        let denominator = u64_to_f32(pixel_count) * 255.0;
        Srgb::new(
            u64_to_f32(red) / denominator,
            u64_to_f32(green) / denominator,
            u64_to_f32(blue) / denominator,
        )
    }

    /// Computes the minimum and maximum luma values.
    #[must_use]
    pub fn min_max_luma(&self) -> MinMaxLuma {
        let mut min = 1.0f32;
        let mut max = 0.0f32;
        for px in self.image.rgba8().chunks_exact(4) {
            let luma = 0.0722f32.mul_add(
                f32::from(px[2]),
                0.2126f32.mul_add(f32::from(px[0]), 0.7152 * f32::from(px[1])),
            ) / 255.0;
            min = min.min(luma);
            max = max.max(luma);
        }
        MinMaxLuma { min, max }
    }

    /// Finds the dominant quantized color bucket.
    ///
    /// # Panics
    ///
    /// Panics when the image contains no pixels.
    #[must_use]
    pub fn dominant_color(&self) -> DominantColor {
        let mut buckets = std::collections::BTreeMap::<u16, u32>::new();
        for px in self.image.rgba8().chunks_exact(4) {
            let key = (u16::from(px[0] >> 5) << 10)
                | (u16::from(px[1] >> 5) << 5)
                | u16::from(px[2] >> 5);
            *buckets.entry(key).or_insert(0) += 1;
        }
        let (&key, &count) = buckets
            .iter()
            .max_by_key(|(_, count)| **count)
            .expect("ImageAnalysis::dominant_color: image must contain at least one pixel");
        let red = f32::from((key >> 10) & 0x1F) / 31.0;
        let green = f32::from((key >> 5) & 0x1F) / 31.0;
        let blue = f32::from(key & 0x1F) / 31.0;
        DominantColor {
            color: Srgb::new(red, green, blue),
            count,
        }
    }
}

fn u64_to_f32(value: u64) -> f32 {
    value
        .to_f32()
        .expect("image_analysis: u64 value must be representable as f32")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_generator::{CheckerboardGenerator, ImageGenerator};

    #[test]
    fn analysis_reports_expected_luma_range() {
        let image = CheckerboardGenerator {
            width: 8,
            height: 8,
            cell_size: 2,
            light: Srgb::WHITE,
            dark: Srgb::BLACK,
            offset_x: 0,
            offset_y: 0,
        }
        .generate();
        let analysis = ImageAnalysis::new(&image);
        let range = analysis.min_max_luma();
        assert!(range.min.abs() < f32::EPSILON);
        assert!((range.max - 1.0).abs() < f32::EPSILON);
    }
}
