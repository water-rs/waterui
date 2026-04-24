//! Gradient primitives and mesh gradient GPU rendering.
//!
//! Linear/radial/angular gradients are resolved into `ResolvedGradient` raw views
//! so backends render them natively.
//! Mesh gradients remain GPU-backed (`GpuView`) because they require custom
//! interpolation in shader space.

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};
use num_traits::ToPrimitive;

use crate::color::ResolvedColor;
use crate::gpu_surface::{GpuContext, GpuFrame, GpuSurface, GpuView};
use crate::include_shader;
use encase::{ShaderSize, StorageBuffer, UniformBuffer};
use waterui_core::{AnyView, Signal, View};

static MESH_GRADIENT_SHADER: crate::prewarm::PrewarmedShader =
    include_shader!("shaders/mesh_gradient.wgsl");

/// Maximum number of color stops supported by the mesh shader buffer layout.
pub const MAX_COLOR_STOPS: usize = 16;

/// Maximum number of mesh vertices supported by the mesh shader.
pub const MAX_MESH_VERTICES: usize = 64;

/// Gradient type discriminator.
#[repr(u32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GradientType {
    /// Linear gradient along a line.
    #[default]
    Linear = 0,
    /// Radial gradient from a center point.
    Radial = 1,
    /// Angular (conic) gradient around a center point.
    Angular = 2,
    /// 2D mesh gradient.
    Mesh = 3,
}

nami::impl_constant!(GradientType);

/// A resolved color stop for backend-native gradient rendering.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedGradientStop {
    /// Position in range `[0.0, 1.0]`.
    pub position: f32,
    /// Stop color in linear color space.
    pub color: ResolvedColor,
}

impl ResolvedGradientStop {
    /// Creates a stop from position + color.
    ///
    /// # Panics
    ///
    /// Panics when the position or any color channel is outside the documented range.
    #[must_use]
    pub fn new(position: f32, color: ResolvedColor) -> Self {
        assert!(
            position.is_finite(),
            "gradient stop position must be finite"
        );
        assert!(
            (0.0..=1.0).contains(&position),
            "gradient stop position must be within [0, 1]"
        );
        assert!(
            color.red.is_finite(),
            "gradient stop red channel must be finite"
        );
        assert!(
            color.green.is_finite(),
            "gradient stop green channel must be finite"
        );
        assert!(
            color.blue.is_finite(),
            "gradient stop blue channel must be finite"
        );
        assert!(
            color.headroom.is_finite() && color.headroom >= 0.0,
            "gradient stop headroom must be finite and >= 0"
        );
        assert!(
            color.opacity.is_finite() && (0.0..=1.0).contains(&color.opacity),
            "gradient stop opacity must be finite and within [0, 1]"
        );
        Self { position, color }
    }
}

/// Resolved gradient payload rendered by backend-native engines.
#[derive(Debug, Clone)]
pub struct ResolvedGradient {
    /// Gradient kind.
    pub gradient_type: GradientType,
    /// Gradient stops.
    pub stops: Vec<ResolvedGradientStop>,
    /// Start point (linear) or center (radial/angular).
    pub start_point: [f32; 2],
    /// End point (linear).
    pub end_point: [f32; 2],
    /// Start radius (radial) or start angle (angular).
    pub start_value: f32,
    /// End radius (radial) or end angle (angular).
    pub end_value: f32,
}

impl ResolvedGradient {
    fn validate_stops(stops: &[ResolvedGradientStop]) {
        assert!(
            !stops.is_empty(),
            "resolved gradient must contain at least one stop"
        );

        let mut prev = f32::NEG_INFINITY;
        for stop in stops {
            assert!(
                stop.position > prev,
                "gradient stops must be strictly increasing by position"
            );
            prev = stop.position;
        }
    }

    fn validate_point(point: [f32; 2], name: &str) {
        assert!(point[0].is_finite(), "{name}.x must be finite");
        assert!(point[1].is_finite(), "{name}.y must be finite");
    }

    /// Creates a linear gradient.
    #[must_use]
    pub fn linear(stops: Vec<ResolvedGradientStop>, start: [f32; 2], end: [f32; 2]) -> Self {
        Self::validate_stops(&stops);
        Self::validate_point(start, "linear gradient start_point");
        Self::validate_point(end, "linear gradient end_point");
        Self {
            gradient_type: GradientType::Linear,
            stops,
            start_point: start,
            end_point: end,
            start_value: 0.0,
            end_value: 1.0,
        }
    }

    /// Creates a radial gradient.
    ///
    /// # Panics
    ///
    /// Panics when the center is invalid or the radii violate the required bounds.
    #[must_use]
    pub fn radial(
        stops: Vec<ResolvedGradientStop>,
        center: [f32; 2],
        start_radius: f32,
        end_radius: f32,
    ) -> Self {
        Self::validate_stops(&stops);
        Self::validate_point(center, "radial gradient center");
        assert!(
            start_radius.is_finite() && start_radius >= 0.0,
            "radial gradient start radius must be finite and >= 0"
        );
        assert!(
            end_radius.is_finite() && end_radius > 0.0,
            "radial gradient end radius must be finite and > 0"
        );
        Self {
            gradient_type: GradientType::Radial,
            stops,
            start_point: center,
            end_point: center,
            start_value: start_radius,
            end_value: end_radius,
        }
    }

    /// Creates an angular gradient.
    ///
    /// # Panics
    ///
    /// Panics when the center is invalid or the angle sweep is not finite and positive.
    #[must_use]
    pub fn angular(
        stops: Vec<ResolvedGradientStop>,
        center: [f32; 2],
        start_angle: f32,
        end_angle: f32,
    ) -> Self {
        Self::validate_stops(&stops);
        Self::validate_point(center, "angular gradient center");
        assert!(
            start_angle.is_finite(),
            "angular gradient start angle must be finite"
        );
        assert!(
            end_angle.is_finite(),
            "angular gradient end angle must be finite"
        );
        let sweep = end_angle - start_angle;
        assert!(sweep > 0.0, "angular gradient sweep must be positive");
        assert!(
            sweep <= core::f32::consts::TAU,
            "angular gradient sweep must be <= TAU"
        );
        Self {
            gradient_type: GradientType::Angular,
            stops,
            start_point: center,
            end_point: center,
            start_value: start_angle,
            end_value: end_angle,
        }
    }
}

// Linear/radial/angular gradients are lightweight native-rendered primitives.
waterui_core::raw_view!(ResolvedGradient, waterui_core::layout::StretchAxis::Both);

/// Configuration for creating a gradient view.
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
                (
                    0.0,
                    ResolvedColor {
                        red: 1.0,
                        green: 0.0,
                        blue: 0.0,
                        opacity: 1.0,
                        headroom: 0.0,
                    },
                ),
                (
                    1.0,
                    ResolvedColor {
                        red: 0.0,
                        green: 0.0,
                        blue: 1.0,
                        opacity: 1.0,
                        headroom: 0.0,
                    },
                ),
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
    #[must_use]
    pub fn linear(stops: Vec<(f32, ResolvedColor)>, start: [f32; 2], end: [f32; 2]) -> Self {
        Self {
            gradient_type: GradientType::Linear,
            stops,
            start_point: start,
            end_point: end,
            ..Default::default()
        }
    }

    /// Creates a radial gradient configuration.
    #[must_use]
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
    #[must_use]
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
    ///
    /// # Panics
    ///
    /// Panics when `vertices.len() != width * height`.
    #[must_use]
    pub fn mesh(
        width: u32,
        height: u32,
        vertices: Vec<([f32; 2], ResolvedColor)>,
        smooths_colors: bool,
    ) -> Self {
        assert_eq!(
            vertices.len(),
            (width * height) as usize,
            "mesh gradients require exactly width*height vertices"
        );
        Self {
            gradient_type: GradientType::Mesh,
            stops: Vec::new(),
            mesh_size: (width, height),
            mesh_vertices: vertices,
            smooths_colors,
            ..Default::default()
        }
    }

    fn into_resolved_gradient(self) -> ResolvedGradient {
        assert!(
            !(self.gradient_type == GradientType::Mesh),
            "mesh gradients must use MeshGradient/GPU path, not ResolvedGradient"
        );

        let mut stops = self
            .stops
            .into_iter()
            .map(|(position, color)| ResolvedGradientStop::new(position, color))
            .collect::<Vec<_>>();
        stops.sort_by(|a, b| a.position.total_cmp(&b.position));

        match self.gradient_type {
            GradientType::Linear => {
                ResolvedGradient::linear(stops, self.start_point, self.end_point)
            }
            GradientType::Radial => {
                ResolvedGradient::radial(stops, self.start_point, self.start_value, self.end_value)
            }
            GradientType::Angular => {
                ResolvedGradient::angular(stops, self.start_point, self.start_value, self.end_value)
            }
            GradientType::Mesh => panic!("mesh gradients must use MeshGradient/GPU path"),
        }
    }
}

/// User-facing gradient view.
///
/// - Linear/radial/angular gradients resolve to `ResolvedGradient` raw views.
/// - Mesh gradients remain GPU-rendered.
#[derive(Debug, Clone)]
pub struct Gradient {
    config: GradientConfig,
}

impl Gradient {
    /// Creates a gradient from config.
    #[must_use]
    pub const fn new(config: GradientConfig) -> Self {
        Self { config }
    }

    /// Creates a linear gradient view.
    #[must_use]
    pub fn linear(stops: Vec<(f32, ResolvedColor)>, start: [f32; 2], end: [f32; 2]) -> Self {
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
        Self::new(GradientConfig::radial(
            stops,
            center,
            start_radius,
            end_radius,
        ))
    }

    /// Creates an angular gradient view.
    #[must_use]
    pub fn angular(
        stops: Vec<(f32, ResolvedColor)>,
        center: [f32; 2],
        start_angle: f32,
        end_angle: f32,
    ) -> Self {
        Self::new(GradientConfig::angular(
            stops,
            center,
            start_angle,
            end_angle,
        ))
    }

    /// Creates a static mesh gradient view.
    #[must_use]
    pub fn mesh(
        width: u32,
        height: u32,
        vertices: Vec<([f32; 2], ResolvedColor)>,
        smooths_colors: bool,
    ) -> Self {
        Self::new(GradientConfig::mesh(
            width,
            height,
            vertices,
            smooths_colors,
        ))
    }
}

impl View for Gradient {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        let config = self.config;
        match config.gradient_type {
            GradientType::Mesh => AnyView::new(GpuSurface::new(StaticMeshRenderer::new(
                config.mesh_size.0,
                config.mesh_size.1,
                config.mesh_vertices,
                config.smooths_colors,
            ))),
            GradientType::Linear | GradientType::Radial | GradientType::Angular => {
                AnyView::new(config.into_resolved_gradient())
            }
        }
    }
}

mod shader_types {
    #![allow(dead_code)]

    use encase::ShaderType;

    /// A resolved color stop ready for GPU upload.
    #[derive(Debug, Clone, Copy, Default, ShaderType)]
    pub(super) struct GpuColorStop {
        /// RGBA color in linear space.
        pub(super) color: glam::Vec4,
        /// Position along the gradient (0.0 to 1.0).
        pub(super) position: f32,
    }

    /// A resolved mesh vertex ready for GPU upload.
    #[derive(Debug, Clone, Copy, Default, ShaderType)]
    pub(super) struct GpuMeshVertex {
        /// Position in unit coordinates (0.0 to 1.0).
        pub(super) position: glam::Vec2,
        /// RGBA color in linear space.
        pub(super) color: glam::Vec4,
    }

    /// Uniform buffer layout for mesh gradient parameters.
    #[derive(Debug, Clone, Copy, Default, ShaderType)]
    pub(super) struct GradientUniforms {
        pub(super) gradient_type: u32,
        pub(super) num_stops: u32,
        pub(super) mesh_width: u32,
        pub(super) mesh_height: u32,
        pub(super) start_point: glam::Vec2,
        pub(super) end_point: glam::Vec2,
        pub(super) start_value: f32,
        pub(super) end_value: f32,
        pub(super) smooths_colors: u32,
    }
}

use shader_types::{GpuColorStop, GpuMeshVertex, GradientUniforms};

struct MeshGpuResources {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    stops_buffer: wgpu::Buffer,
    mesh_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    pipeline_format: wgpu::TextureFormat,
}

#[allow(clippy::too_many_lines)]
fn create_mesh_resources(ctx: &GpuContext<'_>, label_prefix: &str) -> MeshGpuResources {
    let shader = crate::shared_context::create_cached_shader_module_prewarmed(
        ctx.device,
        &MESH_GRADIENT_SHADER,
    );

    let uniform_size = <GradientUniforms as ShaderSize>::SHADER_SIZE.get();
    let uniform_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&format!("{label_prefix} Uniforms")),
        size: uniform_size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let stop_size = <GpuColorStop as ShaderSize>::SHADER_SIZE.get();
    let stops_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&format!("{label_prefix} Color Stops")),
        size: stop_size * MAX_COLOR_STOPS as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let vertex_size = <GpuMeshVertex as ShaderSize>::SHADER_SIZE.get();
    let mesh_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(&format!("{label_prefix} Vertices")),
        size: vertex_size * MAX_MESH_VERTICES as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let bind_group_layout = ctx
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(&format!("{label_prefix} Bind Group Layout")),
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
        label: Some(&format!("{label_prefix} Bind Group")),
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

    let pipeline_layout = ctx
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&format!("{label_prefix} Pipeline Layout")),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

    let blend = if ctx.is_hdr() {
        None
    } else {
        Some(wgpu::BlendState::ALPHA_BLENDING)
    };

    ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let pipeline = ctx
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&format!("{label_prefix} Pipeline")),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader.as_ref(),
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: shader.as_ref(),
                entry_point: Some("fs_main"),
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
            multiview: None,
            cache: ctx.pipeline_cache,
        });

    let pipeline_error = crate::pop_error_scope_now(
        ctx.device,
        "gradient_renderer::create_mesh_pipeline::validation_error_scope",
    );
    assert!(
        pipeline_error.is_none(),
        "mesh gradient pipeline creation failed: {pipeline_error:?}"
    );

    MeshGpuResources {
        pipeline,
        uniform_buffer,
        stops_buffer,
        mesh_buffer,
        bind_group,
        pipeline_format: ctx.surface_format,
    }
}

fn write_mesh_data(
    frame: &GpuFrame,
    resources: &MeshGpuResources,
    width: u32,
    height: u32,
    smooths_colors: bool,
    vertices: &[GpuMeshVertex],
) {
    let uniforms = GradientUniforms {
        gradient_type: GradientType::Mesh as u32,
        num_stops: 0,
        mesh_width: width,
        mesh_height: height,
        start_point: glam::Vec2::new(0.0, 0.0),
        end_point: glam::Vec2::new(1.0, 1.0),
        start_value: 0.0,
        end_value: 1.0,
        smooths_colors: u32::from(smooths_colors),
    };

    let mut uniform_data = UniformBuffer::new(Vec::new());
    uniform_data
        .write(&uniforms)
        .expect("failed to encode mesh uniform buffer");
    frame
        .queue
        .write_buffer(&resources.uniform_buffer, 0, uniform_data.as_ref());

    let stops = vec![GpuColorStop::default(); MAX_COLOR_STOPS];
    let mut stops_data = StorageBuffer::new(Vec::new());
    stops_data
        .write(&stops)
        .expect("failed to encode mesh stops buffer");
    frame
        .queue
        .write_buffer(&resources.stops_buffer, 0, stops_data.as_ref());

    let mut padded = Vec::with_capacity(MAX_MESH_VERTICES);
    padded.extend(vertices.iter().copied().take(MAX_MESH_VERTICES));
    while padded.len() < MAX_MESH_VERTICES {
        padded.push(GpuMeshVertex::default());
    }

    let mut mesh_data = StorageBuffer::new(Vec::new());
    mesh_data
        .write(&padded)
        .expect("failed to encode mesh vertex buffer");
    frame
        .queue
        .write_buffer(&resources.mesh_buffer, 0, mesh_data.as_ref());
}

fn draw_mesh(frame: &mut GpuFrame, resources: &MeshGpuResources, label_prefix: &str) {
    let mut encoder = frame
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some(&format!("{label_prefix} Encoder")),
        });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(&format!("{label_prefix} Render Pass")),
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
        });

        render_pass.set_pipeline(&resources.pipeline);
        render_pass.set_bind_group(0, &resources.bind_group, &[]);
        render_pass.draw(0..6, 0..1);
    }

    frame.queue.submit(core::iter::once(encoder.finish()));
}

struct StaticMeshRenderer {
    width: u32,
    height: u32,
    smooths_colors: bool,
    vertices: Vec<GpuMeshVertex>,
    resources: Option<MeshGpuResources>,
    dirty: bool,
}

impl StaticMeshRenderer {
    fn new(
        width: u32,
        height: u32,
        vertices: Vec<([f32; 2], ResolvedColor)>,
        smooths_colors: bool,
    ) -> Self {
        assert_eq!(
            vertices.len(),
            (width * height) as usize,
            "mesh gradients require exactly width*height vertices"
        );
        let vertices = vertices
            .into_iter()
            .map(|(position, color)| GpuMeshVertex {
                position: glam::Vec2::new(position[0], position[1]),
                color: glam::Vec4::new(color.red, color.green, color.blue, color.opacity),
            })
            .collect();

        Self {
            width,
            height,
            smooths_colors,
            vertices,
            resources: None,
            dirty: true,
        }
    }
}

impl GpuView for StaticMeshRenderer {
    fn setup(
        &mut self,
        ctx: &GpuContext<'_>,
        _env: &mut waterui_core::Environment,
    ) -> impl core::future::Future<Output = ()> {
        self.resources = Some(create_mesh_resources(ctx, "Static Mesh Gradient"));
        core::future::ready(())
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        let resources = self
            .resources
            .as_ref()
            .expect("StaticMeshRenderer::setup() must run before render()");
        assert_eq!(
            resources.pipeline_format, frame.format,
            "mesh gradient format mismatch"
        );

        if self.dirty {
            write_mesh_data(
                frame,
                resources,
                self.width,
                self.height,
                self.smooths_colors,
                &self.vertices,
            );
            self.dirty = false;
        }

        draw_mesh(frame, resources, "Static Mesh Gradient");
    }
}

crate::impl_gpu_subview!(StaticMeshRenderer);

/// A mesh gradient that accepts reactive Signal parameters for animation.
pub struct MeshGradient<C> {
    width: u32,
    height: u32,
    colors: C,
    smooths_colors: bool,
}

impl<C> fmt::Debug for MeshGradient<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MeshGradient")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("smooths_colors", &self.smooths_colors)
            .finish_non_exhaustive()
    }
}

impl<C> MeshGradient<C> {
    /// Creates a new reactive mesh gradient.
    #[must_use]
    pub const fn new(width: u32, height: u32, colors: C) -> Self {
        Self {
            width,
            height,
            colors,
            smooths_colors: true,
        }
    }

    /// Sets whether to smooth color interpolation.
    #[must_use]
    pub const fn smooths_colors(mut self, smooths: bool) -> Self {
        self.smooths_colors = smooths;
        self
    }
}

struct ReactiveMeshRenderer<C> {
    width: u32,
    height: u32,
    colors: C,
    smooths_colors: bool,
    pending_update: Arc<AtomicBool>,
    watcher_guard: Option<Box<dyn Any>>,
    resources: Option<MeshGpuResources>,
    last_colors: Option<Vec<ResolvedColor>>,
}

impl<C> ReactiveMeshRenderer<C> {
    fn new(width: u32, height: u32, colors: C, smooths_colors: bool) -> Self {
        Self {
            width,
            height,
            colors,
            smooths_colors,
            pending_update: Arc::new(AtomicBool::new(false)),
            watcher_guard: None,
            resources: None,
            last_colors: None,
        }
    }
}

impl<C> GpuView for ReactiveMeshRenderer<C>
where
    C: Signal + 'static,
    C::Output: IntoIterator<Item = ResolvedColor>,
{
    fn setup(
        &mut self,
        ctx: &GpuContext<'_>,
        _env: &mut waterui_core::Environment,
    ) -> impl core::future::Future<Output = ()> {
        if self.watcher_guard.is_none() {
            let pending_update = Arc::clone(&self.pending_update);
            let redraw_handle = ctx.redraw_handle.clone();
            let guard = self.colors.watch(move |_context| {
                pending_update.store(true, Ordering::Release);
                redraw_handle.request_redraw();
            });
            self.watcher_guard = Some(Box::new(guard));
        }

        self.resources = Some(create_mesh_resources(ctx, "Reactive Mesh Gradient"));
        core::future::ready(())
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        let resources = self
            .resources
            .as_ref()
            .expect("ReactiveMeshRenderer::setup() must run before render()");
        assert_eq!(
            resources.pipeline_format, frame.format,
            "reactive mesh gradient format mismatch"
        );

        let pending_update = self.pending_update.swap(false, Ordering::AcqRel);

        if pending_update || self.last_colors.is_none() {
            let colors: Vec<ResolvedColor> = self.colors.get().into_iter().collect();

            let colors_changed = self.last_colors.as_ref().is_none_or(|last| {
                last.len() != colors.len()
                    || last.iter().zip(&colors).any(|(a, b)| !resolved_color_eq(a, b))
            });

            if colors_changed {
                self.last_colors = Some(colors.clone());

                let w = self.width as usize;
                let h = self.height as usize;
                let mut vertices = Vec::with_capacity(MAX_MESH_VERTICES);
                for (index, color) in colors.iter().take(MAX_MESH_VERTICES).enumerate() {
                    let x = usize_to_f32(index % w) / usize_to_f32((w - 1).max(1));
                    let y = usize_to_f32(index / w) / usize_to_f32((h - 1).max(1));
                    vertices.push(GpuMeshVertex {
                        position: glam::Vec2::new(x, y),
                        color: glam::Vec4::new(color.red, color.green, color.blue, color.opacity),
                    });
                }

                write_mesh_data(
                    frame,
                    resources,
                    self.width,
                    self.height,
                    self.smooths_colors,
                    &vertices,
                );
            }
        }

        draw_mesh(frame, resources, "Reactive Mesh Gradient");
    }
}

impl<C> waterui_core::layout::SubView for ReactiveMeshRenderer<C>
where
    C: Signal + 'static,
    C::Output: IntoIterator<Item = ResolvedColor>,
{
    fn measure(
        &self,
        proposal: waterui_core::layout::ProposalSize,
    ) -> waterui_core::layout::ViewDimensions {
        waterui_core::layout::ViewDimensions::new(waterui_core::layout::Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ))
    }

    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::Both
    }

    fn priority(&self) -> i32 {
        0
    }
}

impl<C> MeshGradient<C>
where
    C: Signal + 'static,
    C::Output: IntoIterator<Item = ResolvedColor>,
{
    /// Converts this mesh gradient into a GPU surface.
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

const fn resolved_color_eq(a: &ResolvedColor, b: &ResolvedColor) -> bool {
    a.red.to_bits() == b.red.to_bits()
        && a.green.to_bits() == b.green.to_bits()
        && a.blue.to_bits() == b.blue.to_bits()
        && a.opacity.to_bits() == b.opacity.to_bits()
        && a.headroom.to_bits() == b.headroom.to_bits()
}

fn usize_to_f32(value: usize) -> f32 {
    value
        .to_f32()
        .expect("gradient_renderer: index must be representable as f32")
}
