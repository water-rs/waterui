//! Shape system for WaterUI with HDR support.
//!
//! This module provides a trait-based system for defining shapes that can be used
//! for clipping views and as filled views. Uses Lyon for tessellation and custom
//! wgpu shaders for HDR-capable rendering.
//!
//! # Example
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//! use waterui::shape::*;
//!
//! // Clip to a circle
//! image("avatar.jpg").clip(Circle);
//!
//! // Fill a shape with HDR color
//! Circle.fill(Color::red().with_headroom(0.5))
//! ```

#![allow(clippy::multiple_crate_versions)]

extern crate alloc;

use core::f32::consts::{FRAC_PI_2, PI, TAU};

use nami::Signal;
use waterui_core::{metadata::MetadataKey, Environment, View};
use waterui_graphics::color::Color;
use waterui_graphics::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};

// ============================================================================
// PathCommand - The primitive operations for drawing paths
// ============================================================================

/// A single path command for drawing shapes.
///
/// All coordinates are normalized (0.0-1.0) and scale with view bounds.
/// Native backends convert these to absolute coordinates based on view size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCommand {
    /// Move to a position without drawing.
    MoveTo {
        /// X coordinate (normalized 0.0-1.0)
        x: f32,
        /// Y coordinate (normalized 0.0-1.0)
        y: f32,
    },

    /// Draw a straight line to a position.
    LineTo {
        /// X coordinate (normalized 0.0-1.0)
        x: f32,
        /// Y coordinate (normalized 0.0-1.0)
        y: f32,
    },

    /// Draw a quadratic bezier curve.
    QuadTo {
        /// Control point x
        cx: f32,
        /// Control point y
        cy: f32,
        /// End point x
        x: f32,
        /// End point y
        y: f32,
    },

    /// Draw a cubic bezier curve.
    CubicTo {
        /// First control point x
        c1x: f32,
        /// First control point y
        c1y: f32,
        /// Second control point x
        c2x: f32,
        /// Second control point y
        c2y: f32,
        /// End point x
        x: f32,
        /// End point y
        y: f32,
    },

    /// Draw an arc.
    Arc {
        /// Center x (normalized)
        cx: f32,
        /// Center y (normalized)
        cy: f32,
        /// Radius x (normalized, relative to width)
        rx: f32,
        /// Radius y (normalized, relative to height)
        ry: f32,
        /// Start angle in radians
        start: f32,
        /// Sweep angle in radians (positive = clockwise)
        sweep: f32,
    },

    /// Close the current subpath by drawing a line to the start.
    Close,
}

// ============================================================================
// Shape Trait
// ============================================================================

/// A trait for types that can produce path commands for clipping.
///
/// All coordinates are normalized (0.0-1.0) and scale with view bounds.
/// Built-in shapes use stack-allocated arrays for zero heap allocation.
pub trait Shape {
    /// The iterator type returned by `path()`.
    type Iter: IntoIterator<Item = PathCommand>;

    /// Returns the path commands that define this shape.
    fn path(&self) -> Self::Iter;
}

// ============================================================================
// Common Shape Implementations
// ============================================================================

/// A circle inscribed in the view bounds.
#[derive(Debug, Clone, Copy, Default)]
pub struct Circle;

impl Shape for Circle {
    type Iter = [PathCommand; 1];

    fn path(&self) -> Self::Iter {
        [PathCommand::Arc {
            cx: 0.5,
            cy: 0.5,
            rx: 0.5,
            ry: 0.5,
            start: 0.0,
            sweep: TAU,
        }]
    }
}

/// An ellipse that fills the view bounds.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ellipse;

impl Shape for Ellipse {
    type Iter = [PathCommand; 1];

    fn path(&self) -> Self::Iter {
        [PathCommand::Arc {
            cx: 0.5,
            cy: 0.5,
            rx: 0.5,
            ry: 0.5,
            start: 0.0,
            sweep: TAU,
        }]
    }
}

/// A capsule (pill) shape.
#[derive(Debug, Clone, Copy, Default)]
pub struct Capsule;

impl Shape for Capsule {
    type Iter = [PathCommand; 4];

    fn path(&self) -> Self::Iter {
        [
            PathCommand::MoveTo { x: 0.5, y: 0.0 },
            PathCommand::Arc {
                cx: 0.5,
                cy: 0.5,
                rx: 0.5,
                ry: 0.5,
                start: -FRAC_PI_2,
                sweep: PI,
            },
            PathCommand::Arc {
                cx: 0.5,
                cy: 0.5,
                rx: 0.5,
                ry: 0.5,
                start: FRAC_PI_2,
                sweep: PI,
            },
            PathCommand::Close,
        ]
    }
}

/// A rectangle with uniform corner radius.
#[derive(Debug, Clone, Copy)]
pub struct RoundedRectangle {
    /// Corner radius (normalized, 0.0-0.5 range).
    pub corner_radius: f32,
}

impl RoundedRectangle {
    /// Creates a new rounded rectangle with the given corner radius.
    #[must_use]
    pub const fn new(corner_radius: f32) -> Self {
        Self { corner_radius }
    }
}

impl Shape for RoundedRectangle {
    type Iter = [PathCommand; 10];

    fn path(&self) -> Self::Iter {
        let r = self.corner_radius;
        [
            PathCommand::MoveTo { x: r, y: 0.0 },
            PathCommand::LineTo { x: 1.0 - r, y: 0.0 },
            PathCommand::Arc {
                cx: 1.0 - r,
                cy: r,
                rx: r,
                ry: r,
                start: -FRAC_PI_2,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: 1.0, y: 1.0 - r },
            PathCommand::Arc {
                cx: 1.0 - r,
                cy: 1.0 - r,
                rx: r,
                ry: r,
                start: 0.0,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: r, y: 1.0 },
            PathCommand::Arc {
                cx: r,
                cy: 1.0 - r,
                rx: r,
                ry: r,
                start: FRAC_PI_2,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: 0.0, y: r },
            PathCommand::Arc {
                cx: r,
                cy: r,
                rx: r,
                ry: r,
                start: PI,
                sweep: FRAC_PI_2,
            },
            PathCommand::Close,
        ]
    }
}

/// A rectangle with independent corner radii.
#[derive(Debug, Clone, Copy)]
pub struct UnevenRoundedRectangle {
    /// Top-leading corner radius (normalized).
    pub top_leading: f32,
    /// Top-trailing corner radius (normalized).
    pub top_trailing: f32,
    /// Bottom-leading corner radius (normalized).
    pub bottom_leading: f32,
    /// Bottom-trailing corner radius (normalized).
    pub bottom_trailing: f32,
}

impl UnevenRoundedRectangle {
    /// Creates a new uneven rounded rectangle with independent corner radii.
    #[must_use]
    pub const fn new(
        top_leading: f32,
        top_trailing: f32,
        bottom_leading: f32,
        bottom_trailing: f32,
    ) -> Self {
        Self {
            top_leading,
            top_trailing,
            bottom_leading,
            bottom_trailing,
        }
    }
}

impl Shape for UnevenRoundedRectangle {
    type Iter = [PathCommand; 10];

    fn path(&self) -> Self::Iter {
        let tl = self.top_leading;
        let tr = self.top_trailing;
        let bl = self.bottom_leading;
        let br = self.bottom_trailing;
        [
            PathCommand::MoveTo { x: tl, y: 0.0 },
            PathCommand::LineTo { x: 1.0 - tr, y: 0.0 },
            PathCommand::Arc {
                cx: 1.0 - tr,
                cy: tr,
                rx: tr,
                ry: tr,
                start: -FRAC_PI_2,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: 1.0, y: 1.0 - br },
            PathCommand::Arc {
                cx: 1.0 - br,
                cy: 1.0 - br,
                rx: br,
                ry: br,
                start: 0.0,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: bl, y: 1.0 },
            PathCommand::Arc {
                cx: bl,
                cy: 1.0 - bl,
                rx: bl,
                ry: bl,
                start: FRAC_PI_2,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo { x: 0.0, y: tl },
            PathCommand::Arc {
                cx: tl,
                cy: tl,
                rx: tl,
                ry: tl,
                start: PI,
                sweep: FRAC_PI_2,
            },
            PathCommand::Close,
        ]
    }
}

/// A simple rectangle with sharp corners.
#[derive(Debug, Clone, Copy, Default)]
pub struct Rectangle;

impl Shape for Rectangle {
    type Iter = [PathCommand; 5];

    fn path(&self) -> Self::Iter {
        [
            PathCommand::MoveTo { x: 0.0, y: 0.0 },
            PathCommand::LineTo { x: 1.0, y: 0.0 },
            PathCommand::LineTo { x: 1.0, y: 1.0 },
            PathCommand::LineTo { x: 0.0, y: 1.0 },
            PathCommand::Close,
        ]
    }
}

// ============================================================================
// Custom Path Builder
// ============================================================================

/// A custom path defined by explicit commands.
#[derive(Debug, Clone, Default)]
pub struct Path {
    commands: Vec<PathCommand>,
}

impl Path {
    /// Creates a new empty path.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves to a position without drawing.
    #[must_use]
    pub fn move_to(mut self, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::MoveTo { x, y });
        self
    }

    /// Draws a straight line to a position.
    #[must_use]
    pub fn line_to(mut self, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::LineTo { x, y });
        self
    }

    /// Draws a quadratic bezier curve.
    #[must_use]
    pub fn quad_to(mut self, cx: f32, cy: f32, x: f32, y: f32) -> Self {
        self.commands.push(PathCommand::QuadTo { cx, cy, x, y });
        self
    }

    /// Draws a cubic bezier curve.
    #[must_use]
    pub fn cubic_to(mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) -> Self {
        self.commands
            .push(PathCommand::CubicTo { c1x, c1y, c2x, c2y, x, y });
        self
    }

    /// Draws an arc.
    #[must_use]
    pub fn arc(mut self, cx: f32, cy: f32, rx: f32, ry: f32, start: f32, sweep: f32) -> Self {
        self.commands.push(PathCommand::Arc {
            cx,
            cy,
            rx,
            ry,
            start,
            sweep,
        });
        self
    }

    /// Closes the current subpath.
    #[must_use]
    pub fn close(mut self) -> Self {
        self.commands.push(PathCommand::Close);
        self
    }
}

impl Shape for Path {
    type Iter = alloc::vec::IntoIter<PathCommand>;

    fn path(&self) -> Self::Iter {
        self.commands.clone().into_iter()
    }
}

// ============================================================================
// ClipShape Metadata
// ============================================================================

/// Metadata for clipping a view to a shape.
#[derive(Debug)]
pub struct ClipShape {
    commands: Vec<PathCommand>,
}

impl ClipShape {
    /// Creates a new clip shape from any type implementing Shape.
    pub fn new(shape: impl Shape) -> Self {
        Self {
            commands: shape.path().into_iter().collect(),
        }
    }

    /// Returns the path commands.
    #[must_use]
    pub fn commands(&self) -> &[PathCommand] {
        &self.commands
    }
}

impl MetadataKey for ClipShape {}

// ============================================================================
// ShapeKind - For GPU rendering optimization
// ============================================================================

/// The kind of shape for GPU rendering optimization.
#[derive(Debug, Clone, Copy, Default)]
pub enum ShapeKind {
    /// Rectangle with sharp corners.
    #[default]
    Rect,
    /// Circle inscribed in bounds.
    Circle,
    /// Ellipse filling bounds.
    Ellipse,
    /// Rectangle with uniform corner radius.
    RoundedRect {
        /// Corner radius (normalized 0.0-0.5).
        corner_radius: f32,
    },
    /// Rectangle with per-corner radii.
    UnevenRoundedRect {
        /// Top-left corner radius.
        top_left: f32,
        /// Top-right corner radius.
        top_right: f32,
        /// Bottom-left corner radius.
        bottom_left: f32,
        /// Bottom-right corner radius.
        bottom_right: f32,
    },
    /// Capsule (pill) shape.
    Capsule,
    /// Custom path.
    CustomPath,
}

// ============================================================================
// FilledShape - Shape as a View with fill color (Lyon + HDR)
// ============================================================================

/// A shape filled with a color, rendered via Lyon tessellation for HDR support.
#[derive(Debug)]
pub struct FilledShape {
    kind: ShapeKind,
    commands: Vec<PathCommand>,
    fill: Color,
}

impl FilledShape {
    /// Creates a new filled shape from a shape and color.
    pub fn new(shape: impl Shape, fill: impl Into<Color>) -> Self {
        Self {
            kind: ShapeKind::CustomPath,
            commands: shape.path().into_iter().collect(),
            fill: fill.into(),
        }
    }

    fn with_kind(kind: ShapeKind, shape: impl Shape, fill: impl Into<Color>) -> Self {
        Self {
            kind,
            commands: shape.path().into_iter().collect(),
            fill: fill.into(),
        }
    }

    /// Returns the path commands.
    #[must_use]
    pub fn commands(&self) -> &[PathCommand] {
        &self.commands
    }

    /// Returns the fill color.
    #[must_use]
    pub fn fill(&self) -> &Color {
        &self.fill
    }

    /// Returns the shape kind.
    #[must_use]
    pub fn kind(&self) -> ShapeKind {
        self.kind
    }
}

impl View for FilledShape {
    fn body(self, env: &Environment) -> impl View {
        let resolved = self.fill.resolve(env).get();
        GpuSurface::new(LyonShapeRenderer::new(self.kind, self.commands, resolved))
    }
}

// ============================================================================
// LyonShapeRenderer - Lyon tessellation + HDR GPU rendering
// ============================================================================

/// Vertex for shape rendering.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ShapeVertex {
    position: [f32; 2],
}

/// GPU renderer for shapes using Lyon tessellation.
struct LyonShapeRenderer {
    kind: ShapeKind,
    commands: Vec<PathCommand>,
    fill_color: waterui_graphics::ResolvedColor,
    pipeline: Option<wgpu::RenderPipeline>,
    vertex_buffer: Option<wgpu::Buffer>,
    index_buffer: Option<wgpu::Buffer>,
    uniform_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    num_indices: u32,
    cached_size: (u32, u32),
}

impl core::fmt::Debug for LyonShapeRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LyonShapeRenderer")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl LyonShapeRenderer {
    fn new(
        kind: ShapeKind,
        commands: Vec<PathCommand>,
        fill_color: waterui_graphics::ResolvedColor,
    ) -> Self {
        Self {
            kind,
            commands,
            fill_color,
            pipeline: None,
            vertex_buffer: None,
            index_buffer: None,
            uniform_buffer: None,
            bind_group: None,
            num_indices: 0,
            cached_size: (0, 0),
        }
    }

    /// Tessellates the shape using Lyon at the given dimensions.
    fn tessellate(&self, width: f32, height: f32) -> (Vec<ShapeVertex>, Vec<u32>) {
        use lyon::math::point;
        use lyon::path::Path as LyonPath;
        use lyon::tessellation::{
            BuffersBuilder, FillOptions, FillTessellator, FillVertex, VertexBuffers,
        };

        let mut builder = LyonPath::builder();
        let mut in_subpath = false;

        // Build Lyon path from commands
        for cmd in &self.commands {
            match *cmd {
                PathCommand::MoveTo { x, y } => {
                    if in_subpath {
                        builder.end(false);
                    }
                    builder.begin(point(x * width, y * height));
                    in_subpath = true;
                }
                PathCommand::LineTo { x, y } => {
                    if !in_subpath {
                        builder.begin(point(0.0, 0.0));
                        in_subpath = true;
                    }
                    builder.line_to(point(x * width, y * height));
                }
                PathCommand::QuadTo { cx, cy, x, y } => {
                    if !in_subpath {
                        builder.begin(point(0.0, 0.0));
                        in_subpath = true;
                    }
                    builder.quadratic_bezier_to(
                        point(cx * width, cy * height),
                        point(x * width, y * height),
                    );
                }
                PathCommand::CubicTo {
                    c1x,
                    c1y,
                    c2x,
                    c2y,
                    x,
                    y,
                } => {
                    if !in_subpath {
                        builder.begin(point(0.0, 0.0));
                        in_subpath = true;
                    }
                    builder.cubic_bezier_to(
                        point(c1x * width, c1y * height),
                        point(c2x * width, c2y * height),
                        point(x * width, y * height),
                    );
                }
                PathCommand::Arc {
                    cx,
                    cy,
                    rx,
                    ry,
                    start,
                    sweep,
                } => {
                    // Approximate arc with line segments for reliability
                    let center_x = cx * width;
                    let center_y = cy * height;
                    let radius_x = rx * width;
                    let radius_y = ry * height;

                    // Number of segments based on arc length
                    let segments = ((sweep.abs() / TAU) * 64.0).max(8.0) as usize;
                    let step = sweep / segments as f32;

                    for i in 0..=segments {
                        let angle = start + step * i as f32;
                        let px = center_x + radius_x * angle.cos();
                        let py = center_y + radius_y * angle.sin();

                        if i == 0 {
                            if !in_subpath {
                                builder.begin(point(px, py));
                                in_subpath = true;
                            } else {
                                builder.line_to(point(px, py));
                            }
                        } else {
                            builder.line_to(point(px, py));
                        }
                    }

                    // For a full circle, close the path
                    if sweep.abs() >= TAU - 0.01 {
                        builder.close();
                        in_subpath = false;
                    }
                }
                PathCommand::Close => {
                    if in_subpath {
                        builder.close();
                        in_subpath = false;
                    }
                }
            }
        }

        // End any unclosed subpath
        if in_subpath {
            builder.end(false);
        }

        let path = builder.build();

        // Tessellate
        let mut geometry: VertexBuffers<ShapeVertex, u32> = VertexBuffers::new();
        let mut tessellator = FillTessellator::new();

        if let Err(e) = tessellator.tessellate_path(
            &path,
            &FillOptions::tolerance(0.1),
            &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex| ShapeVertex {
                position: vertex.position().to_array(),
            }),
        ) {
            tracing::error!("[Shape] tessellation failed: {:?}", e);
        }

        (geometry.vertices, geometry.indices)
    }
}

impl GpuRenderer for LyonShapeRenderer {
    fn setup(&mut self, ctx: &GpuContext) {
        tracing::debug!("[Shape] setup called, format: {:?}", ctx.surface_format);

        // Create shader
        let shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Shape Shader"),
                source: wgpu::ShaderSource::Wgsl(SHAPE_SHADER.into()),
            });

        // Create uniform buffer for color
        let uniform_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shape Uniform Buffer"),
            size: 32, // vec4<f32> color + vec2<f32> size + padding
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Shape Bind Group Layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shape Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Shape Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Shape Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<ShapeVertex>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        }],
                    }],
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
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        self.pipeline = Some(pipeline);
        self.uniform_buffer = Some(uniform_buffer);
        self.bind_group = Some(bind_group);
    }

    fn resize(&mut self, _width: u32, _height: u32) {
        // Mark for re-tessellation
        self.cached_size = (0, 0);
    }

    fn render(&mut self, frame: &GpuFrame) {
        eprintln!("[Shape] render called, frame {}x{}", frame.width, frame.height);

        let Some(pipeline) = &self.pipeline else {
            tracing::warn!("[Shape] no pipeline");
            return;
        };
        let Some(uniform_buffer) = &self.uniform_buffer else {
            tracing::warn!("[Shape] no uniform buffer");
            return;
        };
        let Some(bind_group) = &self.bind_group else {
            tracing::warn!("[Shape] no bind group");
            return;
        };

        // Re-tessellate if size changed
        if self.cached_size != (frame.width, frame.height) {
            tracing::debug!(
                "[Shape] tessellating at {}x{}, commands: {:?}",
                frame.width,
                frame.height,
                self.commands
            );

            #[allow(clippy::cast_precision_loss)]
            let (vertices, indices) =
                self.tessellate(frame.width as f32, frame.height as f32);

            tracing::debug!(
                "[Shape] tessellated: {} vertices, {} indices",
                vertices.len(),
                indices.len()
            );

            if !vertices.is_empty() && !indices.is_empty() {
                use wgpu::util::DeviceExt;

                self.vertex_buffer =
                    Some(frame.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Shape Vertex Buffer"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }));

                self.index_buffer =
                    Some(frame.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Shape Index Buffer"),
                        contents: bytemuck::cast_slice(&indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }));

                #[allow(clippy::cast_possible_truncation)]
                {
                    self.num_indices = indices.len() as u32;
                }
            }

            self.cached_size = (frame.width, frame.height);
        }

        let Some(vertex_buffer) = &self.vertex_buffer else {
            tracing::warn!("[Shape] no vertex buffer - tessellation produced no vertices");
            return;
        };
        let Some(index_buffer) = &self.index_buffer else {
            tracing::warn!("[Shape] no index buffer");
            return;
        };

        if self.num_indices == 0 {
            tracing::warn!("[Shape] num_indices is 0");
            return;
        }

        tracing::warn!("[Shape] drawing {} indices", self.num_indices);

        // Update uniforms with HDR color
        let [r, g, b] = self.fill_color.linear_with_headroom();
        let opacity = self.fill_color.opacity;
        tracing::warn!(
            "[Shape] color: r={}, g={}, b={}, a={}, size={}x{}",
            r, g, b, opacity, frame.width, frame.height
        );
        #[allow(clippy::cast_precision_loss)]
        let uniforms = [
            r,
            g,
            b,
            opacity,
            frame.width as f32,
            frame.height as f32,
            0.0,
            0.0,
        ];
        frame
            .queue
            .write_buffer(uniform_buffer, 0, bytemuck::cast_slice(&uniforms));

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Shape Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shape Render Pass"),
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
            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..self.num_indices, 0, 0..1);
        }

        frame.queue.submit(std::iter::once(encoder.finish()));
    }
}

/// WGSL shader for HDR shape rendering
const SHAPE_SHADER: &str = r"
struct Uniforms {
    color: vec4<f32>,
    size: vec2<f32>,
    _padding: vec2<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_main(@location(0) position: vec2<f32>) -> VertexOutput {
    var out: VertexOutput;
    // Convert from pixel coordinates to clip space (-1 to 1)
    let x = (position.x / uniforms.size.x) * 2.0 - 1.0;
    let y = 1.0 - (position.y / uniforms.size.y) * 2.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Output HDR color directly (values > 1.0 are preserved)
    return uniforms.color;
}
";

// ============================================================================
// ShapeExt - Extension trait for adding fill to shapes
// ============================================================================

/// Extension trait for filling shapes with color.
pub trait ShapeExt: Shape + Sized {
    /// Returns the shape kind for GPU rendering optimization.
    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::CustomPath
    }

    /// Fills the shape with the specified color.
    fn fill(self, color: impl Into<Color>) -> FilledShape {
        FilledShape::with_kind(self.shape_kind(), self, color)
    }
}

impl ShapeExt for Circle {
    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::Circle
    }
}

impl ShapeExt for Ellipse {
    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::Ellipse
    }
}

impl ShapeExt for Capsule {
    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::Capsule
    }
}

impl ShapeExt for Rectangle {
    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::Rect
    }
}

impl ShapeExt for RoundedRectangle {
    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::RoundedRect {
            corner_radius: self.corner_radius,
        }
    }
}

impl ShapeExt for UnevenRoundedRectangle {
    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::UnevenRoundedRect {
            top_left: self.top_leading,
            top_right: self.top_trailing,
            bottom_left: self.bottom_leading,
            bottom_right: self.bottom_trailing,
        }
    }
}

impl ShapeExt for Path {
    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::CustomPath
    }
}
