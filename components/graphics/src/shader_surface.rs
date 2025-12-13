//! Simplified shader-based GPU rendering surface.
//!
//! `ShaderSurface` provides an easy way to create GPU-rendered views using
//! just a WGSL fragment shader. It automatically handles pipeline creation,
//! uniform buffers, and the render loop.
//!
//! # Example
//!
//! ```ignore
//! use waterui::graphics::shader;
//!
//! // Load shader from file (recommended)
//! shader!("shaders/effect.wgsl")
//!
//! // Inline shader
//! ShaderSurface::new(r#"
//!     @fragment
//!     fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
//!         let t = uniforms.time;
//!         return vec4<f32>(uv.x, uv.y, sin(t), 1.0);
//!     }
//! "#)
//! ```
//!
//! # Built-in Uniforms
//!
//! The following uniforms are automatically available in your shader:
//!
//! ```wgsl
//! struct Uniforms {
//!     time: f32,           // Elapsed time in seconds
//!     resolution: vec2<f32>, // Surface size in pixels
//!     _padding: f32,
//! }
//! @group(0) @binding(0) var<uniform> uniforms: Uniforms;
//! ```

extern crate alloc;

use alloc::borrow::Cow;
use alloc::string::String;

use crate::gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};

/// A simplified GPU surface that renders a custom fragment shader.
///
/// Unlike `GpuSurface` which requires implementing `GpuRenderer`,
/// `ShaderSurface` only needs a WGSL fragment shader string.
/// All pipeline setup and rendering is handled automatically.
pub struct ShaderSurface {
    inner: GpuSurface,
}

impl core::fmt::Debug for ShaderSurface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ShaderSurface").finish_non_exhaustive()
    }
}

impl ShaderSurface {
    /// Creates a new shader surface with the given fragment shader.
    ///
    /// The shader should define a `main` function with signature:
    /// ```wgsl
    /// @fragment
    /// fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32>
    /// ```
    ///
    /// Where `uv` is normalized coordinates (0.0 to 1.0).
    ///
    /// # Example
    ///
    /// ```ignore
    /// ShaderSurface::new(r#"
    ///     @fragment
    ///     fn main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    ///         return vec4<f32>(uv, 0.5, 1.0);
    ///     }
    /// "#)
    /// ```
    #[must_use]
    pub fn new(fragment_shader: impl Into<Cow<'static, str>>) -> Self {
        Self {
            inner: GpuSurface::new(ShaderRenderer::new(fragment_shader.into())),
        }
    }

    /// Consumes the `ShaderSurface` and returns the inner `GpuSurface`.
    #[must_use]
    pub fn into_inner(self) -> GpuSurface {
        self.inner
    }
}

// Implement View by delegating to GpuSurface
impl waterui_core::View for ShaderSurface {
    fn body(self, _env: &waterui_core::Environment) -> impl waterui_core::View {
        self.inner
    }
}

/// Creates a [`ShaderSurface`] from a shader file path.
///
/// This macro loads the shader source at compile time using `include_str!`
/// and creates a `ShaderSurface` with it.
///
/// # Example
///
/// ```ignore
/// use waterui::graphics::shader;
///
/// // Load shader from file relative to the current source file
/// let surface = shader!("shaders/flame.wgsl");
///
/// // Use in a view
/// vstack((
///     text("My Effect"),
///     shader!("effects/glow.wgsl"),
/// ))
/// ```
#[macro_export]
macro_rules! shader {
    ($path:literal) => {
        $crate::shader_surface::ShaderSurface::new(include_str!($path))
    };
}

/// Internal renderer that handles all the wgpu boilerplate.
struct ShaderRenderer {
    fragment_source: Cow<'static, str>,
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    start_time: std::time::Instant,
    /// The format the pipeline was created for
    pipeline_format: Option<wgpu::TextureFormat>,
}

impl ShaderRenderer {
    fn new(fragment_source: Cow<'static, str>) -> Self {
        Self {
            fragment_source,
            pipeline: None,
            uniform_buffer: None,
            bind_group: None,
            start_time: std::time::Instant::now(),
            pipeline_format: None,
        }
    }

    fn build_full_shader(&self) -> String {
        // Prepend the uniform struct and vertex shader to user's fragment shader
        let prelude = r"
// === ShaderSurface Prelude (auto-generated) ===

struct Uniforms {
    time: f32,
    resolution: vec2<f32>,
    _padding: f32,
}

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Full-screen quad using 6 vertices (2 triangles)
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
    );

    let pos = positions[vertex_index];
    var output: VertexOutput;
    output.position = vec4<f32>(pos, 0.0, 1.0);
    // UV: (0,0) at bottom-left, (1,1) at top-right
    output.uv = (pos + 1.0) * 0.5;
    return output;
}

// === User Fragment Shader ===

";
        let mut full = String::with_capacity(prelude.len() + self.fragment_source.len());
        full.push_str(prelude);
        full.push_str(&self.fragment_source);
        full
    }
}

impl GpuRenderer for ShaderRenderer {
    fn setup(&mut self, ctx: &GpuContext) {
        tracing::debug!(
            "[ShaderSurface] setup() called with format: {:?}",
            ctx.surface_format
        );
        let full_shader = self.build_full_shader();

        let shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("ShaderSurface Shader"),
                source: wgpu::ShaderSource::Wgsl(full_shader.into()),
            });

        // Uniform buffer layout (WGSL alignment rules):
        // - time: f32 at offset 0 (4 bytes)
        // - padding: 4 bytes (vec2 needs 8-byte alignment)
        // - resolution: vec2<f32> at offset 8 (8 bytes)
        // - _padding: f32 at offset 16 (4 bytes)
        // - struct padding to 8-byte alignment: 4 bytes
        // Total: 24 bytes
        let uniform_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ShaderSurface Uniforms"),
            size: 24,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("ShaderSurface Bind Group Layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: core::num::NonZeroU64::new(24),
                        },
                        count: None,
                    }],
                });

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ShaderSurface Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ShaderSurface Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        // Render directly to surface format (no intermediate texture needed for simple shaders)
        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("ShaderSurface Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("main"),
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

        self.pipeline = Some(pipeline);
        self.uniform_buffer = Some(uniform_buffer);
        self.bind_group = Some(bind_group);
        self.pipeline_format = Some(ctx.surface_format);
        self.start_time = std::time::Instant::now();
    }

    fn render(&mut self, frame: &GpuFrame) {
        // Check if pipeline format matches current frame format
        if let Some(pipeline_fmt) = self.pipeline_format
            && pipeline_fmt != frame.format {
                tracing::error!(
                    "[ShaderSurface] FORMAT MISMATCH! Pipeline: {:?}, Frame: {:?}",
                    pipeline_fmt,
                    frame.format
                );
                self.pipeline = None;
                self.pipeline_format = None;
            }

        // If no pipeline, we need setup
        if self.pipeline.is_none() {
            tracing::warn!("[ShaderSurface] No pipeline - need setup");
            return;
        }

        let Some(pipeline) = &self.pipeline else {
            return;
        };
        let Some(uniform_buffer) = &self.uniform_buffer else {
            return;
        };
        let Some(bind_group) = &self.bind_group else {
            return;
        };

        // Update uniforms with correct WGSL alignment:
        // [time: f32, _pad: f32, resolution.x: f32, resolution.y: f32, _padding: f32, _pad: f32]
        let elapsed = self.start_time.elapsed().as_secs_f32();
        #[allow(clippy::cast_precision_loss)]
        let uniforms: [f32; 6] = [
            elapsed,             // time at offset 0
            0.0,                 // padding at offset 4 (for vec2 alignment)
            frame.width as f32,  // resolution.x at offset 8
            frame.height as f32, // resolution.y at offset 12
            0.0,                 // _padding at offset 16
            0.0,                 // struct padding at offset 20
        ];
        frame
            .queue
            .write_buffer(uniform_buffer, 0, bytemuck::cast_slice(&uniforms));

        // Render directly to target
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ShaderSurface Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ShaderSurface Render Pass"),
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
            render_pass.set_bind_group(0, bind_group, &[]);
            render_pass.draw(0..6, 0..1);
        }

        frame.queue.submit(std::iter::once(encoder.finish()));
    }
}
