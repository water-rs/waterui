//! GPU-accelerated gradient rendering.
//!
//! This module provides a [`GradientRenderer`] that implements [`GpuRenderer`]
//! for rendering linear, radial, angular, and mesh gradients using WGSL shaders.
//!
//! # Architecture
//!
//! The renderer uses a single shader that handles all gradient types via uniforms.
//! Color stops are passed via a storage buffer, and gradient parameters via a uniform buffer.
//!
//! # Animation
//!
//! Mesh gradient vertices support animation through reactive `Computed<T>` values.
//! The renderer reads current values each frame and updates the uniform buffer accordingly.

extern crate alloc;

use alloc::vec::Vec;

use crate::gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};
use crate::color::ResolvedColor;
use waterui_core::View;
use crate::include_shader;

static GRADIENT_SHADER: crate::prewarm::PrewarmedShader = include_shader!("shaders/gradient.wgsl");

/// Maximum number of color stops supported by the shader.
pub const MAX_COLOR_STOPS: usize = 16;

/// Maximum number of mesh vertices supported (for mesh gradients).
pub const MAX_MESH_VERTICES: usize = 64;

/// Gradient type discriminator for the shader.
#[repr(u32)]
#[derive(Debug, Clone, Copy, Default)]
pub enum GradientType {
    /// Linear gradient along a line.
    #[default]
    Linear = 0,
    /// Radial gradient from a center point.
    Radial = 1,
    /// Angular (conic) gradient around a center.
    Angular = 2,
    /// 2D mesh gradient with interpolated vertices.
    Mesh = 3,
}

/// A resolved color stop ready for GPU upload.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuColorStop {
    /// RGBA color in linear space.
    pub color: [f32; 4],
    /// Position along the gradient (0.0 to 1.0).
    pub position: f32,
    /// Padding for alignment.
    pub _padding: [f32; 3],
}

/// A resolved mesh vertex ready for GPU upload.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuMeshVertex {
    /// Position in unit coordinates (0.0 to 1.0).
    pub position: [f32; 2],
    /// Padding for alignment.
    pub _padding1: [f32; 2],
    /// RGBA color in linear space.
    pub color: [f32; 4],
}

/// Uniform buffer layout for gradient parameters.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GradientUniforms {
    /// Gradient type (0=linear, 1=radial, 2=angular, 3=mesh).
    pub gradient_type: u32,
    /// Number of color stops.
    pub num_stops: u32,
    /// Mesh grid width (for mesh gradients).
    pub mesh_width: u32,
    /// Mesh grid height (for mesh gradients).
    pub mesh_height: u32,
    /// Start point for linear gradient, center for radial/angular.
    pub start_point: [f32; 2],
    /// End point for linear gradient.
    pub end_point: [f32; 2],
    /// Start radius for radial, start angle for angular.
    pub start_value: f32,
    /// End radius for radial, end angle for angular.
    pub end_value: f32,
    /// Whether to smooth colors (for mesh gradients).
    pub smooths_colors: u32,
    /// Padding for alignment.
    pub _padding: u32,
}

/// Configuration for creating a gradient renderer.
#[derive(Debug, Clone)]
pub struct GradientConfig {
    /// Type of gradient.
    pub gradient_type: GradientType,
    /// Color stops (position + color).
    pub stops: Vec<(f32, ResolvedColor)>,
    /// Start point (linear) or center (radial/angular).
    pub start_point: [f32; 2],
    /// End point (linear only).
    pub end_point: [f32; 2],
    /// Start radius (radial) or start angle in radians (angular).
    pub start_value: f32,
    /// End radius (radial) or end angle in radians (angular).
    pub end_value: f32,
    /// Mesh grid dimensions (width, height) for mesh gradients.
    pub mesh_size: (u32, u32),
    /// Mesh vertices for mesh gradients.
    pub mesh_vertices: Vec<([f32; 2], ResolvedColor)>,
    /// Whether to smooth colors (mesh gradients).
    pub smooths_colors: bool,
}

impl Default for GradientConfig {
    fn default() -> Self {
        Self {
            gradient_type: GradientType::Linear,
            stops: vec![
                (0.0, ResolvedColor { red: 1.0, green: 0.0, blue: 0.0, opacity: 1.0, headroom: 0.0 }),
                (1.0, ResolvedColor { red: 0.0, green: 0.0, blue: 1.0, opacity: 1.0, headroom: 0.0 }),
            ],
            start_point: [0.5, 0.0],
            end_point: [0.5, 1.0],
            start_value: 0.0,
            end_value: 1.0,
            mesh_size: (2, 2),
            mesh_vertices: Vec::new(),
            smooths_colors: true,
        }
    }
}

impl GradientConfig {
    /// Creates a linear gradient configuration.
    pub fn linear(
        stops: Vec<(f32, ResolvedColor)>,
        start: [f32; 2],
        end: [f32; 2],
    ) -> Self {
        Self {
            gradient_type: GradientType::Linear,
            stops,
            start_point: start,
            end_point: end,
            ..Default::default()
        }
    }

    /// Creates a radial gradient configuration.
    pub fn radial(
        stops: Vec<(f32, ResolvedColor)>,
        center: [f32; 2],
        start_radius: f32,
        end_radius: f32,
    ) -> Self {
        Self {
            gradient_type: GradientType::Radial,
            stops,
            start_point: center,
            end_point: center,
            start_value: start_radius,
            end_value: end_radius,
            ..Default::default()
        }
    }

    /// Creates an angular gradient configuration.
    pub fn angular(
        stops: Vec<(f32, ResolvedColor)>,
        center: [f32; 2],
        start_angle: f32,
        end_angle: f32,
    ) -> Self {
        Self {
            gradient_type: GradientType::Angular,
            stops,
            start_point: center,
            end_point: center,
            start_value: start_angle,
            end_value: end_angle,
            ..Default::default()
        }
    }

    /// Creates a mesh gradient configuration.
    pub fn mesh(
        width: u32,
        height: u32,
        vertices: Vec<([f32; 2], ResolvedColor)>,
        smooths_colors: bool,
    ) -> Self {
        Self {
            gradient_type: GradientType::Mesh,
            stops: Vec::new(),
            mesh_size: (width, height),
            mesh_vertices: vertices,
            smooths_colors,
            ..Default::default()
        }
    }
}

/// GPU renderer for gradient backgrounds.
///
/// This renderer uses wgpu to draw gradients with a fragment shader.
/// It supports linear, radial, angular, and mesh gradients.
pub struct GradientRenderer {
    config: GradientConfig,
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    stops_buffer: Option<wgpu::Buffer>,
    mesh_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    pipeline_format: Option<wgpu::TextureFormat>,
    dirty: bool,
}

impl core::fmt::Debug for GradientRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GradientRenderer")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl GradientRenderer {
    /// Creates a new gradient renderer with the given configuration.
    #[must_use]
    pub fn new(config: GradientConfig) -> Self {
        Self {
            config,
            pipeline: None,
            uniform_buffer: None,
            stops_buffer: None,
            mesh_buffer: None,
            bind_group: None,
            pipeline_format: None,
            dirty: true,
        }
    }

    /// Creates a GpuSurface wrapping this renderer.
    #[must_use]
    pub fn into_surface(self) -> GpuSurface {
        GpuSurface::new(self)
    }

    fn build_shader() -> &'static str {
        // Use pre-warmed shader source
        &GRADIENT_SHADER.source
    }

    fn prepare_uniforms(&self) -> GradientUniforms {
        GradientUniforms {
            gradient_type: self.config.gradient_type as u32,
            num_stops: self.config.stops.len().min(MAX_COLOR_STOPS) as u32,
            mesh_width: self.config.mesh_size.0,
            mesh_height: self.config.mesh_size.1,
            start_point: self.config.start_point,
            end_point: self.config.end_point,
            start_value: self.config.start_value,
            end_value: self.config.end_value,
            smooths_colors: u32::from(self.config.smooths_colors),
            _padding: 0,
        }
    }

    fn prepare_stops(&self) -> [GpuColorStop; MAX_COLOR_STOPS] {
        let mut stops = [GpuColorStop::default(); MAX_COLOR_STOPS];
        for (i, (position, color)) in self.config.stops.iter().take(MAX_COLOR_STOPS).enumerate() {
            stops[i] = GpuColorStop {
                color: [color.red, color.green, color.blue, color.opacity],
                position: *position,
                _padding: [0.0; 3],
            };
        }
        stops
    }

    fn prepare_mesh_vertices(&self) -> [GpuMeshVertex; MAX_MESH_VERTICES] {
        let mut vertices = [GpuMeshVertex::default(); MAX_MESH_VERTICES];
        for (i, (position, color)) in self.config.mesh_vertices.iter().take(MAX_MESH_VERTICES).enumerate() {
            vertices[i] = GpuMeshVertex {
                position: *position,
                _padding1: [0.0; 2],
                color: [color.red, color.green, color.blue, color.opacity],
            };
        }
        vertices
    }
}

impl GpuRenderer for GradientRenderer {
    fn setup(&mut self, ctx: &GpuContext) {
        tracing::debug!(
            "[GradientRenderer] setup() called with format: {:?}",
            ctx.surface_format
        );

        let shader_source = Self::build_shader();
        let shader = ctx.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Gradient Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        // Create uniform buffer
        let uniform_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Gradient Uniforms"),
            size: core::mem::size_of::<GradientUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create color stops buffer
        let stops_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Gradient Color Stops"),
            size: (core::mem::size_of::<GpuColorStop>() * MAX_COLOR_STOPS) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create mesh vertices buffer
        let mesh_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Gradient Mesh Vertices"),
            size: (core::mem::size_of::<GpuMeshVertex>() * MAX_MESH_VERTICES) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = ctx.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Gradient Bind Group Layout"),
            entries: &[
                // Uniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Color stops
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Mesh vertices
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Gradient Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: stops_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: mesh_buffer.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = ctx.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Gradient Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Try with cache first
        ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);

        let mut pipeline = ctx.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Gradient Pipeline"),
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
            cache: ctx.pipeline_cache,
        });

        // Check for validation error
        let error = crate::pollster::block_on(ctx.device.pop_error_scope());
        if let Some(e) = error {
            tracing::warn!("[GradientRenderer] Pipeline creation with cache failed: {}", e);
            // Retry without cache
            pipeline = ctx.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Gradient Pipeline (No Cache)"),
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
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
        } else {
            tracing::info!("[GradientRenderer] Pipeline creation with cache SUCCESS");
        }

        self.pipeline = Some(pipeline);
        self.uniform_buffer = Some(uniform_buffer);
        self.stops_buffer = Some(stops_buffer);
        self.mesh_buffer = Some(mesh_buffer);
        self.bind_group = Some(bind_group);
        self.pipeline_format = Some(ctx.surface_format);
    }

    fn render(&mut self, frame: &GpuFrame) {
        // Check if pipeline format matches
        if let Some(pipeline_fmt) = self.pipeline_format {
            if pipeline_fmt != frame.format {
                tracing::warn!(
                    "[GradientRenderer] Format mismatch: {:?} vs {:?}",
                    pipeline_fmt,
                    frame.format
                );
                self.pipeline = None;
                self.pipeline_format = None;
                return;
            }
        }

        let Some(pipeline) = &self.pipeline else {
            tracing::warn!("[GradientRenderer] No pipeline available");
            return;
        };
        let Some(uniform_buffer) = &self.uniform_buffer else { return };
        let Some(stops_buffer) = &self.stops_buffer else { return };
        let Some(mesh_buffer) = &self.mesh_buffer else { return };
        let Some(bind_group) = &self.bind_group else { return };

        let Some(bind_group) = &self.bind_group else { return };
        
        // Only update buffers if dirty
        if self.dirty {
            // Update uniforms
            let uniforms = self.prepare_uniforms();
            frame.queue.write_buffer(uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

            // Update color stops
            let stops = self.prepare_stops();
            frame.queue.write_buffer(stops_buffer, 0, bytemuck::cast_slice(&stops));

            // Update mesh vertices (if mesh gradient)
            if matches!(self.config.gradient_type, GradientType::Mesh) {
                let vertices = self.prepare_mesh_vertices();
                frame.queue.write_buffer(mesh_buffer, 0, bytemuck::cast_slice(&vertices));
            }
            
            self.dirty = false;
        }

        // Render
        let mut encoder = frame.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Gradient Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Gradient Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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
            render_pass.draw(0..6, 0..1); // Full-screen quad
        }

        frame.queue.submit(core::iter::once(encoder.finish()));
    }
}

/// A gradient view that wraps `GpuSurface` with a gradient renderer.
///
/// This is the primary way to use gradients as views.
pub struct Gradient {
    inner: GpuSurface,
}

impl core::fmt::Debug for Gradient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Gradient").finish_non_exhaustive()
    }
}

impl Gradient {
    /// Creates a new gradient view with the given configuration.
    #[must_use]
    pub fn new(config: GradientConfig) -> Self {
        Self {
            inner: GradientRenderer::new(config).into_surface(),
        }
    }

    /// Creates a linear gradient view.
    #[must_use]
    pub fn linear(
        stops: Vec<(f32, ResolvedColor)>,
        start: [f32; 2],
        end: [f32; 2],
    ) -> Self {
        Self::new(GradientConfig::linear(stops, start, end))
    }

    /// Creates a radial gradient view.
    #[must_use]
    pub fn radial(
        stops: Vec<(f32, ResolvedColor)>,
        center: [f32; 2],
        start_radius: f32,
        end_radius: f32,
    ) -> Self {
        Self::new(GradientConfig::radial(stops, center, start_radius, end_radius))
    }

    /// Creates an angular gradient view.
    #[must_use]
    pub fn angular(
        stops: Vec<(f32, ResolvedColor)>,
        center: [f32; 2],
        start_angle: f32,
        end_angle: f32,
    ) -> Self {
        Self::new(GradientConfig::angular(stops, center, start_angle, end_angle))
    }

    /// Creates a mesh gradient view.
    #[must_use]
    pub fn mesh(
        width: u32,
        height: u32,
        vertices: Vec<([f32; 2], ResolvedColor)>,
        smooths_colors: bool,
    ) -> Self {
        Self::new(GradientConfig::mesh(width, height, vertices, smooths_colors))
    }
}

impl View for Gradient {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        self.inner
    }
}

// ============================================================================
// Reactive Mesh Gradient (accepts Signal parameters for animation)
// ============================================================================

use waterui_core::Signal;

/// A mesh gradient that accepts reactive Signal parameters for animation.
///
/// Unlike the static `GradientConfig::mesh()`, this type can accept `Signal`
/// parameters that are read each frame, enabling smooth animations.
///
/// # Example
///
/// ```ignore
/// use waterui_graphics::MeshGradient;
/// use waterui_core::{Binding, binding};
/// use crate::color::ResolvedColor;
///
/// // Create animated colors binding
/// let colors: Binding<Vec<ResolvedColor>> = binding(vec![
///     ResolvedColor::default(),
///     // ... 9 colors for 3x3 mesh
/// ]);
///
/// // Create mesh gradient with reactive colors
/// let gradient = MeshGradient::new(3, 3, colors.clone());
///
/// // Animate by updating the binding
/// colors.set(new_colors); // Gradient updates automatically!
/// ```
pub struct MeshGradient<C> {
    width: u32,
    height: u32,
    colors: C,
    smooths_colors: bool,
}

impl<C> core::fmt::Debug for MeshGradient<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MeshGradient")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("smooths_colors", &self.smooths_colors)
            .finish_non_exhaustive()
    }
}

impl<C> MeshGradient<C> {
    /// Creates a new mesh gradient with the given dimensions and colors signal.
    ///
    /// The colors signal is read each frame to update the gradient.
    #[must_use]
    pub const fn new(width: u32, height: u32, colors: C) -> Self {
        Self {
            width,
            height,
            colors,
            smooths_colors: true,
        }
    }

    /// Sets whether to smooth colors between mesh vertices.
    #[must_use]
    pub const fn smooths_colors(mut self, smooths: bool) -> Self {
        self.smooths_colors = smooths;
        self
    }
}

/// Internal renderer for reactive mesh gradients.
struct ReactiveMeshRenderer<C> {
    width: u32,
    height: u32,
    colors: C,
    smooths_colors: bool,
    // GPU resources
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    stops_buffer: Option<wgpu::Buffer>,
    mesh_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    pipeline_format: Option<wgpu::TextureFormat>,
    last_colors: Option<Vec<ResolvedColor>>,
}

impl<C> ReactiveMeshRenderer<C> {
    fn new(width: u32, height: u32, colors: C, smooths_colors: bool) -> Self {
        Self {
            width,
            height,
            colors,
            smooths_colors,
            pipeline: None,
            uniform_buffer: None,
            stops_buffer: None,
            mesh_buffer: None,
            bind_group: None,
            pipeline_format: None,
            last_colors: None,
        }
    }
}

impl<C> GpuRenderer for ReactiveMeshRenderer<C>
where
    C: Signal + 'static,
    C::Output: IntoIterator<Item = ResolvedColor>,
{
    fn setup(&mut self, ctx: &GpuContext) {
        // Use pre-warmed shader
        let shader = ctx.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Mesh Gradient Shader"),
            source: wgpu::ShaderSource::Wgsl(GRADIENT_SHADER.source.clone()),
        });

        // Create uniform buffer
        let uniform_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mesh Gradient Uniforms"),
            size: core::mem::size_of::<GradientUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create color stops buffer (not used for mesh, but required by shader)
        let stops_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mesh Gradient Color Stops"),
            size: (core::mem::size_of::<GpuColorStop>() * MAX_COLOR_STOPS) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Create mesh vertices buffer
        let mesh_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mesh Gradient Vertices"),
            size: (core::mem::size_of::<GpuMeshVertex>() * MAX_MESH_VERTICES) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = ctx.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Mesh Gradient Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Mesh Gradient Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: stops_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: mesh_buffer.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = ctx.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Mesh Gradient Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Try with cache first
        ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);

        let mut pipeline = ctx.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Mesh Gradient Pipeline"),
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
            cache: ctx.pipeline_cache,
        });

        // Check for validation error
        let error = crate::pollster::block_on(ctx.device.pop_error_scope());
        if let Some(e) = error {
            tracing::warn!("[ReactiveMeshRenderer] Pipeline creation with cache failed: {}", e);
            // Retry without cache
            pipeline = ctx.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Mesh Gradient Pipeline (No Cache)"),
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
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
        } else {
            tracing::info!("[ReactiveMeshRenderer] Pipeline creation with cache SUCCESS");
        }

        self.pipeline = Some(pipeline);
        self.uniform_buffer = Some(uniform_buffer);
        self.stops_buffer = Some(stops_buffer);
        self.mesh_buffer = Some(mesh_buffer);
        self.bind_group = Some(bind_group);
        self.pipeline_format = Some(ctx.surface_format);
    }

    fn render(&mut self, frame: &GpuFrame) {
        // Check format match
        if let Some(pipeline_fmt) = self.pipeline_format {
            if pipeline_fmt != frame.format {
                self.pipeline = None;
                self.pipeline_format = None;
                return;
            }
        }

        let Some(pipeline) = &self.pipeline else { return };
        let Some(uniform_buffer) = &self.uniform_buffer else { return };
        let Some(stops_buffer) = &self.stops_buffer else { return };
        let Some(mesh_buffer) = &self.mesh_buffer else { return };
        let Some(bind_group) = &self.bind_group else { return };

        // Read current colors from Signal
        let colors: Vec<ResolvedColor> = self.colors.get().into_iter().collect();

        // Check if colors changed
        let colors_changed = match &self.last_colors {
            Some(last) => last.len() != colors.len() || last.iter().zip(&colors).any(|(a, b)| {
                a.red != b.red || a.green != b.green || a.blue != b.blue || a.opacity != b.opacity
            }),
            None => true,
        };

        if colors_changed {
            // Update cache
            self.last_colors = Some(colors.clone());

            // Prepare uniforms
            let uniforms = GradientUniforms {
                gradient_type: GradientType::Mesh as u32,
                num_stops: 0,
                mesh_width: self.width,
                mesh_height: self.height,
                start_point: [0.0, 0.0],
                end_point: [1.0, 1.0],
                start_value: 0.0,
                end_value: 1.0,
                smooths_colors: u32::from(self.smooths_colors),
                _padding: 0,
            };
            frame.queue.write_buffer(uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

            // Prepare empty stops (mesh gradient doesn't use stops)
            let stops = [GpuColorStop::default(); MAX_COLOR_STOPS];
            frame.queue.write_buffer(stops_buffer, 0, bytemuck::cast_slice(&stops));

            // Prepare mesh vertices from colors
            // Generate grid positions and map colors
            let mut vertices = [GpuMeshVertex::default(); MAX_MESH_VERTICES];
            let w = self.width as usize;
            let h = self.height as usize;

            for (i, color) in colors.iter().take(MAX_MESH_VERTICES).enumerate() {
                let x = (i % w) as f32 / (w - 1).max(1) as f32;
                let y = (i / w) as f32 / (h - 1).max(1) as f32;
                vertices[i] = GpuMeshVertex {
                    position: [x, y],
                    _padding1: [0.0; 2],
                    color: [color.red, color.green, color.blue, color.opacity],
                };
            }
            frame.queue.write_buffer(mesh_buffer, 0, bytemuck::cast_slice(&vertices));
        }

        // Render
        let mut encoder = frame.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Mesh Gradient Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Mesh Gradient Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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

        frame.queue.submit(core::iter::once(encoder.finish()));
    }
}

impl<C> MeshGradient<C>
where
    C: Signal + 'static,
    C::Output: IntoIterator<Item = ResolvedColor>,
{
    /// Converts this mesh gradient into a `GpuSurface` for rendering.
    #[must_use]
    pub fn into_surface(self) -> GpuSurface {
        GpuSurface::new(ReactiveMeshRenderer::new(
            self.width,
            self.height,
            self.colors,
            self.smooths_colors,
        ))
    }
}

impl<C> View for MeshGradient<C>
where
    C: Signal + 'static,
    C::Output: IntoIterator<Item = ResolvedColor>,
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        self.into_surface()
    }
}
