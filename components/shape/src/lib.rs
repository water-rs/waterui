//! Shape system for WaterUI with HDR support.
//!
//! This module provides a trait-based system for defining shapes that can be used
//! for clipping views and as filled views.
//!
//! Filled shapes are emitted as native `ResolvedShape` raw views so each backend
//! renders paths with its own 2D engine. Morphing shapes stay GPU-backed.
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
use core::time::Duration;

use nami::{Computed, Signal, signal::IntoComputed};
use waterui_core::{Environment, View, easing::EasingCurve, metadata::MetadataKey};
use waterui_graphics::color::Color;
use waterui_graphics::{GpuContext, GpuFrame, GpuSurface, GpuView, impl_gpu_subview};

const MORPH_SHADER_LABEL: &str = "shaders/morph.wgsl";
const MORPH_SHADER_SOURCE: &str = include_str!("shaders/morph.wgsl");

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

#[inline]
fn clamp_radius(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 0.5)
    } else {
        0.0
    }
}

#[derive(Debug, Clone, Copy)]
struct CornerRadii {
    top_left: f32,
    top_right: f32,
    bottom_right: f32,
    bottom_left: f32,
}

impl CornerRadii {
    #[inline]
    fn sanitized(mut self) -> Self {
        self.top_left = clamp_radius(self.top_left);
        self.top_right = clamp_radius(self.top_right);
        self.bottom_right = clamp_radius(self.bottom_right);
        self.bottom_left = clamp_radius(self.bottom_left);

        // Prevent overlapping corner arcs (same behavior as CSS border-radius normalization).
        let mut scale = 1.0f32;
        let pairs = [
            self.top_left + self.top_right,
            self.bottom_left + self.bottom_right,
            self.top_left + self.bottom_left,
            self.top_right + self.bottom_right,
        ];
        for sum in pairs {
            if sum > 1.0 {
                scale = scale.min(1.0 / sum);
            }
        }
        if scale < 1.0 {
            self.top_left *= scale;
            self.top_right *= scale;
            self.bottom_right *= scale;
            self.bottom_left *= scale;
        }
        self
    }
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
        let r = CornerRadii {
            top_left: self.corner_radius,
            top_right: self.corner_radius,
            bottom_right: self.corner_radius,
            bottom_left: self.corner_radius,
        }
        .sanitized()
        .top_left;
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
        let corners = CornerRadii {
            top_left: self.top_leading,
            top_right: self.top_trailing,
            bottom_right: self.bottom_trailing,
            bottom_left: self.bottom_leading,
        }
        .sanitized();
        let tl = corners.top_left;
        let tr = corners.top_right;
        let bl = corners.bottom_left;
        let br = corners.bottom_right;
        [
            PathCommand::MoveTo { x: tl, y: 0.0 },
            PathCommand::LineTo {
                x: 1.0 - tr,
                y: 0.0,
            },
            PathCommand::Arc {
                cx: 1.0 - tr,
                cy: tr,
                rx: tr,
                ry: tr,
                start: -FRAC_PI_2,
                sweep: FRAC_PI_2,
            },
            PathCommand::LineTo {
                x: 1.0,
                y: 1.0 - br,
            },
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
        self.commands.push(PathCommand::CubicTo {
            c1x,
            c1y,
            c2x,
            c2y,
            x,
            y,
        });
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
// ShapeKind - For backend rendering optimization
// ============================================================================

/// The kind of shape for backend rendering optimization.
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

/// Resolved shape payload rendered directly by native backends.
#[derive(Debug, Clone)]
pub struct ResolvedShape {
    /// Shape kind for backend-side optimization.
    pub kind: ShapeKind,
    /// Path commands in unit coordinate space.
    pub commands: Vec<PathCommand>,
    /// Resolved fill color.
    pub fill: waterui_graphics::ResolvedColor,
}

waterui_core::raw_view!(ResolvedShape, waterui_core::layout::StretchAxis::Both);

// ============================================================================
// FilledShape - Shape as a View with backend-native fill rendering
// ============================================================================

/// A shape filled with a color, resolved to `ResolvedShape`.
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

    /// Creates a morphing shape animation from this shape to another built-in shape.
    ///
    /// Morphing currently supports SDF-backed built-in shapes:
    /// `Rectangle`, `Circle`, `Ellipse`, `RoundedRectangle`, `UnevenRoundedRectangle`, `Capsule`.
    #[must_use]
    pub fn morph_to(self, target: impl ShapeExt) -> MorphShape {
        MorphShape::new(self.kind, target.shape_kind(), self.fill)
    }
}

/// Configuration for shape morph animations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MorphAnimation {
    /// Duration of one forward morph cycle.
    pub duration: Duration,
    /// Easing curve applied to normalized cycle progress.
    pub easing: EasingCurve,
    /// Whether the animation repeats after reaching the end.
    pub repeat: bool,
    /// Whether repeating animation should play in reverse every other cycle.
    pub autoreverse: bool,
}

impl Default for MorphAnimation {
    fn default() -> Self {
        Self {
            duration: Duration::from_millis(900),
            easing: EasingCurve::EASE_IN_OUT,
            repeat: true,
            autoreverse: true,
        }
    }
}

impl MorphAnimation {
    /// Creates a one-shot morph animation.
    #[must_use]
    pub fn once(duration: Duration, easing: EasingCurve) -> Self {
        Self {
            duration,
            easing,
            repeat: false,
            autoreverse: false,
        }
    }

    #[must_use]
    fn sample(self, elapsed: Duration) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        let raw = elapsed.as_secs_f32() / self.duration.as_secs_f32();
        let cycle = if self.repeat {
            let base = raw.fract();
            let index = raw.floor() as u64;
            if self.autoreverse && index % 2 == 1 {
                1.0 - base
            } else {
                base
            }
        } else {
            raw.clamp(0.0, 1.0)
        };
        self.easing.ease(cycle).clamp(0.0, 1.0)
    }
}

/// A morphing filled shape view.
#[derive(Debug, Clone)]
pub struct MorphShape {
    from: ShapeKind,
    to: ShapeKind,
    fill: Color,
    animation: MorphAnimation,
    progress: Option<Computed<f32>>,
}

impl MorphShape {
    fn new(from: ShapeKind, to: ShapeKind, fill: Color) -> Self {
        Self {
            from,
            to,
            fill,
            animation: MorphAnimation::default(),
            progress: None,
        }
    }

    /// Sets explicit animation configuration.
    #[must_use]
    pub const fn animation(mut self, animation: MorphAnimation) -> Self {
        self.animation = animation;
        self
    }

    /// Sets the cycle duration (keeps other animation options unchanged).
    #[must_use]
    pub const fn duration(mut self, duration: Duration) -> Self {
        self.animation.duration = duration;
        self
    }

    /// Sets easing (keeps other animation options unchanged).
    #[must_use]
    pub const fn easing(mut self, easing: EasingCurve) -> Self {
        self.animation.easing = easing;
        self
    }

    /// Enables/disables repeating.
    #[must_use]
    pub const fn repeat(mut self, repeat: bool) -> Self {
        self.animation.repeat = repeat;
        self
    }

    /// Enables/disables autoreverse for repeating animations.
    #[must_use]
    pub const fn autoreverse(mut self, autoreverse: bool) -> Self {
        self.animation.autoreverse = autoreverse;
        self
    }

    /// Overrides animated progress with an explicit reactive progress signal `[0, 1]`.
    ///
    /// When set, this takes precedence over the time-based animation config.
    #[must_use]
    pub fn progress(mut self, progress: impl IntoComputed<f32>) -> Self {
        self.progress = Some(progress.into_computed());
        self
    }
}

impl View for FilledShape {
    fn body(self, env: &Environment) -> impl View {
        ResolvedShape {
            kind: self.kind,
            commands: self.commands,
            fill: self.fill.resolve(env).get(),
        }
    }
}

impl View for MorphShape {
    fn body(self, env: &Environment) -> impl View {
        let resolved = self.fill.resolve(env).get();
        let from = kind_to_morph_shape(self.from)
            .expect("morph source shape must be a built-in morphable shape");
        let to = kind_to_morph_shape(self.to)
            .expect("morph target shape must be a built-in morphable shape");

        GpuSurface::new(MorphShapeRenderer::new(
            from,
            to,
            resolved,
            self.animation,
            self.progress,
        ))
    }
}

// ============================================================================
// MorphShapeRenderer - SDF morphing for built-in shapes
// ============================================================================

#[derive(Debug, Clone, Copy)]
struct MorphSdfShape {
    shape_type: u32,
    radii: [f32; 4],
}

fn kind_to_morph_shape(kind: ShapeKind) -> Option<MorphSdfShape> {
    match kind {
        ShapeKind::Rect => Some(MorphSdfShape {
            shape_type: 0,
            radii: [0.0; 4],
        }),
        ShapeKind::Circle => Some(MorphSdfShape {
            shape_type: 1,
            radii: [0.0; 4],
        }),
        ShapeKind::Ellipse => Some(MorphSdfShape {
            shape_type: 2,
            radii: [0.0; 4],
        }),
        ShapeKind::RoundedRect { corner_radius } => Some(MorphSdfShape {
            shape_type: 3,
            radii: [clamp_radius(corner_radius); 4],
        }),
        ShapeKind::UnevenRoundedRect {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        } => {
            let corners = CornerRadii {
                top_left,
                top_right,
                bottom_right,
                bottom_left,
            }
            .sanitized();
            Some(MorphSdfShape {
                shape_type: 3,
                radii: [
                    corners.top_left,
                    corners.top_right,
                    corners.bottom_right,
                    corners.bottom_left,
                ],
            })
        }
        ShapeKind::Capsule => Some(MorphSdfShape {
            shape_type: 4,
            radii: [0.0; 4],
        }),
        ShapeKind::CustomPath => None,
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct MorphUniforms {
    color: [f32; 4],
    dimensions_and_progress: [f32; 4], // width, height, progress, pad
    shape_types: [f32; 4],             // from_type, to_type, pad, pad
    from_radii: [f32; 4],              // tl, tr, br, bl
    to_radii: [f32; 4],                // tl, tr, br, bl
}

struct MorphShapeRenderer {
    from: MorphSdfShape,
    to: MorphSdfShape,
    fill_color: waterui_graphics::ResolvedColor,
    animation: MorphAnimation,
    progress: Option<Computed<f32>>,
    start_time: std::time::Instant,
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    pipeline_format: Option<wgpu::TextureFormat>,
}

impl core::fmt::Debug for MorphShapeRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MorphShapeRenderer")
            .field("from", &self.from)
            .field("to", &self.to)
            .finish_non_exhaustive()
    }
}

impl MorphShapeRenderer {
    fn new(
        from: MorphSdfShape,
        to: MorphSdfShape,
        fill_color: waterui_graphics::ResolvedColor,
        animation: MorphAnimation,
        progress: Option<Computed<f32>>,
    ) -> Self {
        Self {
            from,
            to,
            fill_color,
            animation,
            progress,
            start_time: std::time::Instant::now(),
            pipeline: None,
            uniform_buffer: None,
            bind_group: None,
            pipeline_format: None,
        }
    }
}

impl GpuView for MorphShapeRenderer {
    async fn setup(&mut self, ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
        let shader = waterui_graphics::shared_context::create_cached_shader_module(
            ctx.device,
            MORPH_SHADER_LABEL,
            MORPH_SHADER_SOURCE,
        );

        let uniform_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Morph Shape Uniforms"),
            size: core::mem::size_of::<MorphUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Morph Shape Bind Group Layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Morph Shape Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Morph Shape Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let blend = if ctx.is_hdr() {
            None
        } else {
            Some(wgpu::BlendState::ALPHA_BLENDING)
        };

        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Morph Shape Pipeline"),
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

        self.pipeline = Some(pipeline);
        self.uniform_buffer = Some(uniform_buffer);
        self.bind_group = Some(bind_group);
        self.pipeline_format = Some(ctx.surface_format);
        self.start_time = std::time::Instant::now();
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        if let Some(target_fmt) = self.pipeline_format {
            if target_fmt != frame.format {
                return;
            }
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

        let progress = if let Some(signal) = &self.progress {
            let value = signal.get();
            if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            }
        } else {
            self.animation.sample(self.start_time.elapsed())
        };

        let [r, g, b] = self.fill_color.linear_with_headroom();
        let uniforms = MorphUniforms {
            color: [r, g, b, self.fill_color.opacity],
            dimensions_and_progress: [frame.width as f32, frame.height as f32, progress, 0.0],
            shape_types: [
                self.from.shape_type as f32,
                self.to.shape_type as f32,
                0.0,
                0.0,
            ],
            from_radii: self.from.radii,
            to_radii: self.to.radii,
        };
        frame
            .queue
            .write_buffer(uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Morph Shape Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Morph Shape Render Pass"),
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

            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(0, bind_group, &[]);
            render_pass.draw(0..6, 0..1);
        }

        frame.queue.submit(core::iter::once(encoder.finish()));

        // Request continuous redraw while animation is active
        let animation_active = self.progress.is_some()
            || self.animation.repeat
            || self.start_time.elapsed() < self.animation.duration;
        if animation_active {
            frame.request_redraw();
        }
    }
}

impl_gpu_subview!(MorphShapeRenderer);

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

    /// Creates a morphing filled shape from this shape to another built-in shape.
    ///
    /// Morphing currently supports SDF-backed built-in shapes:
    /// `Rectangle`, `Circle`, `Ellipse`, `RoundedRectangle`, `UnevenRoundedRectangle`, `Capsule`.
    fn morph_to(self, target: impl ShapeExt, fill: impl Into<Color>) -> MorphShape {
        MorphShape::new(self.shape_kind(), target.shape_kind(), fill.into())
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
        let r = CornerRadii {
            top_left: self.corner_radius,
            top_right: self.corner_radius,
            bottom_right: self.corner_radius,
            bottom_left: self.corner_radius,
        }
        .sanitized()
        .top_left;
        ShapeKind::RoundedRect { corner_radius: r }
    }
}

impl ShapeExt for UnevenRoundedRectangle {
    fn shape_kind(&self) -> ShapeKind {
        let corners = CornerRadii {
            top_left: self.top_leading,
            top_right: self.top_trailing,
            bottom_right: self.bottom_trailing,
            bottom_left: self.bottom_leading,
        }
        .sanitized();
        ShapeKind::UnevenRoundedRect {
            top_left: corners.top_left,
            top_right: corners.top_right,
            bottom_left: corners.bottom_left,
            bottom_right: corners.bottom_right,
        }
    }
}

impl ShapeExt for Path {
    fn shape_kind(&self) -> ShapeKind {
        ShapeKind::CustomPath
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounded_rectangle_radius_is_clamped() {
        let kind = RoundedRectangle::new(9.0).shape_kind();
        match kind {
            ShapeKind::RoundedRect { corner_radius } => {
                assert!((corner_radius - 0.5).abs() < 1e-6);
            }
            _ => panic!("unexpected kind"),
        }
    }

    #[test]
    fn uneven_radii_are_normalized_when_edges_overlap() {
        let kind = UnevenRoundedRectangle::new(0.8, 0.8, 0.8, 0.8).shape_kind();
        match kind {
            ShapeKind::UnevenRoundedRect {
                top_left,
                top_right,
                bottom_left,
                bottom_right,
            } => {
                assert!((top_left - 0.5).abs() < 1e-6);
                assert!((top_right - 0.5).abs() < 1e-6);
                assert!((bottom_left - 0.5).abs() < 1e-6);
                assert!((bottom_right - 0.5).abs() < 1e-6);
            }
            _ => panic!("unexpected kind"),
        }
    }

    #[test]
    fn one_shot_animation_reaches_end() {
        let animation = MorphAnimation::once(Duration::from_millis(200), EasingCurve::LINEAR);
        assert!((animation.sample(Duration::ZERO) - 0.0).abs() < 1e-6);
        assert!((animation.sample(Duration::from_millis(100)) - 0.5).abs() < 1e-3);
        assert!((animation.sample(Duration::from_secs(1)) - 1.0).abs() < 1e-6);
    }
}
