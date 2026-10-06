//! The `resolved_gradient` leaf: `Native<Gradient>` rendered through a
//! `CAGradientLayer` pinned to a host view.
//!
//! The layer is re-framed to the view's bounds and its gradient endpoints are
//! recomputed on every layout pass. The gradient type maps one-to-one onto the
//! layer's three kinds (`Mesh` is an authoring error, as the Swift `fatalError`
//! was).

use cocoa_ui::gradient::{GradientKind, GradientLayer, GradientStop};
use waterui::graphics::Gradient;
use waterui::graphics::draw::{Paint, kurbo::Point};
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

/// A Cherenkov working color as a `CGColor` in extended linear Display-P3.
fn cg_color(
    color: &waterui::graphics::draw::WorkingColor,
) -> cocoa_ui::objc2_core_foundation::CFRetained<cocoa_ui::objc2_core_graphics::CGColor> {
    let [red, green, blue, alpha] = color.components;
    cocoa_ui::color::cg_extended_linear_display_p3(
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    )
}

/// A radial layer endpoint whose per-axis offsets describe a circle in points.
///
/// `CAGradientLayer` reads the rim's offset from `startPoint` per axis in unit
/// space, so a circle of `radius * min(w, h)` points is
/// `radius * min(w, h) / w` across and `/ h` down. An empty box has a
/// zero-length rim, which is the centre itself.
fn radial_end_point(center: Point, radius: f64, size: cocoa_ui::Size) -> cocoa_ui::Point {
    assert!(
        size.width.is_finite() && size.width >= 0.0,
        "radial gradient width must be finite and >= 0, got {}",
        size.width
    );
    assert!(
        size.height.is_finite() && size.height >= 0.0,
        "radial gradient height must be finite and >= 0, got {}",
        size.height
    );
    let points = radius * size.width.min(size.height);
    if points == 0.0 {
        cocoa_ui::Point::new(center.x, center.y)
    } else {
        cocoa_ui::Point::new(
            center.x + points / size.width,
            center.y + points / size.height,
        )
    }
}

/// The gradient's shape and normalized geometry on the layer.
fn endpoints(
    paint: &Paint,
    size: cocoa_ui::Size,
) -> (GradientKind, cocoa_ui::Point, cocoa_ui::Point) {
    match paint {
        Paint::Linear(linear) => (
            GradientKind::Linear,
            cocoa_ui::Point::new(linear.start.x, linear.start.y),
            cocoa_ui::Point::new(linear.end.x, linear.end.y),
        ),
        Paint::Radial(radial) => (
            GradientKind::Radial,
            cocoa_ui::Point::new(radial.start_center.x, radial.start_center.y),
            radial_end_point(radial.end_center, radial.end_radius, size),
        ),
        Paint::Sweep(sweep) => (
            GradientKind::Angular,
            cocoa_ui::Point::new(sweep.center.x, sweep.center.y),
            cocoa_ui::Point::new(sweep.center.x, sweep.center.y),
        ),
        Paint::Mesh(_) => panic!("a mesh gradient is rendered by the scene engine"),
        _ => panic!("a native gradient must carry a gradient paint"),
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
        let layer = GradientLayer::new();
        let paint = gradient.paint().clone();
        cocoa_ui::view::layer(&view)
            .expect("host view is layer-backed")
            .addSublayer(&layer.layer());

        let stops: Vec<GradientStop> = match gradient.paint() {
            Paint::Linear(linear) => &linear.stops,
            Paint::Radial(radial) => &radial.stops,
            Paint::Sweep(sweep) => &sweep.stops,
            Paint::Mesh(_) => panic!("a mesh gradient is rendered by the scene engine"),
            _ => panic!("a native gradient must carry a gradient paint"),
        }
        .iter()
        .map(|stop| GradientStop {
            position: f64::from(stop.offset),
            color: cg_color(&stop.color),
        })
        .collect();
        layer.set_stops(&stops);
        let bounds = cocoa_ui::view::bounds(&view);
        layer.set_frame(bounds);
        let (kind, start, end) = endpoints(&paint, bounds.size);
        layer.configure(kind, start, end);

        // `gradientLayer.frame = bounds` on every layout pass — the Swift
        // `layout()` override.
        view.set_layout_handler(move |view| {
            let bounds = cocoa_ui::view::bounds(view);
            layer.set_frame(bounds);
            let (kind, start, end) = endpoints(&paint, bounds.size);
            layer.configure(kind, start, end);
        });

        NativeLeaf::new(&*view, GradientSubView)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui::graphics::draw::WorkingColor;

    fn radial_gradient() -> Gradient {
        Gradient::radial(
            vec![(0.0, WorkingColor::WHITE), (1.0, WorkingColor::BLACK)],
            [0.5, 0.5],
            0.0,
            0.5,
        )
    }

    #[test]
    fn radial_gradient_end_point_is_a_circle_on_a_200x100_box() {
        let gradient = radial_gradient();
        let (kind, start, end) = endpoints(gradient.paint(), cocoa_ui::Size::new(200.0, 100.0));

        assert_eq!(kind, GradientKind::Radial);
        assert_eq!(start, cocoa_ui::Point::new(0.5, 0.5));
        assert_eq!(end, cocoa_ui::Point::new(0.75, 1.0));
    }

    #[test]
    fn radial_gradient_end_point_tracks_the_box() {
        let gradient = radial_gradient();
        let (_, _, end) = endpoints(gradient.paint(), cocoa_ui::Size::new(100.0, 200.0));
        assert_eq!(end, cocoa_ui::Point::new(1.0, 0.75));

        let (_, _, end) = endpoints(gradient.paint(), cocoa_ui::Size::ZERO);
        assert_eq!(end, cocoa_ui::Point::new(0.5, 0.5));
    }
}
