//! Canvas view for 2D vector graphics rendering.
//!
//! `Canvas` provides an easy-to-use API for drawing 2D graphics using Vello.
//! It renders at full GPU speed while exposing a simple, declarative interface.
//!
//! # Example
//!
//! ```ignore
//! use waterui::graphics::{Canvas, DrawingContext};
//! use waterui::graphics::kurbo::{Circle, Rect};
//! use waterui::graphics::peniko::Color;
//!
//! Canvas::new(|ctx: &mut DrawingContext| {
//!     // Fill a circle
//!     ctx.fill(
//!         Circle::new((100.0, 100.0), 50.0),
//!         Color::RED,
//!     );
//!
//!     // Stroke a rectangle
//!     ctx.stroke(
//!         Rect::new(10.0, 10.0, 200.0, 150.0),
//!         Color::BLUE,
//!         2.0,
//!     );
//! })
//! ```

use crate::gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};

// Re-export vello types for user convenience
pub use vello::kurbo;
pub use vello::peniko;
pub use vello::peniko::Color;

/// A canvas for 2D vector graphics rendering.
///
/// Canvas provides a simple callback-based API where you receive a
/// [`DrawingContext`] to draw shapes, paths, and text.
pub struct Canvas {
    inner: GpuSurface,
}

impl core::fmt::Debug for Canvas {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Canvas").finish_non_exhaustive()
    }
}

impl Canvas {
    /// Creates a new canvas with a drawing callback.
    ///
    /// The callback is invoked each frame with a [`DrawingContext`] that
    /// provides methods for drawing shapes, paths, and more.
    ///
    /// # Example
    ///
    /// ```ignore
    /// Canvas::new(|ctx| {
    ///     ctx.fill(Circle::new((50.0, 50.0), 25.0), Color::RED);
    /// })
    /// ```
    #[must_use]
    pub fn new<F>(draw: F) -> Self
    where
        F: FnMut(&mut DrawingContext) + Send + 'static,
    {
        Self {
            inner: GpuSurface::new(CanvasRenderer::new(draw)),
        }
    }
}

impl waterui_core::View for Canvas {
    fn body(self, _env: &waterui_core::Environment) -> impl waterui_core::View {
        self.inner
    }
}

/// Context for drawing 2D graphics.
///
/// This is passed to your drawing callback each frame. Use it to draw
/// shapes, paths, text, and images.
pub struct DrawingContext<'a> {
    scene: &'a mut vello::Scene,
    /// Width of the canvas in pixels.
    pub width: f32,
    /// Height of the canvas in pixels.
    pub height: f32,
}

impl core::fmt::Debug for DrawingContext<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DrawingContext")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl DrawingContext<'_> {
    /// Returns the size of the canvas as a `kurbo::Size`.
    #[must_use]
    pub fn size(&self) -> kurbo::Size {
        kurbo::Size::new(f64::from(self.width), f64::from(self.height))
    }

    /// Returns the center point of the canvas.
    #[must_use]
    pub fn center(&self) -> kurbo::Point {
        kurbo::Point::new(f64::from(self.width) / 2.0, f64::from(self.height) / 2.0)
    }

    /// Fills a shape with a color.
    ///
    /// # Example
    ///
    /// ```ignore
    /// ctx.fill(Circle::new((100.0, 100.0), 50.0), Color::RED);
    /// ```
    pub fn fill(&mut self, shape: impl kurbo::Shape, color: peniko::Color) {
        self.scene.fill(
            peniko::Fill::NonZero,
            kurbo::Affine::IDENTITY,
            color,
            None,
            &shape,
        );
    }

    /// Fills a shape with a brush (gradient, pattern, etc).
    pub fn fill_brush(&mut self, shape: impl kurbo::Shape, brush: &peniko::Brush) {
        self.scene.fill(
            peniko::Fill::NonZero,
            kurbo::Affine::IDENTITY,
            brush,
            None,
            &shape,
        );
    }

    /// Fills a shape with a color and custom transform.
    pub fn fill_with_transform(
        &mut self,
        shape: impl kurbo::Shape,
        color: peniko::Color,
        transform: kurbo::Affine,
    ) {
        self.scene
            .fill(peniko::Fill::NonZero, transform, color, None, &shape);
    }

    /// Strokes a shape with a color and line width.
    ///
    /// # Example
    ///
    /// ```ignore
    /// ctx.stroke(Rect::new(10.0, 10.0, 100.0, 80.0), Color::BLUE, 2.0);
    /// ```
    pub fn stroke(&mut self, shape: impl kurbo::Shape, color: peniko::Color, width: f64) {
        let stroke = kurbo::Stroke::new(width);
        self.scene
            .stroke(&stroke, kurbo::Affine::IDENTITY, color, None, &shape);
    }

    /// Strokes a shape with a brush and line width.
    pub fn stroke_brush(&mut self, shape: impl kurbo::Shape, brush: &peniko::Brush, width: f64) {
        let stroke = kurbo::Stroke::new(width);
        self.scene
            .stroke(&stroke, kurbo::Affine::IDENTITY, brush, None, &shape);
    }

    /// Strokes a shape with custom stroke style.
    pub fn stroke_with_style(
        &mut self,
        shape: impl kurbo::Shape,
        color: peniko::Color,
        stroke: &kurbo::Stroke,
    ) {
        self.scene
            .stroke(stroke, kurbo::Affine::IDENTITY, color, None, &shape);
    }

    /// Strokes a shape with custom stroke style and transform.
    pub fn stroke_with_transform(
        &mut self,
        shape: impl kurbo::Shape,
        color: peniko::Color,
        stroke: &kurbo::Stroke,
        transform: kurbo::Affine,
    ) {
        self.scene.stroke(stroke, transform, color, None, &shape);
    }

    /// Pushes a clip layer. All subsequent drawing will be clipped to the shape.
    ///
    /// Call [`pop_layer`](Self::pop_layer) when done drawing in this layer.
    pub fn push_clip(&mut self, clip: impl kurbo::Shape) {
        self.scene.push_clip_layer(kurbo::Affine::IDENTITY, &clip);
    }

    /// Pushes a layer with alpha (opacity).
    ///
    /// Call [`pop_layer`](Self::pop_layer) when done drawing in this layer.
    pub fn push_alpha(&mut self, alpha: f32, bounds: impl kurbo::Shape) {
        self.scene.push_layer(
            peniko::BlendMode::default(),
            alpha,
            kurbo::Affine::IDENTITY,
            &bounds,
        );
    }

    /// Pops the current layer.
    pub fn pop_layer(&mut self) {
        self.scene.pop_layer();
    }

    /// Access the underlying Vello scene for advanced operations.
    ///
    /// Use this when you need features not exposed by the simplified API.
    #[must_use]
    pub const fn scene(&mut self) -> &mut vello::Scene {
        self.scene
    }
}

/// Internal renderer that bridges Canvas to `GpuSurface`.
struct CanvasRenderer<F> {
    draw_fn: F,
    scene: vello::Scene,
    renderer: Option<vello::Renderer>,
    /// Intermediate texture for Vello (Rgba8Unorm format required by Vello)
    intermediate_texture: Option<wgpu::Texture>,
    intermediate_view: Option<wgpu::TextureView>,
    /// Blit pipeline for copying intermediate texture to target (handles HDR surfaces)
    blit_pipeline: Option<wgpu::RenderPipeline>,
    blit_bind_group_layout: Option<wgpu::BindGroupLayout>,
    blit_sampler: Option<wgpu::Sampler>,
    /// Current intermediate texture dimensions
    intermediate_size: (u32, u32),
}

impl<F> CanvasRenderer<F> {
    fn new(draw_fn: F) -> Self {
        Self {
            draw_fn,
            scene: vello::Scene::new(),
            renderer: None,
            intermediate_texture: None,
            intermediate_view: None,
            blit_pipeline: None,
            blit_bind_group_layout: None,
            blit_sampler: None,
            intermediate_size: (0, 0),
        }
    }
}

impl<F> GpuRenderer for CanvasRenderer<F>
where
    F: FnMut(&mut DrawingContext) + Send + 'static,
{
    fn setup(&mut self, ctx: &GpuContext) {
        let renderer = vello::Renderer::new(
            ctx.device,
            vello::RendererOptions {
                use_cpu: false,
                antialiasing_support: vello::AaSupport::area_only(),
                num_init_threads: std::num::NonZeroUsize::new(1), // Single thread on macOS
                pipeline_cache: None,
            },
        )
        .expect("Failed to create Vello renderer");
        self.renderer = Some(renderer);

        // Create blit pipeline for copying from Rgba8Unorm to target format
        let shader = ctx.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Canvas Blit Shader"),
            source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
        });

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Canvas Blit Bind Group Layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Canvas Blit Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Canvas Blit Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.surface_format,
                        blend: Some(wgpu::BlendState::REPLACE),
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
                multiview: None,
                cache: None,
            });

        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Canvas Blit Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        self.blit_pipeline = Some(pipeline);
        self.blit_bind_group_layout = Some(bind_group_layout);
        self.blit_sampler = Some(sampler);
    }

    fn resize(&mut self, width: u32, height: u32) {
        // Mark that we need to recreate the intermediate texture
        self.intermediate_size = (0, 0);
        let _ = (width, height);
    }

    fn render(&mut self, frame: &GpuFrame) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };

        // Recreate intermediate texture if size changed
        if self.intermediate_size != (frame.width, frame.height) {
            let texture = frame.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Canvas Intermediate Texture"),
                size: wgpu::Extent3d {
                    width: frame.width,
                    height: frame.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // Vello requires Rgba8Unorm format
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.intermediate_texture = Some(texture);
            self.intermediate_view = Some(view);
            self.intermediate_size = (frame.width, frame.height);
        }

        let Some(intermediate_view) = &self.intermediate_view else {
            return;
        };

        // Clear and rebuild scene
        self.scene.reset();

        // Create drawing context and invoke user's draw function
        #[allow(clippy::cast_precision_loss)]
        let mut ctx = DrawingContext {
            scene: &mut self.scene,
            width: frame.width as f32,
            height: frame.height as f32,
        };
        (self.draw_fn)(&mut ctx);

        // Render the scene to intermediate texture (Rgba8Unorm)
        let params = vello::RenderParams {
            base_color: peniko::Color::TRANSPARENT,
            width: frame.width,
            height: frame.height,
            antialiasing_method: vello::AaConfig::Area,
        };

        renderer
            .render_to_texture(frame.device, frame.queue, &self.scene, intermediate_view, &params)
            .expect("Failed to render Vello scene");

        // Blit from intermediate texture to target (may be HDR format)
        let Some(pipeline) = &self.blit_pipeline else {
            return;
        };
        let Some(bind_group_layout) = &self.blit_bind_group_layout else {
            return;
        };
        let Some(sampler) = &self.blit_sampler else {
            return;
        };

        let bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Canvas Blit Bind Group"),
            layout: bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(intermediate_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Canvas Blit Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Canvas Blit Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..6, 0..1);
        }

        frame.queue.submit(std::iter::once(encoder.finish()));
    }
}

/// WGSL shader for blitting from Rgba8Unorm to target format
const BLIT_SHADER: &str = r"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Full-screen triangle pair
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
    );
    var tex_coords = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, 0.0),
    );

    var output: VertexOutput;
    output.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    output.tex_coord = tex_coords[vertex_index];
    return output;
}

@group(0) @binding(0) var t_source: texture_2d<f32>;
@group(0) @binding(1) var s_source: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(t_source, s_source, in.tex_coord);
}
";
