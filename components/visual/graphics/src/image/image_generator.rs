use crate::GpuRuntime;
use crate::color::Srgb;
use crate::gpu_surface::{
    GpuContext, GpuFrame, GpuSurface, GpuView, OffscreenRenderConfig, OffscreenSize,
};
use crate::multi_input_filter::FilterImage;
use crate::shaders::IMAGE_GENERATOR;
use bytemuck::{Pod, Zeroable};
use core::future::Future;
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
    pub fn save_png(&self, path: impl AsRef<Path>) -> image::ImageResult<()> {
        image::save_buffer(
            path,
            &self.rgba8,
            self.width,
            self.height,
            image::ColorType::Rgba8,
        )
    }
}

/// Trait implemented by procedural image generators.
pub trait ImageGenerator {
    /// Produces a fresh image from the generator's current parameters.
    fn generate(&self, runtime: &GpuRuntime) -> impl Future<Output = GeneratedImage>;
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
enum GeneratorKind {
    Checkerboard,
    Stripes,
    DotGrid,
    Noise,
    LinearGradient,
    RadialGradient,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct GeneratorUniforms {
    resolution: [f32; 2],
    kind: u32,
    flag: u32,
    color_a: [f32; 4],
    color_b: [f32; 4],
    params_a: [f32; 4],
    uint_params: [u32; 4],
}

impl GeneratorUniforms {
    const fn new(kind: GeneratorKind) -> Self {
        Self {
            resolution: [0.0; 2],
            kind: kind as u32,
            flag: 0,
            color_a: [0.0; 4],
            color_b: [0.0; 4],
            params_a: [0.0; 4],
            uint_params: [0; 4],
        }
    }

    const fn colors(mut self, color_a: Srgb, color_b: Srgb) -> Self {
        self.color_a = [color_a.red, color_a.green, color_a.blue, 1.0];
        self.color_b = [color_b.red, color_b.green, color_b.blue, 1.0];
        self
    }
}

struct GeneratorResources {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

struct GeneratorRenderer {
    uniforms: GeneratorUniforms,
    resources: Option<GeneratorResources>,
}

impl GeneratorRenderer {
    const fn new(uniforms: GeneratorUniforms) -> Self {
        Self {
            uniforms,
            resources: None,
        }
    }
}

impl GpuView for GeneratorRenderer {
    #[expect(
        clippy::future_not_send,
        reason = "GpuView setup is confined to the render thread and borrows its device context"
    )]
    async fn setup(&mut self, ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
        let (vertex_shader, fragment_shader) =
            IMAGE_GENERATOR.create_render_stages(ctx.device, "vs_main", "fs_main");
        let mut bind_group_layouts = IMAGE_GENERATOR.create_bind_group_layouts(ctx.device);
        assert_eq!(
            bind_group_layouts.len(),
            1,
            "image generator shader must use exactly one bind group"
        );
        let bind_group_layout = bind_group_layouts
            .pop()
            .expect("one image generator bind group was asserted");
        let uniform_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Image Generator Uniforms"),
            size: size_of::<GeneratorUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Image Generator Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Image Generator Pipeline Layout"),
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });
        let pipeline_scope = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Image Generator Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: vertex_shader.module(),
                    entry_point: Some(vertex_shader.entry_point()),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: fragment_shader.module(),
                    entry_point: Some(fragment_shader.entry_point()),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.surface_format,
                        blend: (!ctx.is_hdr()).then_some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
        let pipeline_error = pipeline_scope.pop().await;
        assert!(
            pipeline_error.is_none(),
            "image generator pipeline creation failed: {pipeline_error:?}"
        );
        self.resources = Some(GeneratorResources {
            format: ctx.surface_format,
            pipeline,
            uniform_buffer,
            bind_group,
        });
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        let resources = self
            .resources
            .as_ref()
            .expect("GeneratorRenderer::setup() must run before render()");
        assert_eq!(
            resources.format, frame.format,
            "image generator format mismatch"
        );
        #[expect(
            clippy::cast_precision_loss,
            reason = "GPU viewport dimensions are represented as f32 shader uniforms"
        )]
        {
            self.uniforms.resolution = [frame.width as f32, frame.height as f32];
        }
        frame.queue.write_buffer(
            &resources.uniform_buffer,
            0,
            bytemuck::bytes_of(&self.uniforms),
        );
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Image Generator Encoder"),
            });
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Image Generator Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            render_pass.set_pipeline(&resources.pipeline);
            render_pass.set_bind_group(0, &resources.bind_group, &[]);
            render_pass.draw(0..6, 0..1);
        }
        frame.queue.submit(core::iter::once(encoder.finish()));
    }
}

#[expect(
    clippy::future_not_send,
    reason = "procedural image rendering uses the UI-local offscreen GpuView environment"
)]
async fn render_fragment(
    runtime: &GpuRuntime,
    width: u32,
    height: u32,
    uniforms: GeneratorUniforms,
) -> GeneratedImage {
    let size = OffscreenSize::try_from_pixels(width, height)
        .expect("ImageGenerator::render_fragment: dimensions must be non-zero");
    let config = OffscreenRenderConfig::new(size).format(wgpu::TextureFormat::Rgba8Unorm);
    let mut env = waterui_core::Environment::new();
    let output = GpuSurface::new(GeneratorRenderer::new(uniforms))
        .render_offscreen(runtime, config, &mut env)
        .await
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
    #[expect(
        clippy::future_not_send,
        reason = "image generators await the UI-local offscreen GpuView renderer"
    )]
    async fn generate(&self, runtime: &GpuRuntime) -> GeneratedImage {
        assert!(
            self.cell_size > 0,
            "CheckerboardGenerator: cell_size must be > 0"
        );
        let mut uniforms =
            GeneratorUniforms::new(GeneratorKind::Checkerboard).colors(self.light, self.dark);
        uniforms.uint_params = [self.cell_size, self.cell_size, self.offset_x, self.offset_y];
        render_fragment(runtime, self.width, self.height, uniforms).await
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
    #[expect(
        clippy::future_not_send,
        reason = "image generators await the UI-local offscreen GpuView renderer"
    )]
    async fn generate(&self, runtime: &GpuRuntime) -> GeneratedImage {
        assert!(
            self.stripe_width > 0,
            "StripeGenerator: stripe_width must be > 0"
        );
        let mut uniforms =
            GeneratorUniforms::new(GeneratorKind::Stripes).colors(self.primary, self.secondary);
        uniforms.flag = u32::from(self.horizontal);
        uniforms.uint_params = [self.stripe_width, self.offset, 0, 0];
        render_fragment(runtime, self.width, self.height, uniforms).await
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
    #[expect(
        clippy::future_not_send,
        reason = "image generators await the UI-local offscreen GpuView renderer"
    )]
    async fn generate(&self, runtime: &GpuRuntime) -> GeneratedImage {
        assert!(self.spacing > 0, "DotGridGenerator: spacing must be > 0");
        let mut uniforms =
            GeneratorUniforms::new(GeneratorKind::DotGrid).colors(self.foreground, self.background);
        uniforms.params_a[0] = self.radius;
        uniforms.uint_params[0] = self.spacing;
        render_fragment(runtime, self.width, self.height, uniforms).await
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
    #[expect(
        clippy::future_not_send,
        reason = "image generators await the UI-local offscreen GpuView renderer"
    )]
    async fn generate(&self, runtime: &GpuRuntime) -> GeneratedImage {
        let mut uniforms = GeneratorUniforms::new(GeneratorKind::Noise);
        #[expect(
            clippy::cast_precision_loss,
            reason = "the previous WGSL constant represented the seed as f32"
        )]
        {
            uniforms.params_a[0] = self.seed as f32;
        }
        render_fragment(runtime, self.width, self.height, uniforms).await
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
    #[expect(
        clippy::future_not_send,
        reason = "image generators await the UI-local offscreen GpuView renderer"
    )]
    async fn generate(&self, runtime: &GpuRuntime) -> GeneratedImage {
        let mut uniforms = GeneratorUniforms::new(GeneratorKind::LinearGradient)
            .colors(self.start_color, self.end_color);
        uniforms.params_a = [
            self.start_point[0],
            self.start_point[1],
            self.end_point[0],
            self.end_point[1],
        ];
        render_fragment(runtime, self.width, self.height, uniforms).await
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
    #[expect(
        clippy::future_not_send,
        reason = "image generators await the UI-local offscreen GpuView renderer"
    )]
    async fn generate(&self, runtime: &GpuRuntime) -> GeneratedImage {
        let mut uniforms = GeneratorUniforms::new(GeneratorKind::RadialGradient)
            .colors(self.inner_color, self.outer_color);
        uniforms.params_a = [self.center[0], self.center[1], self.radius, 0.0];
        render_fragment(runtime, self.width, self.height, uniforms).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> GpuRuntime {
        pollster::block_on(GpuRuntime::new())
            .expect("image generator tests require a working GPU runtime")
    }

    #[test]
    fn checkerboard_generator_produces_expected_dimensions() {
        let generator = CheckerboardGenerator {
            width: 16,
            height: 12,
            cell_size: 4,
            light: Srgb::WHITE,
            dark: Srgb::BLACK,
            offset_x: 0,
            offset_y: 0,
        };
        let image = pollster::block_on(generator.generate(&runtime()));
        assert_eq!(image.width(), 16);
        assert_eq!(image.height(), 12);
        assert_eq!(image.rgba8().len(), 16 * 12 * 4);
    }

    #[test]
    fn noise_generator_is_not_flat() {
        let generator = NoiseGenerator {
            width: 16,
            height: 16,
            seed: 42,
        };
        let image = pollster::block_on(generator.generate(&runtime()));
        let first = &image.rgba8()[..4];
        assert!(image.rgba8().chunks_exact(4).any(|px| px != first));
    }
}
