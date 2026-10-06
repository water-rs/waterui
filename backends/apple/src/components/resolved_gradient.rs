//! The `resolved_gradient` leaf: `Native<Gradient>` rendered by a `CGGradient`
//! pinned to a host view.
//!
//! The layer is re-framed to the view's bounds on every layout pass. Mesh
//! gradients are rendered by the scene engine.

use core::cmp::Ordering;
use core::f64::consts::TAU;
use std::rc::Rc;

use cocoa_ui::gradient::{
    GradientLayer, GradientPaint, GradientShape, GradientSpace, GradientStop,
};
use waterui::graphics::Gradient;
use waterui::graphics::draw::{
    ColorSpace, ColorStop, Extend, Interpolation, LinearDisplayP3, LinearGradient, Paint,
    RadialGradient, Srgb, SweepGradient, WorkingColor,
};
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::HostView;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::HostView;
}

use platform::HostView;

const fn gradient_space(interpolation: Interpolation) -> GradientSpace {
    match interpolation {
        Interpolation::Working => GradientSpace::ExtendedLinearDisplayP3,
        Interpolation::SrgbEncoded => GradientSpace::ExtendedSrgb,
    }
}

fn stop_components(color: &WorkingColor, interpolation: Interpolation) -> [f64; 4] {
    let [red, green, blue, alpha] = color.components;
    let [red, green, blue] = match interpolation {
        Interpolation::Working => [red, green, blue],
        Interpolation::SrgbEncoded => {
            <LinearDisplayP3 as ColorSpace>::convert::<Srgb>([red, green, blue])
        }
    };
    [
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    ]
}

fn layer_stops(
    stops: &[ColorStop],
    interpolation: Interpolation,
    location: impl Fn(f64) -> f64,
) -> Vec<GradientStop> {
    stops
        .iter()
        .map(|stop| GradientStop {
            location: location(f64::from(stop.offset)),
            components: stop_components(&stop.color, interpolation),
        })
        .collect()
}

fn check_pad(extend: Extend) {
    assert!(
        extend == Extend::Pad,
        "the Apple backend draws only pad-extended gradients (Gradient's constructors build no other), got {extend:?}"
    );
}

/// Maps a `WaterUI` paint to its native gradient space, stops, and geometry.
fn layer_paint(paint: &Paint) -> GradientPaint {
    match paint {
        Paint::Linear(gradient) => linear_paint(gradient),
        Paint::Radial(gradient) => radial_paint(gradient),
        Paint::Sweep(gradient) => sweep_paint(gradient),
        Paint::Mesh(_) => panic!("a mesh gradient is rendered by the scene engine"),
        _ => panic!("a native gradient must carry a gradient paint"),
    }
}

fn linear_paint(gradient: &LinearGradient) -> GradientPaint {
    check_pad(gradient.extend);
    GradientPaint {
        space: gradient_space(gradient.interpolation),
        stops: layer_stops(&gradient.stops, gradient.interpolation, |offset| offset),
        shape: GradientShape::Linear {
            start: cocoa_ui::Point::new(gradient.start.x, gradient.start.y),
            end: cocoa_ui::Point::new(gradient.end.x, gradient.end.y),
        },
    }
}

fn radial_paint(gradient: &RadialGradient) -> GradientPaint {
    check_pad(gradient.extend);
    let (stops, shape) = match gradient.start_radius.total_cmp(&gradient.end_radius) {
        Ordering::Less | Ordering::Greater => (
            layer_stops(&gradient.stops, gradient.interpolation, |offset| offset),
            GradientShape::Radial {
                start_center: cocoa_ui::Point::new(
                    gradient.start_center.x,
                    gradient.start_center.y,
                ),
                start_radius: gradient.start_radius,
                end_center: cocoa_ui::Point::new(gradient.end_center.x, gradient.end_center.y),
                end_radius: gradient.end_radius,
            },
        ),
        Ordering::Equal => {
            // Gradient::radial makes the circles concentric and sorts the
            // stops ascending. Equal concentric circles define no CG PDF type 3
            // shading gradient, so represent the decided hard edge explicitly.
            let first = gradient
                .stops
                .first()
                .expect("Gradient::radial keeps at least one stop");
            let last = gradient
                .stops
                .last()
                .expect("Gradient::radial keeps at least one stop");
            let first_color = stop_components(&first.color, gradient.interpolation);
            let last_color = stop_components(&last.color, gradient.interpolation);
            (
                vec![
                    GradientStop {
                        location: 0.0,
                        components: first_color,
                    },
                    GradientStop {
                        location: 0.5,
                        components: first_color,
                    },
                    GradientStop {
                        location: 0.5,
                        components: last_color,
                    },
                    GradientStop {
                        location: 1.0,
                        components: last_color,
                    },
                ],
                GradientShape::Radial {
                    start_center: cocoa_ui::Point::new(
                        gradient.start_center.x,
                        gradient.start_center.y,
                    ),
                    start_radius: 0.0,
                    end_center: cocoa_ui::Point::new(
                        gradient.start_center.x,
                        gradient.start_center.y,
                    ),
                    end_radius: 2.0 * gradient.start_radius,
                },
            )
        }
    };
    GradientPaint {
        space: gradient_space(gradient.interpolation),
        stops,
        shape,
    }
}

fn sweep_paint(gradient: &SweepGradient) -> GradientPaint {
    check_pad(gradient.extend);
    let span = gradient.end_angle - gradient.start_angle;
    // `Gradient::angular` admits a sweep up to f32 TAU; widened to f64, that
    // bound exceeds f64 TAU, so a full turn can land a hair over one turn.
    assert!(
        span > 0.0 && span <= f64::from(core::f32::consts::TAU),
        "Apple angular gradient sweep must satisfy 0 < span <= TAU, got {span}"
    );
    let turn = (span / TAU).min(1.0);
    GradientPaint {
        space: gradient_space(gradient.interpolation),
        // Padding after the last stop matches Cherenkov's pad sweep, whose
        // parameter is `((atan2 - start) mod TAU) / span`.
        stops: layer_stops(&gradient.stops, gradient.interpolation, |offset| {
            offset * turn
        }),
        shape: GradientShape::Conic {
            center: cocoa_ui::Point::new(gradient.center.x, gradient.center.y),
            angle: gradient.start_angle,
        },
    }
}

fn update_contents_scale(view: &HostView, layer: &GradientLayer) {
    if let Some(scale) = view.display_scale() {
        layer.set_contents_scale(scale);
    }
}

/// The host view's layout face: greedy, stretching both axes — the face
/// `WuiGraphicsPrimitiveSizing` gave every graphics leaf.
struct GradientSubView;

impl core::fmt::Debug for GradientSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GradientSubView").finish_non_exhaustive()
    }
}

impl SubView for GradientSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        // Greedy: take the whole proposal on every axis.
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Installs the `resolved_gradient` handler.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<Gradient>(|gradient, ctx| {
        let mtm = ctx.mtm();
        let view = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        #[cfg(target_os = "macos")]
        cocoa_ui::view::ensure_layer_backed(&view);
        let layer = Rc::new(GradientLayer::new(mtm, layer_paint(gradient.paint())));
        cocoa_ui::view::layer(&view)
            .expect("host view is layer-backed")
            .addSublayer(&layer.layer());

        layer.set_frame(cocoa_ui::view::bounds(&view));
        update_contents_scale(&view, &layer);

        let layout_layer = Rc::clone(&layer);
        view.set_layout_handler(move |view| {
            layout_layer.set_frame(cocoa_ui::view::bounds(view));
            update_contents_scale(view, &layout_layer);
        });
        let window_layer = Rc::clone(&layer);
        view.set_window_handler(move |view| update_contents_scale(view, &window_layer));
        let backing_layer = Rc::clone(&layer);
        view.set_backing_changed_handler(move |view| {
            update_contents_scale(view, &backing_layer);
        });

        NativeLeaf::new(&*view, GradientSubView)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui::graphics::draw::kurbo::Point;

    const RED: WorkingColor = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);
    const GREEN: WorkingColor = WorkingColor::new([0.0, 1.0, 0.0, 1.0]);
    const BLUE: WorkingColor = WorkingColor::new([0.0, 0.0, 1.0, 1.0]);

    fn three_stop_radial(start_radius: f32, end_radius: f32) -> Gradient {
        Gradient::radial(
            vec![(0.0, RED), (0.5, GREEN), (1.0, BLUE)],
            [0.5, 0.5],
            start_radius,
            end_radius,
        )
    }

    #[test]
    fn working_gradient_preserves_unclamped_components() {
        let gradient = Gradient::linear(
            vec![
                (0.0, WorkingColor::new([2.0, -0.25, 0.5, 1.0])),
                (1.0, BLUE),
            ],
            [0.0, 0.0],
            [1.0, 1.0],
        );
        let paint = layer_paint(gradient.paint());

        assert_eq!(paint.space, GradientSpace::ExtendedLinearDisplayP3);
        assert_eq!(paint.stops[0].components, [2.0, -0.25, 0.5, 1.0]);
        assert_eq!(
            paint
                .stops
                .iter()
                .map(|stop| stop.location)
                .collect::<Vec<_>>(),
            vec![0.0, 1.0]
        );
        assert_eq!(
            paint.shape,
            GradientShape::Linear {
                start: cocoa_ui::Point::new(0.0, 0.0),
                end: cocoa_ui::Point::new(1.0, 1.0),
            }
        );
    }

    #[test]
    fn srgb_encoded_gradient_converts_working_colors() {
        let paint = Paint::Linear(
            LinearGradient::new(Point::new(0.0, 0.0), Point::new(1.0, 0.0))
                .stop(
                    0.0,
                    WorkingColor::new([0.214_041, 0.214_041, 0.214_041, 1.0]),
                )
                .stop(
                    1.0,
                    WorkingColor::new([0.822_461_96, 0.033_194_2, 0.017_082_632, 1.0]),
                )
                .interpolation(Interpolation::SrgbEncoded),
        );
        let paint = layer_paint(&paint);

        assert_eq!(paint.space, GradientSpace::ExtendedSrgb);
        for channel in &paint.stops[0].components[..3] {
            assert!((channel - 0.5).abs() <= 1.0e-4);
        }
        for (channel, expected) in paint.stops[1].components[..3].iter().zip([1.0, 0.0, 0.0]) {
            assert!((channel - expected).abs() <= 1.0e-3);
        }
    }

    #[test]
    fn forward_radial_gradient_preserves_both_circles_and_stops() {
        let paint = layer_paint(three_stop_radial(0.25, 0.5).paint());

        assert_eq!(
            paint.shape,
            GradientShape::Radial {
                start_center: cocoa_ui::Point::new(0.5, 0.5),
                start_radius: 0.25,
                end_center: cocoa_ui::Point::new(0.5, 0.5),
                end_radius: 0.5,
            }
        );
        assert_eq!(
            paint
                .stops
                .iter()
                .map(|stop| stop.location)
                .collect::<Vec<_>>(),
            vec![0.0, 0.5, 1.0]
        );
        assert_eq!(
            paint
                .stops
                .iter()
                .map(|stop| stop.components)
                .collect::<Vec<_>>(),
            vec![
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0, 1.0]
            ]
        );
    }

    #[test]
    fn reversed_radial_gradient_preserves_both_circles_and_stop_order() {
        let paint = layer_paint(three_stop_radial(0.5, 0.25).paint());

        assert_eq!(
            paint.shape,
            GradientShape::Radial {
                start_center: cocoa_ui::Point::new(0.5, 0.5),
                start_radius: 0.5,
                end_center: cocoa_ui::Point::new(0.5, 0.5),
                end_radius: 0.25,
            }
        );
        assert_eq!(
            paint
                .stops
                .iter()
                .map(|stop| stop.location)
                .collect::<Vec<_>>(),
            vec![0.0, 0.5, 1.0]
        );
        assert_eq!(
            paint
                .stops
                .iter()
                .map(|stop| stop.components)
                .collect::<Vec<_>>(),
            vec![
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0, 1.0]
            ]
        );
    }

    #[test]
    fn equal_radius_radial_gradient_has_an_explicit_hard_edge() {
        let paint = layer_paint(three_stop_radial(0.25, 0.25).paint());

        assert_eq!(
            paint.shape,
            GradientShape::Radial {
                start_center: cocoa_ui::Point::new(0.5, 0.5),
                start_radius: 0.0,
                end_center: cocoa_ui::Point::new(0.5, 0.5),
                end_radius: 0.5,
            }
        );
        assert_eq!(
            paint
                .stops
                .iter()
                .map(|stop| (stop.location, stop.components))
                .collect::<Vec<_>>(),
            vec![
                (0.0, [1.0, 0.0, 0.0, 1.0]),
                (0.5, [1.0, 0.0, 0.0, 1.0]),
                (0.5, [0.0, 0.0, 1.0, 1.0]),
                (1.0, [0.0, 0.0, 1.0, 1.0])
            ]
        );
    }

    #[test]
    fn angular_gradient_maps_a_partial_sweep() {
        let gradient = Gradient::angular(
            vec![(0.0, RED), (0.5, GREEN), (1.0, BLUE)],
            [0.5, 0.5],
            core::f32::consts::FRAC_PI_2,
            core::f32::consts::FRAC_PI_2 + core::f32::consts::PI,
        );
        let paint = layer_paint(gradient.paint());

        assert_eq!(
            paint.shape,
            GradientShape::Conic {
                center: cocoa_ui::Point::new(0.5, 0.5),
                angle: f64::from(core::f32::consts::FRAC_PI_2),
            }
        );
        // The f32 angles reach the paint rounded, so the span is PI only to
        // f32 precision.
        let locations: Vec<f64> = paint.stops.iter().map(|stop| stop.location).collect();
        assert_eq!(locations.len(), 3);
        for (location, expected) in locations.into_iter().zip([0.0, 0.25, 0.5]) {
            assert!(
                (location - expected).abs() <= 1.0e-6,
                "expected location {expected}, got {location}"
            );
        }
    }

    #[test]
    fn angular_gradient_with_a_full_turn_sweep_ends_at_location_one() {
        let gradient = Gradient::angular(
            vec![(0.0, RED), (1.0, BLUE)],
            [0.5, 0.5],
            0.0,
            core::f32::consts::TAU,
        );
        let paint = layer_paint(gradient.paint());

        assert_eq!(paint.stops.last().map(|stop| stop.location), Some(1.0));
    }

    #[test]
    #[should_panic(expected = "pad-extended")]
    fn repeating_gradient_is_rejected() {
        let paint = Paint::Linear(
            LinearGradient::new(Point::new(0.0, 0.0), Point::new(1.0, 0.0))
                .stop(0.0, RED)
                .stop(1.0, BLUE)
                .extend(Extend::Repeat),
        );
        let _ = layer_paint(&paint);
    }
}
