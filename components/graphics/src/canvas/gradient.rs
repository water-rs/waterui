//! Gradient builders for Canvas.
//!
//! This module provides HTML5 Canvas-style gradient builders that use
//! WaterUI's native `ResolvedColor` type.

use waterui_color::ResolvedColor;
use waterui_core::layout::Point;

// Internal imports for rendering
use vello::peniko;

use super::conversions::{point_to_kurbo, resolved_color_to_peniko};

/// A color stop in a gradient.
///
/// This represents a color at a specific position along the gradient.
#[derive(Debug, Clone)]
pub struct ColorStop {
    /// Position along the gradient (0.0 to 1.0).
    pub offset: f32,
    /// Color at this position.
    pub color: ResolvedColor,
}

impl ColorStop {
    /// Creates a new color stop.
    #[must_use]
    pub fn new(offset: f32, color: impl Into<ResolvedColor>) -> Self {
        Self {
            offset,
            color: color.into(),
        }
    }
}

/// Linear gradient builder.
///
/// Creates a gradient that transitions colors along a straight line
/// from a start point to an end point.
///
/// # Example
///
/// ```ignore
/// let gradient = ctx.create_linear_gradient(0.0, 0.0, 100.0, 100.0);
/// gradient.add_color_stop(0.0, Srgb::new(1.0, 0.0, 0.0));
/// gradient.add_color_stop(1.0, Srgb::new(0.0, 0.0, 1.0));
/// ctx.set_fill_style(gradient);
/// ```
#[derive(Debug, Clone)]
pub struct LinearGradient {
    start: Point,
    end: Point,
    stops: Vec<ColorStop>,
}

impl LinearGradient {
    /// Creates a new linear gradient from (x0, y0) to (x1, y1).
    #[must_use]
    pub const fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self {
            start: Point::new(x0, y0),
            end: Point::new(x1, y1),
            stops: Vec::new(),
        }
    }

    /// Adds a color stop to the gradient.
    ///
    /// # Arguments
    /// * `offset` - Position (0.0 to 1.0) along the gradient
    /// * `color` - Color at this position
    pub fn add_color_stop(&mut self, offset: f32, color: impl Into<ResolvedColor>) {
        self.stops.push(ColorStop::new(offset, color));
    }

    /// Builds the gradient into a peniko Brush for rendering.
    #[must_use]
    pub(crate) fn build(&self) -> peniko::Brush {
        // Convert color stops to peniko format
        let peniko_stops: Vec<peniko::ColorStop> = self
            .stops
            .iter()
            .map(|stop| {
                let peniko_color = resolved_color_to_peniko(stop.color);
                peniko::ColorStop {
                    offset: stop.offset,
                    color: peniko_color.into(),
                }
            })
            .collect();

        // Create linear gradient
        let gradient =
            peniko::Gradient::new_linear(point_to_kurbo(self.start), point_to_kurbo(self.end))
                .with_stops(&*peniko_stops);

        peniko::Brush::Gradient(gradient)
    }
}

/// Radial gradient builder.
///
/// Creates a gradient that transitions colors radially from one circle to another.
///
/// # Example
///
/// ```ignore
/// let gradient = ctx.create_radial_gradient(50.0, 50.0, 10.0, 50.0, 50.0, 50.0);
/// gradient.add_color_stop(0.0, Srgb::new(1.0, 1.0, 1.0));
/// gradient.add_color_stop(1.0, Srgb::new(0.0, 0.0, 0.0));
/// ctx.set_fill_style(gradient);
/// ```
#[derive(Debug, Clone)]
pub struct RadialGradient {
    center0: Point,
    radius0: f32,
    center1: Point,
    radius1: f32,
    stops: Vec<ColorStop>,
}

impl RadialGradient {
    /// Creates a new radial gradient.
    ///
    /// # Arguments
    /// * `x0, y0` - Center of the start circle
    /// * `r0` - Radius of the start circle
    /// * `x1, y1` - Center of the end circle
    /// * `r1` - Radius of the end circle
    #[must_use]
    pub const fn new(x0: f32, y0: f32, r0: f32, x1: f32, y1: f32, r1: f32) -> Self {
        Self {
            center0: Point::new(x0, y0),
            radius0: r0,
            center1: Point::new(x1, y1),
            radius1: r1,
            stops: Vec::new(),
        }
    }

    /// Adds a color stop to the gradient.
    pub fn add_color_stop(&mut self, offset: f32, color: impl Into<ResolvedColor>) {
        self.stops.push(ColorStop::new(offset, color));
    }

    /// Builds the gradient into a peniko Brush for rendering.
    #[must_use]
    pub(crate) fn build(&self) -> peniko::Brush {
        // Convert color stops
        let peniko_stops: Vec<peniko::ColorStop> = self
            .stops
            .iter()
            .map(|stop| {
                let peniko_color = resolved_color_to_peniko(stop.color);
                peniko::ColorStop {
                    offset: stop.offset,
                    color: peniko_color.into(),
                }
            })
            .collect();

        // Create radial gradient
        let gradient = peniko::Gradient::new_two_point_radial(
            point_to_kurbo(self.center0),
            self.radius0,
            point_to_kurbo(self.center1),
            self.radius1,
        )
        .with_stops(&*peniko_stops);

        peniko::Brush::Gradient(gradient)
    }
}

/// Conic (sweep) gradient builder.
///
/// Creates a gradient that transitions colors in a circular sweep around a center point.
///
/// # Example
///
/// ```ignore
/// let gradient = ctx.create_conic_gradient(0.0, 50.0, 50.0);
/// gradient.add_color_stop(0.0, Srgb::new(1.0, 0.0, 0.0));
/// gradient.add_color_stop(0.5, Srgb::new(0.0, 1.0, 0.0));
/// gradient.add_color_stop(1.0, Srgb::new(0.0, 0.0, 1.0));
/// ctx.set_fill_style(gradient);
/// ```
#[derive(Debug, Clone)]
pub struct ConicGradient {
    center: Point,
    start_angle: f32,
    stops: Vec<ColorStop>,
}

impl ConicGradient {
    /// Creates a new conic gradient.
    ///
    /// # Arguments
    /// * `start_angle` - Starting angle in radians
    /// * `x, y` - Center point
    #[must_use]
    pub const fn new(start_angle: f32, x: f32, y: f32) -> Self {
        Self {
            center: Point::new(x, y),
            start_angle,
            stops: Vec::new(),
        }
    }

    /// Adds a color stop to the gradient.
    pub fn add_color_stop(&mut self, offset: f32, color: impl Into<ResolvedColor>) {
        self.stops.push(ColorStop::new(offset, color));
    }

    /// Builds the gradient into a peniko Brush for rendering.
    #[must_use]
    pub(crate) fn build(&self) -> peniko::Brush {
        // Convert color stops
        let peniko_stops: Vec<peniko::ColorStop> = self
            .stops
            .iter()
            .map(|stop| {
                let peniko_color = resolved_color_to_peniko(stop.color);
                peniko::ColorStop {
                    offset: stop.offset,
                    color: peniko_color.into(),
                }
            })
            .collect();

        // Create sweep gradient
        let gradient =
            peniko::Gradient::new_sweep(point_to_kurbo(self.center), self.start_angle, 0.0)
                .with_stops(&*peniko_stops);

        peniko::Brush::Gradient(gradient)
    }
}
