//! A `CALayer` delegate that draws a `CGGradient` in its contents space.

use core::fmt;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{
    CGColorSpace, CGContext, CGGradient, CGGradientDrawingOptions,
    kCGColorSpaceExtendedLinearDisplayP3, kCGColorSpaceExtendedSRGB,
};
use objc2_foundation::{NSNull, NSObjectProtocol, NSString};
use objc2_quartz_core::{CAAction, CALayer, CALayerDelegate};

use crate::callback::guarded;
use crate::geometry::{Point, Rect};

/// The colour space in which the gradient interpolates its components.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientSpace {
    /// Extended linear Display P3.
    ExtendedLinearDisplayP3,
    /// Extended sRGB with the sRGB transfer curve.
    ExtendedSrgb,
}

/// A straight-alpha colour stop in the gradient's interpolation space.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientStop {
    /// Position along the gradient, in `[0, 1]`.
    pub location: f64,
    /// Red, green, blue, and alpha components.
    pub components: [f64; 4],
}

/// Geometry in the layer's unit space: `(0, 0)` is top-left and `(1, 1)` is
/// the bottom-right of its bounds.
#[derive(Clone, Debug, PartialEq)]
pub enum GradientShape {
    /// A gradient along the line from `start` to `end`.
    Linear {
        /// The line's start in unit space.
        start: Point,
        /// The line's end in unit space.
        end: Point,
    },
    /// Radii are fractions of the shorter side: `r` is a circle of
    /// `r * min(width, height)` points.
    Radial {
        /// Centre of the first circle.
        start_center: Point,
        /// Radius of the first circle, as a fraction of the shorter side.
        start_radius: f64,
        /// Centre of the second circle.
        end_center: Point,
        /// Radius of the second circle, as a fraction of the shorter side.
        end_radius: f64,
    },
    /// Location zero lies at `angle` radians from +x, increasing clockwise
    /// on screen (y down).
    Conic {
        /// Centre of the conic gradient.
        center: Point,
        /// Angle of location zero, in radians.
        angle: f64,
    },
}

/// Gradient colours, interpolation space, and geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientPaint {
    /// The space in which stop components are interpolated.
    pub space: GradientSpace,
    /// Stops in ascending location order; equal locations make hard stops.
    pub stops: Vec<GradientStop>,
    /// The gradient geometry in unit-space coordinates.
    pub shape: GradientShape,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ShapeInPoints {
    Linear {
        start: Point,
        end: Point,
    },
    Radial {
        start_center: Point,
        start_radius: f64,
        end_center: Point,
        end_radius: f64,
    },
    Conic {
        center: Point,
        angle: f64,
    },
}

fn validate_stops(stops: &[GradientStop]) -> (Vec<f64>, Vec<f64>) {
    assert!(!stops.is_empty(), "a gradient must have at least one stop");

    let mut components = Vec::with_capacity(stops.len() * 4);
    let mut locations = Vec::with_capacity(stops.len());
    let mut previous_location = None;
    for stop in stops {
        assert!(
            stop.location.is_finite(),
            "gradient stop location must be finite, got {}",
            stop.location
        );
        assert!(
            (0.0..=1.0).contains(&stop.location),
            "gradient stop location must be in [0, 1], got {}",
            stop.location
        );
        if let Some(previous_location) = previous_location {
            assert!(
                previous_location <= stop.location,
                "gradient stop locations must be ascending, got {previous_location} then {}",
                stop.location
            );
        }
        assert!(
            stop.components
                .iter()
                .all(|component| component.is_finite()),
            "gradient stop components must be finite, got {:?}",
            stop.components
        );
        components.extend(stop.components);
        locations.push(stop.location);
        previous_location = Some(stop.location);
    }
    (components, locations)
}

fn shape_in_points(shape: &GradientShape, bounds: Rect) -> ShapeInPoints {
    let point = |unit: Point| {
        Point::new(
            unit.x.mul_add(bounds.size.width, bounds.origin.x),
            unit.y.mul_add(bounds.size.height, bounds.origin.y),
        )
    };
    let radius = |unit: f64| unit * bounds.size.width.min(bounds.size.height);

    match *shape {
        GradientShape::Linear { start, end } => ShapeInPoints::Linear {
            start: point(start),
            end: point(end),
        },
        GradientShape::Radial {
            start_center,
            start_radius,
            end_center,
            end_radius,
        } => ShapeInPoints::Radial {
            start_center: point(start_center),
            start_radius: radius(start_radius),
            end_center: point(end_center),
            end_radius: radius(end_radius),
        },
        GradientShape::Conic { center, angle } => ShapeInPoints::Conic {
            center: point(center),
            angle,
        },
    }
}

struct PainterIvars {
    gradient: CFRetained<CGGradient>,
    shape: GradientShape,
}

define_class!(
    // SAFETY: the NSObject subclass retains immutable gradient drawing data
    // and Quartz invokes its delegate only on the main thread.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiGradientPainter"]
    #[thread_kind = MainThreadOnly]
    #[ivars = PainterIvars]
    struct Painter;

    // SAFETY: NSObjectProtocol has no additional requirements for this class.
    unsafe impl NSObjectProtocol for Painter {}
    // SAFETY: The delegate callback matches CALayer's main-thread drawing contract.
    unsafe impl CALayerDelegate for Painter {}

    impl Painter {
        // SAFETY: see the module safety note.
        #[unsafe(method_id(actionForLayer:forKey:))]
        fn action_for_layer_for_key(
            &self,
            _layer: &CALayer,
            _event: &NSString,
        ) -> Option<Retained<ProtocolObject<dyn CAAction>>> {
            // A non-nil NSNull ends the action search with no action, per
            // CALayer.h: with no implicit animations the standalone layer's
            // `contents` redraw never cross-fades or stretches the old bitmap.
            Some(ProtocolObject::from_retained(NSNull::null()))
        }

        #[unsafe(method(drawLayer:inContext:))]
        fn draw_layer_in_context(&self, layer: &CALayer, context: &CGContext) {
            guarded("CocoaUiGradientPainter drawLayer:inContext:", || {
                let bounds: Rect = layer.bounds().into();
                if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
                    return;
                }
                let shape = shape_in_points(&self.ivars().shape, bounds);
                let gradient = &self.ivars().gradient;
                match shape {
                    ShapeInPoints::Linear { start, end } => {
                        CGContext::draw_linear_gradient(
                            Some(context),
                            Some(gradient),
                            start.into(),
                            end.into(),
                            gradient_drawing_options(),
                        );
                    }
                    ShapeInPoints::Radial {
                        start_center,
                        start_radius,
                        end_center,
                        end_radius,
                    } => {
                        CGContext::draw_radial_gradient(
                            Some(context),
                            Some(gradient),
                            start_center.into(),
                            start_radius,
                            end_center.into(),
                            end_radius,
                            gradient_drawing_options(),
                        );
                    }
                    ShapeInPoints::Conic { center, angle } => {
                        context.draw_conic_gradient(Some(gradient), center.into(), angle);
                    }
                }
            });
        }
    }
);

fn gradient_drawing_options() -> CGGradientDrawingOptions {
    CGGradientDrawingOptions::DrawsBeforeStartLocation
        | CGGradientDrawingOptions::DrawsAfterEndLocation
}

impl Painter {
    fn new(
        mtm: MainThreadMarker,
        gradient: CFRetained<CGGradient>,
        shape: GradientShape,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PainterIvars { gradient, shape });
        // SAFETY: `init` is NSObject's designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

impl fmt::Debug for Painter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Painter").finish_non_exhaustive()
    }
}

/// A gradient rendered by a `CALayer` delegate.
#[derive(Debug)]
pub struct GradientLayer {
    layer: Retained<CALayer>,
    /// Owns the layer's delegate, which `CALayer` holds weakly.
    _painter: Retained<Painter>,
}

impl GradientLayer {
    /// Creates and configures a gradient layer.
    ///
    /// # Panics
    ///
    /// Panics when a stop is invalid or Core Graphics cannot create the
    /// requested gradient or colour space.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, paint: GradientPaint) -> Self {
        let (components, locations) = validate_stops(&paint.stops);
        let color_space = match paint.space {
            GradientSpace::ExtendedLinearDisplayP3 => {
                // SAFETY: this is the Core Graphics colour-space constant.
                CGColorSpace::with_name(Some(unsafe { kCGColorSpaceExtendedLinearDisplayP3 }))
            }
            GradientSpace::ExtendedSrgb => {
                // SAFETY: this is the Core Graphics colour-space constant.
                CGColorSpace::with_name(Some(unsafe { kCGColorSpaceExtendedSRGB }))
            }
        }
        .expect("Core Graphics must provide the requested extended colour space");
        // SAFETY: both slices are valid for the call; there are four
        // components per stop and one location per stop.
        let gradient = unsafe {
            CGGradient::with_color_components(
                Some(&color_space),
                components.as_ptr(),
                locations.as_ptr(),
                locations.len(),
            )
        }
        .expect("Core Graphics must create a gradient from validated stops");
        let painter = Painter::new(mtm, gradient, paint.shape);
        let layer = CALayer::new();
        layer.setDelegate(Some(ProtocolObject::from_ref(&*painter)));
        layer.setNeedsDisplayOnBoundsChange(true);
        layer.setNeedsDisplay();
        Self {
            layer,
            _painter: painter,
        }
    }

    /// The backing layer, for `addSublayer` and related operations.
    #[must_use]
    pub fn layer(&self) -> Retained<CALayer> {
        self.layer.clone()
    }

    /// Sets the layer frame.
    pub fn set_frame(&self, frame: Rect) {
        self.layer.setFrame(frame.into());
    }

    /// Sets the device-pixels-per-point scale and invalidates when it changes.
    ///
    /// # Panics
    ///
    /// Panics when `scale` is not finite and greater than zero.
    pub fn set_contents_scale(&self, scale: f64) {
        assert!(
            scale.is_finite() && scale > 0.0,
            "gradient contents scale must be finite and > 0, got {scale}"
        );
        if self.layer.contentsScale().to_bits() != scale.to_bits() {
            self.layer.setContentsScale(scale);
            self.layer.setNeedsDisplay();
        }
    }
}

impl Drop for GradientLayer {
    fn drop(&mut self) {
        self.layer.setDelegate(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Rect {
        Rect::new(10.0, 20.0, 200.0, 100.0)
    }

    #[test]
    fn gradient_shape_in_points_maps_radial_centres_and_radii() {
        assert_eq!(
            shape_in_points(
                &GradientShape::Radial {
                    start_center: Point::new(0.5, 0.5),
                    start_radius: 0.25,
                    end_center: Point::new(0.5, 0.5),
                    end_radius: 0.5,
                },
                bounds()
            ),
            ShapeInPoints::Radial {
                start_center: Point::new(110.0, 70.0),
                start_radius: 25.0,
                end_center: Point::new(110.0, 70.0),
                end_radius: 50.0,
            }
        );
    }

    #[test]
    fn gradient_shape_in_points_maps_linear_endpoints() {
        assert_eq!(
            shape_in_points(
                &GradientShape::Linear {
                    start: Point::new(0.0, 0.0),
                    end: Point::new(1.0, 1.0),
                },
                bounds()
            ),
            ShapeInPoints::Linear {
                start: Point::new(10.0, 20.0),
                end: Point::new(210.0, 120.0),
            }
        );
    }

    #[test]
    fn gradient_shape_in_points_preserves_conic_angle() {
        assert_eq!(
            shape_in_points(
                &GradientShape::Conic {
                    center: Point::new(0.5, 0.5),
                    angle: core::f64::consts::FRAC_PI_2,
                },
                bounds()
            ),
            ShapeInPoints::Conic {
                center: Point::new(110.0, 70.0),
                angle: core::f64::consts::FRAC_PI_2,
            }
        );
    }

    #[test]
    fn gradient_stop_validation_flattens_components_and_keeps_hard_stops() {
        let stops = [
            GradientStop {
                location: 0.0,
                components: [2.0, -0.25, 0.5, 1.0],
            },
            GradientStop {
                location: 0.5,
                components: [1.0, 0.0, 0.0, 1.0],
            },
            GradientStop {
                location: 0.5,
                components: [0.0, 0.0, 1.0, 1.0],
            },
        ];
        assert_eq!(
            validate_stops(&stops),
            (
                vec![2.0, -0.25, 0.5, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0,],
                vec![0.0, 0.5, 0.5]
            )
        );
    }

    #[test]
    #[should_panic(expected = "ascending")]
    fn gradient_stop_validation_rejects_descending_locations() {
        validate_stops(&[
            GradientStop {
                location: 0.75,
                components: [0.0; 4],
            },
            GradientStop {
                location: 0.25,
                components: [0.0; 4],
            },
        ]);
    }

    #[test]
    #[should_panic(expected = "[0, 1]")]
    fn gradient_stop_validation_rejects_out_of_range_locations() {
        validate_stops(&[GradientStop {
            location: 1.5,
            components: [0.0; 4],
        }]);
    }
}
