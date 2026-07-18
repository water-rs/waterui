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
use core::fmt;
use core::num::NonZeroU64;
use std::time::Instant;

use crate::gpu_surface::{GpuContext, GpuFrame, GpuSurface, GpuView};
use shaderloom::{CompiledShader, CompiledShaderModule};

/// A simplified GPU surface that renders a custom fragment shader.
///
/// Unlike `GpuSurface` which requires implementing `GpuView`,
/// `ShaderSurface` only needs a WGSL fragment shader string.
/// All pipeline setup and rendering is handled automatically.
pub struct ShaderSurface {
    inner: GpuSurface,
}

impl fmt::Debug for ShaderSurface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

    /// Macro implementation for a compile-time labeled fragment shader.
    #[doc(hidden)]
    #[must_use]
    pub fn from_labeled_fragment(
        label: &'static str,
        fragment_shader: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            inner: GpuSurface::new(ShaderRenderer::new_labeled(label, fragment_shader.into())),
        }
    }

    #[must_use]
    pub(crate) fn from_compiled_shader(shader: &'static CompiledShader) -> Self {
        Self {
            inner: GpuSurface::new(ShaderRenderer::new_compiled(shader)),
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
    ($path:literal) => {{
        const SHADER: $crate::shader_source::ShaderSource = $crate::include_fragment_shader!($path);
        $crate::shader_surface::ShaderSurface::from_labeled_fragment(SHADER.label, SHADER.source)
    }};
}

struct ShaderResources {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// Internal renderer that handles all the wgpu boilerplate.
struct ShaderRenderer {
    /// Optional label for cache lookup (from `include_fragment_shader!`)
    label: Option<&'static str>,
    /// Optional backend-native built-in shader.
    compiled_shader: Option<&'static CompiledShader>,
    fragment_source: Cow<'static, str>,
    resources: Option<ShaderResources>,
    start_time: Instant,
}

impl ShaderRenderer {
    fn new(fragment_source: Cow<'static, str>) -> Self {
        Self {
            label: None,
            compiled_shader: None,
            fragment_source,
            resources: None,
            start_time: Instant::now(),
        }
    }

    fn new_labeled(label: &'static str, fragment_source: Cow<'static, str>) -> Self {
        Self {
            label: Some(label),
            compiled_shader: None,
            fragment_source,
            resources: None,
            start_time: Instant::now(),
        }
    }

    fn new_compiled(shader: &'static CompiledShader) -> Self {
        Self {
            label: None,
            compiled_shader: Some(shader),
            fragment_source: Cow::Borrowed(""),
            resources: None,
            start_time: Instant::now(),
        }
    }

    fn build_full_shader(&self) -> String {
        // Prepend the uniform struct and vertex shader to user's fragment shader
        let mut full = String::with_capacity(
            crate::shader_source::SHADER_SURFACE_PRELUDE.len() + self.fragment_source.len(),
        );
        full.push_str(PRELUDE);
        full.push_str(&self.fragment_source);
        full
    }

    fn create_shader_and_layout(
        &self,
        ctx: &GpuContext<'_>,
    ) -> (SetupShaderModules, wgpu::BindGroupLayout) {
        if let Some(compiled) = self.compiled_shader {
            let (vertex, fragment) = compiled.create_render_stages(ctx.device, "vs_main", "main");
            let mut layouts = compiled.create_bind_group_layouts(ctx.device);
            assert_eq!(
                layouts.len(),
                1,
                "a compiled ShaderSurface must use exactly one bind group"
            );
            return (
                SetupShaderModules::Compiled { vertex, fragment },
                layouts
                    .pop()
                    .expect("one compiled ShaderSurface bind group was asserted"),
            );
        }

        let full_shader = self.build_full_shader();
        let shader_label = self.label.unwrap_or("ShaderSurface Shader");
        let module = shaderloom::create_dynamic_wgsl_module(
            ctx.device,
            Some(shader_label),
            &full_shader,
        );
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ShaderSurface Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(24),
                    },
                    count: None,
                }],
            });
        (SetupShaderModules::Dynamic(module), layout)
    }

    fn create_pipeline(
        ctx: &GpuContext<'_>,
        shader: &SetupShaderModules,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> wgpu::RenderPipeline {
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ShaderSurface Pipeline Layout"),
                bind_group_layouts: &[Some(bind_group_layout)],
                immediate_size: 0,
            });
        let blend = (!ctx.is_hdr()).then_some(wgpu::BlendState::REPLACE);
        let (vertex_module, vertex_entry_point) = shader.vertex();
        let (fragment_module, fragment_entry_point) = shader.fragment();

        ctx.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("ShaderSurface Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: vertex_module,
                    entry_point: Some(vertex_entry_point),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: fragment_module,
                    entry_point: Some(fragment_entry_point),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.surface_format,
                        blend,
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
            })
    }
}

/// Standard prelude for `ShaderSurface` shaders.
/// Includes `Uniforms`, `VertexOutput`, and the default vertex shader.
pub const PRELUDE: &str = crate::shader_source::SHADER_SURFACE_PRELUDE;

enum SetupShaderModules {
    Compiled {
        vertex: CompiledShaderModule,
        fragment: CompiledShaderModule,
    },
    Dynamic(wgpu::ShaderModule),
}

impl SetupShaderModules {
    const fn vertex(&self) -> (&wgpu::ShaderModule, &str) {
        match self {
            Self::Compiled { vertex, .. } => (vertex.module(), vertex.entry_point()),
            Self::Dynamic(module) => (module, "vs_main"),
        }
    }

    const fn fragment(&self) -> (&wgpu::ShaderModule, &str) {
        match self {
            Self::Compiled { fragment, .. } => (fragment.module(), fragment.entry_point()),
            Self::Dynamic(module) => (module, "main"),
        }
    }
}

impl GpuView for ShaderRenderer {
    fn setup(
        &mut self,
        ctx: &GpuContext<'_>,
        _env: &mut waterui_core::Environment,
    ) -> impl core::future::Future<Output = ()> {
        let (shader, bind_group_layout) = self.create_shader_and_layout(ctx);
        let pipeline = Self::create_pipeline(ctx, &shader, &bind_group_layout);

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

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ShaderSurface Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        self.resources = Some(ShaderResources {
            format: ctx.surface_format,
            pipeline,
            uniform_buffer,
            bind_group,
        });
        self.start_time = Instant::now();
        core::future::ready(())
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        let resources = self
            .resources
            .as_ref()
            .expect("ShaderSurface render called before setup");
        assert_eq!(
            resources.format, frame.format,
            "ShaderSurface target format changed after setup"
        );

        // Update uniforms with correct WGSL alignment:
        // [time: f32, _pad: f32, resolution.x: f32, resolution.y: f32, _padding: f32, _pad: f32]
        let elapsed = self.start_time.elapsed().as_secs_f32();
        #[expect(
            clippy::cast_precision_loss,
            reason = "GPU viewport dimensions are represented as f32 shader uniforms"
        )]
        let uniforms: [f32; 6] = [
            elapsed,             // time at offset 0
            0.0,                 // padding at offset 4 (for vec2 alignment)
            frame.width as f32,  // resolution.x at offset 8
            frame.height as f32, // resolution.y at offset 12
            0.0,                 // _padding at offset 16
            0.0,                 // struct padding at offset 20
        ];
        frame.queue.write_buffer(
            &resources.uniform_buffer,
            0,
            bytemuck::cast_slice(&uniforms),
        );

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

        frame.queue.submit(std::iter::once(encoder.finish()));
        frame.request_redraw();
    }
}
