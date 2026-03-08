use crate::color::Srgb;
use crate::image_generator::GeneratedImage;

#[derive(Debug, Clone)]
pub struct Histogram {
    pub red: [u32; 256],
    pub green: [u32; 256],
    pub blue: [u32; 256],
    pub alpha: [u32; 256],
}

#[derive(Debug, Clone, Copy)]
pub struct MinMaxLuma {
    pub min: f32,
    pub max: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct DominantColor {
    pub color: Srgb,
    pub count: u32,
}

#[derive(Debug)]
pub struct ImageAnalysis<'a> {
    image: &'a GeneratedImage,
}

impl<'a> ImageAnalysis<'a> {
    #[must_use]
    pub const fn new(image: &'a GeneratedImage) -> Self {
        Self { image }
    }

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

    #[must_use]
    pub fn area_average(&self) -> Srgb {
        let mut red = 0u64;
        let mut green = 0u64;
        let mut blue = 0u64;
        let pixel_count = (self.image.width() * self.image.height()).max(1) as u64;
        for px in self.image.rgba8().chunks_exact(4) {
            red += u64::from(px[0]);
            green += u64::from(px[1]);
            blue += u64::from(px[2]);
        }
        Srgb::new(
            red as f32 / (pixel_count as f32 * 255.0),
            green as f32 / (pixel_count as f32 * 255.0),
            blue as f32 / (pixel_count as f32 * 255.0),
        )
    }

    #[must_use]
    pub fn min_max_luma(&self) -> MinMaxLuma {
        let mut min = 1.0f32;
        let mut max = 0.0f32;
        for px in self.image.rgba8().chunks_exact(4) {
            let luma = (0.2126 * f32::from(px[0])
                + 0.7152 * f32::from(px[1])
                + 0.0722 * f32::from(px[2]))
                / 255.0;
            min = min.min(luma);
            max = max.max(luma);
        }
        MinMaxLuma { min, max }
    }

    #[must_use]
    pub fn dominant_color(&self) -> DominantColor {
        let mut buckets = std::collections::BTreeMap::<u16, u32>::new();
        for px in self.image.rgba8().chunks_exact(4) {
            let key = (((px[0] >> 5) as u16) << 10) | (((px[1] >> 5) as u16) << 5) | (px[2] >> 5) as u16;
            *buckets.entry(key).or_insert(0) += 1;
        }
        let (&key, &count) = buckets
            .iter()
            .max_by_key(|(_, count)| **count)
            .expect("ImageAnalysis::dominant_color: image must contain at least one pixel");
        let red = ((key >> 10) & 0x1F) as f32 / 31.0;
        let green = ((key >> 5) & 0x1F) as f32 / 31.0;
        let blue = (key & 0x1F) as f32 / 31.0;
        DominantColor {
            color: Srgb::new(red, green, blue),
            count,
        }
    }
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
        assert_eq!(range.min, 0.0);
        assert_eq!(range.max, 1.0);
    }
}
