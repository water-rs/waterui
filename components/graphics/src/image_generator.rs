use crate::color::Srgb;
use crate::gpu_surface::{OffscreenRenderConfig, OffscreenSize};
use crate::multi_input_filter::FilterImage;
use crate::shader_surface::ShaderSurface;
use std::path::Path;

/// CPU-owned RGBA8 image produced by shader-based generators.
#[derive(Debug, Clone)]
pub struct GeneratedImage {
    width: u32,
    height: u32,
    rgba8: Vec<u8>,
}

impl GeneratedImage {
    /// Creates an image from RGBA8 bytes.
    ///
    /// # Panics
    ///
    /// Panics when `rgba8` does not contain exactly `width * height * 4` bytes.
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

    /// Returns the image width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }
    /// Returns the image height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
    /// Returns the raw RGBA8 bytes.
    #[must_use]
    pub fn rgba8(&self) -> &[u8] {
        &self.rgba8
    }
    /// Consumes the image and returns the raw RGBA8 bytes.
    #[must_use]
    pub fn into_rgba8(self) -> Vec<u8> {
        self.rgba8
    }
    /// Converts the generated image into a reusable filter input.
    #[must_use]
    pub fn to_filter_image(&self) -> FilterImage {
        FilterImage::from_rgba8(self.width, self.height, self.rgba8.clone())
    }

    /// Saves the image as a PNG file.
    ///
    /// # Errors
    ///
    /// Returns the image encoder's filesystem or encoding error.
    ///
    /// # Panics
    ///
    /// Panics when the cached RGBA buffer shape no longer matches the stored dimensions.
    pub fn save_png(&self, path: impl AsRef<Path>) -> image::ImageResult<()> {
        let image = image::RgbaImage::from_raw(self.width, self.height, self.rgba8.clone())
            .expect("GeneratedImage::save_png: rgba buffer shape must match dimensions");
        image.save(path)
    }
}

/// Trait implemented by procedural image generators.
pub trait ImageGenerator {
    /// Produces a fresh image from the generator's current parameters.
    fn generate(&mut self) -> GeneratedImage;
}

fn srgb_rgb_literal(color: Srgb) -> [String; 3] {
    [
        color.red.to_string(),
        color.green.to_string(),
        color.blue.to_string(),
    ]
}

fn render_fragment(width: u32, height: u32, fragment: String) -> GeneratedImage {
    let size = OffscreenSize::try_from_pixels(width, height)
        .expect("ImageGenerator::render_fragment: dimensions must be non-zero");
    let config = OffscreenRenderConfig::new(size).format(wgpu::TextureFormat::Rgba8Unorm);
    let mut env = waterui_core::Environment::new();
    let output = ShaderSurface::new(fragment)
        .into_inner()
        .render_offscreen(config, &mut env)
        .expect("ImageGenerator::render_fragment: GPU offscreen render should succeed");
    GeneratedImage::from_rgba8(output.width, output.height, output.rgba8)
}

/// Generates a checkerboard texture.
#[derive(Debug, Clone)]
pub struct CheckerboardGenerator {
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Checker cell size in pixels.
    pub cell_size: u32,
    /// Light cell color.
    pub light: Srgb,
    /// Dark cell color.
    pub dark: Srgb,
    /// Horizontal pattern offset in pixels.
    pub offset_x: u32,
    /// Vertical pattern offset in pixels.
    pub offset_y: u32,
}

impl ImageGenerator for CheckerboardGenerator {
    fn generate(&mut self) -> GeneratedImage {
        assert!(
            self.cell_size > 0,
            "CheckerboardGenerator: cell_size must be > 0"
        );
        let [lr, lg, lb] = srgb_rgb_literal(self.light);
        let [dr, dg, db] = srgb_rgb_literal(self.dark);
        let fragment = include_str!("shaders/generators/checkerboard.wgsl")
            .replace("__LIGHT_R__", &lr)
            .replace("__LIGHT_G__", &lg)
            .replace("__LIGHT_B__", &lb)
            .replace("__DARK_R__", &dr)
            .replace("__DARK_G__", &dg)
            .replace("__DARK_B__", &db)
            .replace("__CELL_W__", &self.cell_size.to_string())
            .replace("__CELL_H__", &self.cell_size.to_string())
            .replace("__OFFSET_X__", &self.offset_x.to_string())
            .replace("__OFFSET_Y__", &self.offset_y.to_string());
        render_fragment(self.width, self.height, fragment)
    }
}

/// Generates a striped texture.
#[derive(Debug, Clone)]
pub struct StripeGenerator {
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Stripe width in pixels.
    pub stripe_width: u32,
    /// Whether stripes run horizontally.
    pub horizontal: bool,
    /// Primary stripe color.
    pub primary: Srgb,
    /// Secondary stripe color.
    pub secondary: Srgb,
    /// Pattern offset in pixels.
    pub offset: u32,
}

impl ImageGenerator for StripeGenerator {
    fn generate(&mut self) -> GeneratedImage {
        assert!(
            self.stripe_width > 0,
            "StripeGenerator: stripe_width must be > 0"
        );
        let [pr, pg, pb] = srgb_rgb_literal(self.primary);
        let [sr, sg, sb] = srgb_rgb_literal(self.secondary);
        let fragment = include_str!("shaders/generators/stripes.wgsl")
            .replace("__PRIMARY_R__", &pr)
            .replace("__PRIMARY_G__", &pg)
            .replace("__PRIMARY_B__", &pb)
            .replace("__SECONDARY_R__", &sr)
            .replace("__SECONDARY_G__", &sg)
            .replace("__SECONDARY_B__", &sb)
            .replace("__STRIPE_WIDTH__", &self.stripe_width.to_string())
            .replace(
                "__HORIZONTAL__",
                if self.horizontal { "true" } else { "false" },
            )
            .replace("__OFFSET__", &self.offset.to_string());
        render_fragment(self.width, self.height, fragment)
    }
}

/// Generates a dotted grid texture.
#[derive(Debug, Clone)]
pub struct DotGridGenerator {
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Grid spacing in pixels.
    pub spacing: u32,
    /// Dot radius in pixels.
    pub radius: f32,
    /// Dot color.
    pub foreground: Srgb,
    /// Background color.
    pub background: Srgb,
}

impl ImageGenerator for DotGridGenerator {
    fn generate(&mut self) -> GeneratedImage {
        assert!(self.spacing > 0, "DotGridGenerator: spacing must be > 0");
        let [fr, fg, fb] = srgb_rgb_literal(self.foreground);
        let [br, bg, bb] = srgb_rgb_literal(self.background);
        let fragment = include_str!("shaders/generators/dot_grid.wgsl")
            .replace("__FG_R__", &fr)
            .replace("__FG_G__", &fg)
            .replace("__FG_B__", &fb)
            .replace("__BG_R__", &br)
            .replace("__BG_G__", &bg)
            .replace("__BG_B__", &bb)
            .replace("__SPACING__", &self.spacing.to_string())
            .replace("__RADIUS__", &self.radius.to_string());
        render_fragment(self.width, self.height, fragment)
    }
}

/// Generates deterministic noise from a seed.
#[derive(Debug, Clone)]
pub struct NoiseGenerator {
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Seed value passed to the shader.
    pub seed: u64,
}

impl ImageGenerator for NoiseGenerator {
    fn generate(&mut self) -> GeneratedImage {
        let fragment = include_str!("shaders/generators/noise.wgsl")
            .replace("__SEED__", &self.seed.to_string());
        render_fragment(self.width, self.height, fragment)
    }
}

/// Generates a linear gradient texture.
#[derive(Debug, Clone)]
pub struct LinearGradientGenerator {
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Start color.
    pub start_color: Srgb,
    /// End color.
    pub end_color: Srgb,
    /// Normalized start point.
    pub start_point: [f32; 2],
    /// Normalized end point.
    pub end_point: [f32; 2],
}

impl ImageGenerator for LinearGradientGenerator {
    fn generate(&mut self) -> GeneratedImage {
        let [sr, sg, sb] = srgb_rgb_literal(self.start_color);
        let [er, eg, eb] = srgb_rgb_literal(self.end_color);
        let fragment = include_str!("shaders/generators/linear_gradient.wgsl")
            .replace("__START_R__", &sr)
            .replace("__START_G__", &sg)
            .replace("__START_B__", &sb)
            .replace("__END_R__", &er)
            .replace("__END_G__", &eg)
            .replace("__END_B__", &eb)
            .replace("__START_X__", &self.start_point[0].to_string())
            .replace("__START_Y__", &self.start_point[1].to_string())
            .replace("__END_X__", &self.end_point[0].to_string())
            .replace("__END_Y__", &self.end_point[1].to_string());
        render_fragment(self.width, self.height, fragment)
    }
}

/// Generates a radial gradient texture.
#[derive(Debug, Clone)]
pub struct RadialGradientGenerator {
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Color at the center.
    pub inner_color: Srgb,
    /// Color at the outer radius.
    pub outer_color: Srgb,
    /// Normalized gradient center.
    pub center: [f32; 2],
    /// Gradient radius in normalized units.
    pub radius: f32,
}

impl ImageGenerator for RadialGradientGenerator {
    fn generate(&mut self) -> GeneratedImage {
        let [ir, ig, ib] = srgb_rgb_literal(self.inner_color);
        let [or, og, ob] = srgb_rgb_literal(self.outer_color);
        let fragment = include_str!("shaders/generators/radial_gradient.wgsl")
            .replace("__INNER_R__", &ir)
            .replace("__INNER_G__", &ig)
            .replace("__INNER_B__", &ib)
            .replace("__OUTER_R__", &or)
            .replace("__OUTER_G__", &og)
            .replace("__OUTER_B__", &ob)
            .replace("__CENTER_X__", &self.center[0].to_string())
            .replace("__CENTER_Y__", &self.center[1].to_string())
            .replace("__RADIUS__", &self.radius.to_string());
        render_fragment(self.width, self.height, fragment)
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
