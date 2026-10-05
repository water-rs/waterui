use waterui_graphics::Gradient;
use waterui_graphics::draw::Paint;

use crate::{IntoFFI, WuiArray, color::WuiWorkingColor};

/// C ABI mirror of a gradient colour stop.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct WuiGradientStop {
    /// Position along the gradient, from 0 to 1.
    pub position: f32,
    /// The colour at this position.
    pub color: WuiWorkingColor,
}

/// C ABI discriminator for the gradient payloads backends may receive.
///
/// [`GradientType`](waterui_graphics::GradientType) also has a `Mesh`
/// variant, but a mesh gradient resolves to engine content rather than to
/// this payload, so it is deliberately not represented here.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub enum WuiGradientType {
    /// Linear gradient along a line.
    Linear = 0,
    /// Radial gradient from a center point.
    Radial = 1,
    /// Angular (conic) gradient around a center point.
    Angular = 2,
}

/// C ABI mirror of [`Gradient`], the backend-native gradient payload in unit
/// space.
///
/// Backends scale it onto the view's bounds. A mesh gradient never reaches
/// this payload: it resolves to engine content, so only linear, radial and
/// angular gradients cross.
#[repr(C)]
#[derive(Debug)]
pub struct WuiGradient {
    /// Gradient kind (linear, radial or angular).
    pub gradient_type: WuiGradientType,
    /// The colour stops.
    pub stops: WuiArray<WuiGradientStop>,
    /// Start point (linear) or center (radial/angular) x-coordinate.
    pub start_x: f32,
    /// Start point (linear) or center (radial/angular) y-coordinate.
    pub start_y: f32,
    /// End point (linear) x-coordinate.
    pub end_x: f32,
    /// End point (linear) y-coordinate.
    pub end_y: f32,
    /// Start radius (radial) or start angle in radians (angular).
    pub start_value: f32,
    /// End radius (radial) or end angle in radians (angular).
    pub end_value: f32,
}

#[allow(clippy::cast_possible_truncation)]
impl IntoFFI for Gradient {
    type FFI = WuiGradient;

    fn into_ffi(self) -> Self::FFI {
        let f = |v: f64| v as f32;
        let stops = |stops: &[waterui_graphics::draw::ColorStop]| {
            WuiArray::new(
                stops
                    .iter()
                    .map(|stop| WuiGradientStop {
                        position: stop.offset,
                        color: stop.color.into_ffi(),
                    })
                    .collect::<Vec<_>>(),
            )
        };
        match self.paint() {
            Paint::Linear(linear) => WuiGradient {
                gradient_type: WuiGradientType::Linear,
                stops: stops(&linear.stops),
                start_x: f(linear.start.x),
                start_y: f(linear.start.y),
                end_x: f(linear.end.x),
                end_y: f(linear.end.y),
                start_value: 0.0,
                end_value: 0.0,
            },
            Paint::Radial(radial) => WuiGradient {
                gradient_type: WuiGradientType::Radial,
                stops: stops(&radial.stops),
                start_x: f(radial.start_center.x),
                start_y: f(radial.start_center.y),
                end_x: f(radial.end_center.x),
                end_y: f(radial.end_center.y),
                start_value: f(radial.start_radius),
                end_value: f(radial.end_radius),
            },
            Paint::Sweep(sweep) => WuiGradient {
                gradient_type: WuiGradientType::Angular,
                stops: stops(&sweep.stops),
                start_x: f(sweep.center.x),
                start_y: f(sweep.center.y),
                end_x: f(sweep.center.x),
                end_y: f(sweep.center.y),
                start_value: f(sweep.start_angle),
                end_value: f(sweep.end_angle),
            },
            Paint::Mesh(_) => {
                unreachable!(
                    "a mesh gradient resolves to engine content, not a native gradient view"
                )
            }
            Paint::Solid(_) | Paint::Image(_) | Paint::Shader(_) | Paint::Transformed(_) => {
                unreachable!("a gradient view only carries gradient paints")
            }
        }
    }
}

// `Gradient` is a raw view rendered natively by platform backends.
ffi_view!(Gradient, WuiGradient, gradient);
