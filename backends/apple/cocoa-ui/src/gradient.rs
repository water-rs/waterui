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
use objc2_quartz_core::{
    CAAction, CADynamicRangeHigh, CALayer, CALayerDelegate, kCAContentsFormatRGBA16Float,
};
use waterui_contract::contract_violation;

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

/// The flattened components and locations of `stops`.
///
/// A stop breaking the gradient contract is reported and corrected: an empty
/// list becomes one transparent stop, a location is clamped into `[0, 1]`
/// (a non-finite one becomes `0`) and raised to the previous stop's, and a
/// non-finite component becomes `0`.
fn validate_stops(stops: &[GradientStop]) -> (Vec<f64>, Vec<f64>) {
    const TRANSPARENT: GradientStop = GradientStop {
        location: 0.0,
        components: [0.0; 4],
    };
    let stops = if stops.is_empty() {
        contract_violation!(
            "a gradient must have at least one stop; a single transparent stop is used"
        );
        &[TRANSPARENT][..]
    } else {
        stops
    };

    let mut components = Vec::with_capacity(stops.len() * 4);
    let mut locations = Vec::with_capacity(stops.len());
    let mut previous_location = None;
    for stop in stops {
        let mut location = stop.location;
        if !location.is_finite() {
            contract_violation!(
                "gradient stop location must be finite, got {location}; the location is 0"
            );
            location = 0.0;
        } else if !(0.0..=1.0).contains(&location) {
            contract_violation!(
                "gradient stop location must be in [0, 1], got {location}; the location is clamped"
            );
            location = location.clamp(0.0, 1.0);
        }
        if let Some(previous_location) = previous_location
            && previous_location > location
        {
            contract_violation!(
                "gradient stop locations must be ascending, got {previous_location} then {location}; the location is raised to the previous one"
            );
            location = previous_location;
        }
        let mut stop_components = stop.components;
        if stop_components
            .iter()
            .any(|component| !component.is_finite())
        {
            contract_violation!(
                "gradient stop components must be finite, got {:?}; the non-finite components are 0",
                stop.components
            );
            for component in &mut stop_components {
                if !component.is_finite() {
                    *component = 0.0;
                }
            }
        }
        components.extend(stop_components);
        locations.push(location);
        previous_location = Some(location);
    }
    (components, locations)
}

/// Whether any stop carries a colour outside SDR: an RGB component above
/// `1.0` is an HDR highlight; below `0.0` is a wide-gamut value the extended
/// spaces encode. An unorm `RGBA8Uint` backing clamps both at rasterization.
fn needs_extended_range(stops: &[GradientStop]) -> bool {
    stops
        .iter()
        .flat_map(|stop| &stop.components[..3])
        .any(|component| !(0.0..=1.0).contains(component))
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
        .expect("Core Graphics names both extended colour spaces: the names are its own constants");
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
        .expect("Core Graphics creates a gradient from finite, ascending stops: `validate_stops` made them so");
        let painter = Painter::new(mtm, gradient, paint.shape);
        let layer = CALayer::new();
        if needs_extended_range(&paint.stops) {
            // A delegate-drawn layer rasters into its backing store, and the
            // default `RGBA8Uint` format clamps every channel to [0, 1] —
            // HDR stops flatten to SDR white before the compositor ever sees
            // them. A half-float backing preserves the values, and the High
            // preference asks the display for EDR headroom; on an SDR display
            // both are inert. An explicit `Metadata<StandardDynamicRange>`
            // ancestor still overrides the preference — this is the paint's
            // own range, not a subtree pin.
            // SAFETY: the extern statics are immutable framework constants.
            unsafe {
                layer.setContentsFormat(kCAContentsFormatRGBA16Float);
                layer.setPreferredDynamicRange(CADynamicRangeHigh);
            }
        }
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
    /// A `scale` that is not finite and greater than zero breaks the
    /// contract: it is reported and the call is ignored.
    pub fn set_contents_scale(&self, scale: f64) {
        if !(scale.is_finite() && scale > 0.0) {
            contract_violation!(
                "gradient contents scale must be finite and > 0, got {scale}; the call is ignored"
            );
            return;
        }
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
    #[cfg_attr(debug_assertions, should_panic(expected = "ascending"))]
    fn gradient_stop_validation_raises_descending_locations() {
        let (_, locations) = validate_stops(&[
            GradientStop {
                location: 0.75,
                components: [0.0; 4],
            },
            GradientStop {
                location: 0.25,
                components: [0.0; 4],
            },
        ]);
        assert_eq!(locations, vec![0.75, 0.75]);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "at least one stop"))]
    fn gradient_stop_validation_replaces_an_empty_list_with_a_transparent_stop() {
        assert_eq!(validate_stops(&[]), (vec![0.0; 4], vec![0.0]));
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "must be finite"))]
    fn gradient_stop_validation_zeroes_non_finite_components_and_locations() {
        let (components, locations) = validate_stops(&[GradientStop {
            location: f64::NAN,
            components: [f64::INFINITY, 0.5, f64::NAN, 1.0],
        }]);
        assert_eq!(components, vec![0.0, 0.5, 0.0, 1.0]);
        assert_eq!(locations, vec![0.0]);
    }

    #[test]
    fn gradient_range_check_flags_only_out_of_range_rgb() {
        let sdr = [
            GradientStop {
                location: 0.0,
                components: [0.0, 0.5, 1.0, 1.0],
            },
            GradientStop {
                location: 1.0,
                components: [1.0, 1.0, 1.0, 1.0],
            },
        ];
        assert!(!needs_extended_range(&sdr));

        let hdr = [GradientStop {
            location: 0.5,
            components: [4.0, 4.0, 4.0, 1.0],
        }];
        assert!(needs_extended_range(&hdr));

        let wide_gamut = [GradientStop {
            location: 0.5,
            components: [-0.2, 0.5, 0.5, 1.0],
        }];
        assert!(needs_extended_range(&wide_gamut));

        // Alpha is compositing weight, not luminance — an out-of-range
        // alpha alone does not make the paint extended-range.
        let odd_alpha = [GradientStop {
            location: 0.5,
            components: [0.5, 0.5, 0.5, 2.0],
        }];
        assert!(!needs_extended_range(&odd_alpha));
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "[0, 1]"))]
    fn gradient_stop_validation_clamps_out_of_range_locations() {
        let (_, locations) = validate_stops(&[GradientStop {
            location: 1.5,
            components: [0.0; 4],
        }]);
        assert_eq!(locations, vec![1.0]);
    }
}
