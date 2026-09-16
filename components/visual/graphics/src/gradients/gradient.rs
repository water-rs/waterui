//! Gradient primitives resolved into `ResolvedGradient` raw views so backends
//! render them natively.
//!
//! Linear/radial/angular gradients are lightweight native-rendered primitives
//! and live here unconditionally. Mesh gradients are GPU-backed (`GpuView`)
//! because they require custom interpolation in shader space; the `mesh`
//! constructor and the mesh arm of `body` are compiled only under `gpu`.

extern crate alloc;

use alloc::vec::Vec;

use crate::color::ResolvedColor;
#[cfg(feature = "gpu")]
use crate::gpu_surface::GpuSurface;
#[cfg(feature = "gpu")]
use crate::gradients::gradient_renderer::StaticMeshRenderer;
use waterui_core::{AnyView, View};

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
    /// Mesh gradients are GPU-rendered; this constructor is only available with
    /// the `gpu` feature.
    ///
    /// # Panics
    ///
    /// Panics when `vertices.len() != width * height`.
    #[cfg(feature = "gpu")]
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
    ///
    /// Mesh gradients are GPU-rendered; this constructor is only available with
    /// the `gpu` feature.
    #[cfg(feature = "gpu")]
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
            #[cfg(feature = "gpu")]
            GradientType::Mesh => AnyView::new(GpuSurface::new(StaticMeshRenderer::new(
                config.mesh_size.0,
                config.mesh_size.1,
                config.mesh_vertices,
                config.smooths_colors,
            ))),
            #[cfg(not(feature = "gpu"))]
            GradientType::Mesh => {
                panic!("mesh gradients require the `gpu` feature")
            }
            GradientType::Linear | GradientType::Radial | GradientType::Angular => {
                AnyView::new(config.into_resolved_gradient())
            }
        }
    }

    /// Every branch resolves to a both-axes leaf: `GpuSurface` for mesh
    /// gradients, `ResolvedGradient` (declared `Both`) for the rest.
    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::Both
    }
}
