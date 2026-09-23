//! Gradient definitions for backgrounds.
//!
//! This module provides gradient types that can be used as backgrounds via the
//! `.background()` modifier. All gradients are rendered using GPU shaders for
//! consistent cross-platform appearance and HDR support.
//!
//! # Gradient Types
//!
//! - [`LinearGradient`] - Gradient along a line between two points
//! - [`RadialGradient`] - Circular gradient radiating from a center point
//! - [`AngularGradient`] - Gradient sweeping around a center point (conic)
//! - [`MeshGradient`] - 2D grid of colored vertices with smooth interpolation
//!
//! # Examples
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//! use waterui::gradient::{LinearGradient, ColorStop, UnitPoint};
//!
//! // Linear gradient from top to bottom
//! text!("Hello")
//!     .background(LinearGradient::new(
//!         vec![
//!             ColorStop::new(Color::red(), 0.0),
//!             ColorStop::new(Color::blue(), 1.0),
//!         ],
//!         UnitPoint::TOP,
//!         UnitPoint::BOTTOM,
//!     ));
//!
//! // Animated mesh gradient
//! let phase = binding(0.0f32);
//! MeshGradient::new(3, 3, vec![/* vertices */]);
//! ```

use core::f32::consts::FRAC_1_SQRT_2;

use nami::{SignalExt, constant, signal::IntoComputed};
use waterui_core::Computed;
use waterui_graphics::color::Color;
pub use waterui_layout::UnitPoint;

/// A color stop in a gradient, consisting of a color and its position.
///
/// The position is a value from 0.0 to 1.0 representing where the color
/// appears along the gradient. Multiple stops create smooth color transitions.
///
/// # Examples
///
/// ```rust,ignore
/// use waterui::gradient::ColorStop;
/// use waterui::Color;
///
/// // Static color stop at the start
/// let stop = ColorStop::new(Color::red(), 0.0);
///
/// // Animated color stop
/// let animated_color = some_binding.map(|v| if v { Color::green() } else { Color::red() });
/// let animated_stop = ColorStop::new(animated_color, 0.5);
/// ```
#[derive(Debug, Clone)]
pub struct ColorStop {
    /// The color at this stop position.
    pub color: Computed<Color>,
    /// Position along the gradient (0.0 to 1.0).
    pub position: f32,
}

impl ColorStop {
    /// Creates a new color stop at the specified position.
    ///
    /// # Arguments
    ///
    /// * `color` - The color, can be static or a reactive `Computed<Color>`
    /// * `position` - Position from 0.0 (start) to 1.0 (end)
    ///
    /// # Panics
    ///
    /// Position is clamped to [0.0, 1.0] range.
    pub fn new(color: impl IntoComputed<Color>, position: f32) -> Self {
        Self {
            color: color.into_computed(),
            position: position.clamp(0.0, 1.0),
        }
    }
}

/// A linear gradient that transitions colors along a line.
///
/// Linear gradients blend colors from a start point to an end point.
/// The gradient direction is determined by these two points.
///
/// # Examples
///
/// ```rust,ignore
/// use waterui::gradient::{LinearGradient, ColorStop, UnitPoint};
/// use waterui::Color;
///
/// // Vertical gradient from red at top to blue at bottom
/// let gradient = LinearGradient::new(
///     vec![
///         ColorStop::new(Color::red(), 0.0),
///         ColorStop::new(Color::blue(), 1.0),
///     ],
///     UnitPoint::TOP,
///     UnitPoint::BOTTOM,
/// );
///
/// // Diagonal gradient with multiple stops
/// let rainbow = LinearGradient::new(
///     vec![
///         ColorStop::new(Color::red(), 0.0),
///         ColorStop::new(Color::yellow(), 0.5),
///         ColorStop::new(Color::green(), 1.0),
///     ],
///     UnitPoint::TOP_LEADING,
///     UnitPoint::BOTTOM_TRAILING,
/// );
/// ```
#[derive(Debug, Clone)]
pub struct LinearGradient {
    /// Color stops defining the gradient colors and positions.
    pub stops: Vec<ColorStop>,
    /// Starting point of the gradient line.
    pub start_point: UnitPoint,
    /// Ending point of the gradient line.
    pub end_point: UnitPoint,
}

impl LinearGradient {
    /// Creates a new linear gradient.
    ///
    /// # Arguments
    ///
    /// * `stops` - Color stops defining colors and their positions
    /// * `start_point` - Where the gradient begins
    /// * `end_point` - Where the gradient ends
    #[must_use]
    pub fn new(
        stops: Vec<ColorStop>,
        start_point: impl Into<UnitPoint>,
        end_point: impl Into<UnitPoint>,
    ) -> Self {
        Self {
            stops,
            start_point: start_point.into(),
            end_point: end_point.into(),
        }
    }

    /// Creates a vertical gradient from top to bottom.
    #[must_use]
    pub fn vertical(stops: Vec<ColorStop>) -> Self {
        Self::new(stops, UnitPoint::TOP, UnitPoint::BOTTOM)
    }

    /// Creates a horizontal gradient from left to right.
    #[must_use]
    pub fn horizontal(stops: Vec<ColorStop>) -> Self {
        Self::new(stops, UnitPoint::LEADING, UnitPoint::TRAILING)
    }
}

/// A radial gradient that radiates colors from a center point.
///
/// Radial gradients create circular color transitions, starting from
/// a center point and radiating outward.
///
/// # Examples
///
/// ```rust,ignore
/// use waterui::gradient::{RadialGradient, ColorStop, UnitPoint};
/// use waterui::Color;
///
/// // Circular gradient from white center to dark edges
/// let gradient = RadialGradient::new(
///     vec![
///         ColorStop::new(Color::white(), 0.0),
///         ColorStop::new(Color::black(), 1.0),
///     ],
///     UnitPoint::CENTER,
///     0.0,  // Start at center
///     0.5,  // End at half the view size
/// );
/// ```
#[derive(Debug, Clone)]
pub struct RadialGradient {
    /// Color stops defining the gradient colors and positions.
    pub stops: Vec<ColorStop>,
    /// Center point of the radial gradient.
    pub center: UnitPoint,
    /// Starting radius as a fraction of the view size (0.0 = center).
    pub start_radius: f32,
    /// Ending radius as a fraction of the view size.
    pub end_radius: f32,
}

impl RadialGradient {
    /// Creates a new radial gradient.
    ///
    /// # Arguments
    ///
    /// * `stops` - Color stops defining colors and their positions
    /// * `center` - Center point of the gradient
    /// * `start_radius` - Inner radius (0.0 = point at center)
    /// * `end_radius` - Outer radius (fraction of view size, e.g., 0.5 = half)
    #[must_use]
    pub fn new(
        stops: Vec<ColorStop>,
        center: impl Into<UnitPoint>,
        start_radius: f32,
        end_radius: f32,
    ) -> Self {
        Self {
            stops,
            center: center.into(),
            start_radius,
            end_radius,
        }
    }

    /// Creates a centered radial gradient filling the view.
    #[must_use]
    pub fn centered(stops: Vec<ColorStop>) -> Self {
        Self::new(stops, UnitPoint::CENTER, 0.0, FRAC_1_SQRT_2)
    }
}

/// An angular (conic) gradient that sweeps colors around a center point.
///
/// Angular gradients create color transitions that rotate around a center
/// point, similar to a color wheel.
///
/// # Examples
///
/// ```rust,ignore
/// use waterui::gradient::{AngularGradient, ColorStop, UnitPoint};
/// use waterui::Color;
///
/// // Full color wheel
/// let gradient = AngularGradient::new(
///     vec![
///         ColorStop::new(Color::red(), 0.0),
///         ColorStop::new(Color::yellow(), 0.17),
///         ColorStop::new(Color::green(), 0.33),
///         ColorStop::new(Color::cyan(), 0.5),
///         ColorStop::new(Color::blue(), 0.67),
///         ColorStop::new(Color::magenta(), 0.83),
///         ColorStop::new(Color::red(), 1.0),
///     ],
///     UnitPoint::CENTER,
///     0.0,           // Start at top (0 degrees)
///     360.0_f32.to_radians(), // Full rotation
/// );
/// ```
#[derive(Debug, Clone)]
pub struct AngularGradient {
    /// Color stops defining the gradient colors and positions.
    pub stops: Vec<ColorStop>,
    /// Center point around which the gradient rotates.
    pub center: UnitPoint,
    /// Starting angle in radians (0 = right, PI/2 = down).
    pub start_angle: f32,
    /// Ending angle in radians.
    pub end_angle: f32,
}

impl AngularGradient {
    /// Creates a new angular gradient.
    ///
    /// # Arguments
    ///
    /// * `stops` - Color stops defining colors and their positions
    /// * `center` - Center point of rotation
    /// * `start_angle` - Starting angle in radians
    /// * `end_angle` - Ending angle in radians
    #[must_use]
    pub fn new(
        stops: Vec<ColorStop>,
        center: impl Into<UnitPoint>,
        start_angle: f32,
        end_angle: f32,
    ) -> Self {
        Self {
            stops,
            center: center.into(),
            start_angle,
            end_angle,
        }
    }

    /// Creates a full 360-degree sweep starting from the top.
    #[must_use]
    pub fn full_sweep(stops: Vec<ColorStop>) -> Self {
        use core::f32::consts::TAU;
        Self::new(
            stops,
            UnitPoint::CENTER,
            -core::f32::consts::FRAC_PI_2,
            TAU - core::f32::consts::FRAC_PI_2,
        )
    }
}

/// A single vertex in a mesh gradient.
///
/// Each vertex has a position and color, both of which can be animated
/// using reactive `Computed` values.
///
/// # Examples
///
/// ```rust,ignore
/// use waterui::gradient::MeshVertex;
/// use waterui::Color;
///
/// // Static vertex
/// let vertex = MeshVertex::new((0.5, 0.5), Color::red());
///
/// // Animated position
/// let phase = binding(0.0f32);
/// let animated_pos = phase.map(|p| (0.5 + 0.1 * p.sin(), 0.5));
/// let animated_vertex = MeshVertex::new(animated_pos, Color::white());
/// ```
#[derive(Debug, Clone)]
pub struct MeshVertex {
    /// Position in unit coordinates (0.0 to 1.0 for both x and y).
    pub position: Computed<(f32, f32)>,
    /// Color at this vertex.
    pub color: Computed<Color>,
}

impl MeshVertex {
    /// Creates a new mesh vertex with animatable position and color.
    ///
    /// # Arguments
    ///
    /// * `position` - Position in unit coordinates, can be static or reactive
    /// * `color` - Color at this vertex, can be static or reactive
    pub fn new(position: impl IntoComputed<(f32, f32)>, color: impl IntoComputed<Color>) -> Self {
        Self {
            position: position.into_computed(),
            color: color.into_computed(),
        }
    }

    /// Creates a vertex with a static position.
    ///
    /// Convenience method when the position doesn't need to animate.
    pub fn at(x: f32, y: f32, color: impl IntoComputed<Color>) -> Self {
        Self {
            position: constant((x, y)).computed(),
            color: color.into_computed(),
        }
    }
}

/// A 2D mesh gradient defined by a grid of colored vertices.
///
/// Mesh gradients create complex color patterns by interpolating between
/// a grid of colored points. Both vertex positions and colors can be
/// animated for dynamic effects.
///
/// The mesh is defined by width × height vertices, arranged in a grid.
/// Colors are smoothly interpolated between adjacent vertices.
///
/// # `SwiftUI` Equivalent
///
/// This is similar to `SwiftUI`'s `MeshGradient` introduced in `iOS` 18.
///
/// # Examples
///
/// ```rust,ignore
/// use waterui::gradient::{MeshGradient, MeshVertex};
/// use waterui::Color;
///
/// // Simple 2x2 mesh (4 vertices)
/// let mesh = MeshGradient::new(2, 2, vec![
///     MeshVertex::at(0.0, 0.0, Color::red()),    // Top-left
///     MeshVertex::at(1.0, 0.0, Color::green()),  // Top-right
///     MeshVertex::at(0.0, 1.0, Color::blue()),   // Bottom-left
///     MeshVertex::at(1.0, 1.0, Color::yellow()), // Bottom-right
/// ]);
///
/// // Animated 3x3 mesh
/// let phase = binding(0.0f32);
/// let center_pos = phase.map(|p| {
///     (0.5 + 0.1 * (p * 2.0 * PI).sin(),
///      0.5 + 0.1 * (p * 2.0 * PI).cos())
/// }).animated();
///
/// let animated_mesh = MeshGradient::new(3, 3, vec![
///     MeshVertex::at(0.0, 0.0, Color::red()),
///     MeshVertex::at(0.5, 0.0, Color::orange()),
///     MeshVertex::at(1.0, 0.0, Color::yellow()),
///     MeshVertex::at(0.0, 0.5, Color::pink()),
///     MeshVertex::new(center_pos, Color::white()), // Animated center!
///     MeshVertex::at(1.0, 0.5, Color::green()),
///     MeshVertex::at(0.0, 1.0, Color::purple()),
///     MeshVertex::at(0.5, 1.0, Color::blue()),
///     MeshVertex::at(1.0, 1.0, Color::cyan()),
/// ]);
/// ```
#[derive(Debug, Clone)]
pub struct MeshGradient {
    /// Number of columns in the vertex grid.
    pub width: u32,
    /// Number of rows in the vertex grid.
    pub height: u32,
    /// Vertices arranged row by row (width × height total).
    pub vertices: Vec<MeshVertex>,
    /// Whether to use smooth (cubic) color interpolation.
    pub smooths_colors: bool,
}

impl MeshGradient {
    /// Creates a new mesh gradient.
    ///
    /// # Arguments
    ///
    /// * `width` - Number of columns in the vertex grid
    /// * `height` - Number of rows in the vertex grid
    /// * `vertices` - Vertices arranged row by row (must have width × height elements)
    ///
    /// # Panics
    ///
    /// Panics if `vertices.len() != width * height`.
    #[must_use]
    pub fn new(width: u32, height: u32, vertices: Vec<MeshVertex>) -> Self {
        assert_eq!(
            vertices.len(),
            (width * height) as usize,
            "MeshGradient requires exactly width × height vertices"
        );
        Self {
            width,
            height,
            vertices,
            smooths_colors: true,
        }
    }

    /// Sets whether to use smooth color interpolation.
    ///
    /// When true (default), colors are interpolated using cubic interpolation
    /// for smoother transitions. When false, linear interpolation is used.
    #[must_use]
    pub const fn smooths_colors(mut self, smooth: bool) -> Self {
        self.smooths_colors = smooth;
        self
    }
}

/// Unified enum containing all gradient types.
///
/// This enum is used internally by the `Background` system to store
/// any gradient type.
#[derive(Debug, Clone)]
pub enum Gradient {
    /// Linear gradient along a line.
    Linear(LinearGradient),
    /// Radial gradient from a center point.
    Radial(RadialGradient),
    /// Angular gradient sweeping around a center.
    Angular(AngularGradient),
    /// 2D mesh gradient with interpolated vertices.
    Mesh(MeshGradient),
}

impl From<LinearGradient> for Gradient {
    fn from(g: LinearGradient) -> Self {
        Self::Linear(g)
    }
}

impl From<RadialGradient> for Gradient {
    fn from(g: RadialGradient) -> Self {
        Self::Radial(g)
    }
}

impl From<AngularGradient> for Gradient {
    fn from(g: AngularGradient) -> Self {
        Self::Angular(g)
    }
}

impl From<MeshGradient> for Gradient {
    fn from(g: MeshGradient) -> Self {
        Self::Mesh(g)
    }
}
