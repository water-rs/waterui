use image::RgbaImage;

use crate::color::Srgb;
use crate::multi_input_filter::FilterImage;

#[derive(Debug, Clone)]
pub struct GeneratedImage {
    width: u32,
    height: u32,
    rgba8: Vec<u8>,
}

impl GeneratedImage {
    #[must_use]
    pub fn from_rgba8(width: u32, height: u32, rgba8: Vec<u8>) -> Self {
        let expected_len = width as usize * height as usize * 4;
        assert_eq!(
            rgba8.len(),
            expected_len,
            "GeneratedImage::from_rgba8: expected {expected_len} bytes for {width}x{height}, got {}",
            rgba8.len()
        );
        Self {
            width,
            height,
            rgba8,
        }
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn rgba8(&self) -> &[u8] {
        &self.rgba8
    }

    #[must_use]
    pub fn into_rgba8(self) -> Vec<u8> {
        self.rgba8
    }

    #[must_use]
    pub fn to_filter_image(&self) -> FilterImage {
        FilterImage::from_rgba8(self.width, self.height, self.rgba8.clone())
    }

    pub fn save_png(&self, path: impl AsRef<std::path::Path>) -> image::ImageResult<()> {
        let image = RgbaImage::from_raw(self.width, self.height, self.rgba8.clone())
            .expect("GeneratedImage::save_png: rgba buffer shape must match dimensions");
        image.save(path)
    }
}

pub trait ImageGenerator {
    fn generate(&mut self) -> GeneratedImage;
}

fn srgb_to_rgba8(color: Srgb) -> [u8; 4] {
    [
        (color.red * 255.0).round() as u8,
        (color.green * 255.0).round() as u8,
        (color.blue * 255.0).round() as u8,
        255,
    ]
}

#[derive(Debug, Clone)]
pub struct CheckerboardGenerator {
    pub width: u32,
    pub height: u32,
    pub cell_size: u32,
    pub light: Srgb,
    pub dark: Srgb,
    pub offset_x: u32,
    pub offset_y: u32,
}

impl ImageGenerator for CheckerboardGenerator {
    fn generate(&mut self) -> GeneratedImage {
        assert!(self.cell_size > 0, "CheckerboardGenerator: cell_size must be > 0");
        let light = srgb_to_rgba8(self.light);
        let dark = srgb_to_rgba8(self.dark);
        let mut rgba8 = vec![0u8; self.width as usize * self.height as usize * 4];
        for y in 0..self.height {
            for x in 0..self.width {
                let cell_x = (x + self.offset_x) / self.cell_size;
                let cell_y = (y + self.offset_y) / self.cell_size;
                let color = if (cell_x + cell_y) % 2 == 0 { light } else { dark };
                let index = ((y * self.width + x) * 4) as usize;
                rgba8[index..index + 4].copy_from_slice(&color);
            }
        }
        GeneratedImage::from_rgba8(self.width, self.height, rgba8)
    }
}

#[derive(Debug, Clone)]
pub struct StripeGenerator {
    pub width: u32,
    pub height: u32,
    pub stripe_width: u32,
    pub horizontal: bool,
    pub primary: Srgb,
    pub secondary: Srgb,
    pub offset: u32,
}

impl ImageGenerator for StripeGenerator {
    fn generate(&mut self) -> GeneratedImage {
        assert!(self.stripe_width > 0, "StripeGenerator: stripe_width must be > 0");
        let primary = srgb_to_rgba8(self.primary);
        let secondary = srgb_to_rgba8(self.secondary);
        let mut rgba8 = vec![0u8; self.width as usize * self.height as usize * 4];
        for y in 0..self.height {
            for x in 0..self.width {
                let stripe = if self.horizontal {
                    (y + self.offset) / self.stripe_width
                } else {
                    (x + self.offset) / self.stripe_width
                };
                let color = if stripe % 2 == 0 { primary } else { secondary };
                let index = ((y * self.width + x) * 4) as usize;
                rgba8[index..index + 4].copy_from_slice(&color);
            }
        }
        GeneratedImage::from_rgba8(self.width, self.height, rgba8)
    }
}

#[derive(Debug, Clone)]
pub struct DotGridGenerator {
    pub width: u32,
    pub height: u32,
    pub spacing: u32,
    pub radius: f32,
    pub foreground: Srgb,
    pub background: Srgb,
}

impl ImageGenerator for DotGridGenerator {
    fn generate(&mut self) -> GeneratedImage {
        assert!(self.spacing > 0, "DotGridGenerator: spacing must be > 0");
        let foreground = srgb_to_rgba8(self.foreground);
        let background = srgb_to_rgba8(self.background);
        let mut rgba8 = vec![0u8; self.width as usize * self.height as usize * 4];
        let spacing = self.spacing as f32;
        for y in 0..self.height {
            for x in 0..self.width {
                let local_x = ((x as f32 + spacing * 0.5) % spacing) - spacing * 0.5;
                let local_y = ((y as f32 + spacing * 0.5) % spacing) - spacing * 0.5;
                let color = if (local_x * local_x + local_y * local_y).sqrt() <= self.radius {
                    foreground
                } else {
                    background
                };
                let index = ((y * self.width + x) * 4) as usize;
                rgba8[index..index + 4].copy_from_slice(&color);
            }
        }
        GeneratedImage::from_rgba8(self.width, self.height, rgba8)
    }
}

#[derive(Debug, Clone)]
pub struct NoiseGenerator {
    pub width: u32,
    pub height: u32,
    pub seed: u64,
}

impl ImageGenerator for NoiseGenerator {
    fn generate(&mut self) -> GeneratedImage {
        let mut rgba8 = vec![0u8; self.width as usize * self.height as usize * 4];
        for y in 0..self.height {
            for x in 0..self.width {
                let mut value = u64::from(x).wrapping_mul(0x9E3779B185EBCA87)
                    ^ u64::from(y).wrapping_mul(0xC2B2AE3D27D4EB4F)
                    ^ self.seed;
                value ^= value >> 33;
                value = value.wrapping_mul(0xff51afd7ed558ccd);
                value ^= value >> 33;
                let sample = (value & 0xFF) as u8;
                let index = ((y * self.width + x) * 4) as usize;
                rgba8[index..index + 4].copy_from_slice(&[sample, sample, sample, 255]);
            }
        }
        GeneratedImage::from_rgba8(self.width, self.height, rgba8)
    }
}

#[derive(Debug, Clone)]
pub struct LinearGradientGenerator {
    pub width: u32,
    pub height: u32,
    pub start_color: Srgb,
    pub end_color: Srgb,
    pub start_point: [f32; 2],
    pub end_point: [f32; 2],
}

impl ImageGenerator for LinearGradientGenerator {
    fn generate(&mut self) -> GeneratedImage {
        let mut rgba8 = vec![0u8; self.width as usize * self.height as usize * 4];
        let start = self.start_point;
        let end = self.end_point;
        let direction = [end[0] - start[0], end[1] - start[1]];
        let len_sq = (direction[0] * direction[0] + direction[1] * direction[1]).max(0.0001);
        for y in 0..self.height {
            for x in 0..self.width {
                let uv = [x as f32 / self.width as f32, y as f32 / self.height as f32];
                let rel = [uv[0] - start[0], uv[1] - start[1]];
                let t = ((rel[0] * direction[0] + rel[1] * direction[1]) / len_sq).clamp(0.0, 1.0);
                let color = [
                    self.start_color.red + (self.end_color.red - self.start_color.red) * t,
                    self.start_color.green + (self.end_color.green - self.start_color.green) * t,
                    self.start_color.blue + (self.end_color.blue - self.start_color.blue) * t,
                ];
                let index = ((y * self.width + x) * 4) as usize;
                rgba8[index..index + 4].copy_from_slice(&[
                    (color[0] * 255.0).round() as u8,
                    (color[1] * 255.0).round() as u8,
                    (color[2] * 255.0).round() as u8,
                    255,
                ]);
            }
        }
        GeneratedImage::from_rgba8(self.width, self.height, rgba8)
    }
}

#[derive(Debug, Clone)]
pub struct RadialGradientGenerator {
    pub width: u32,
    pub height: u32,
    pub inner_color: Srgb,
    pub outer_color: Srgb,
    pub center: [f32; 2],
    pub radius: f32,
}

impl ImageGenerator for RadialGradientGenerator {
    fn generate(&mut self) -> GeneratedImage {
        let mut rgba8 = vec![0u8; self.width as usize * self.height as usize * 4];
        let radius = self.radius.max(0.0001);
        for y in 0..self.height {
            for x in 0..self.width {
                let uv = [x as f32 / self.width as f32, y as f32 / self.height as f32];
                let dx = uv[0] - self.center[0];
                let dy = uv[1] - self.center[1];
                let t = ((dx * dx + dy * dy).sqrt() / radius).clamp(0.0, 1.0);
                let color = [
                    self.inner_color.red + (self.outer_color.red - self.inner_color.red) * t,
                    self.inner_color.green + (self.outer_color.green - self.inner_color.green) * t,
                    self.inner_color.blue + (self.outer_color.blue - self.inner_color.blue) * t,
                ];
                let index = ((y * self.width + x) * 4) as usize;
                rgba8[index..index + 4].copy_from_slice(&[
                    (color[0] * 255.0).round() as u8,
                    (color[1] * 255.0).round() as u8,
                    (color[2] * 255.0).round() as u8,
                    255,
                ]);
            }
        }
        GeneratedImage::from_rgba8(self.width, self.height, rgba8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkerboard_generator_produces_expected_dimensions() {
        let mut generator = CheckerboardGenerator {
            width: 16,
            height: 12,
            cell_size: 4,
            light: Srgb::WHITE,
            dark: Srgb::BLACK,
            offset_x: 0,
            offset_y: 0,
        };
        let image = generator.generate();
        assert_eq!(image.width(), 16);
        assert_eq!(image.height(), 12);
        assert_eq!(image.rgba8().len(), 16 * 12 * 4);
    }

    #[test]
    fn noise_generator_is_not_flat() {
        let mut generator = NoiseGenerator {
            width: 16,
            height: 16,
            seed: 42,
        };
        let image = generator.generate();
        let first = &image.rgba8()[..4];
        assert!(image.rgba8().chunks_exact(4).any(|px| px != first));
    }
}
