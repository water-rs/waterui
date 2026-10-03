//! The `resolved_gradient` leaf: `Native<Gradient>` rendered through a
//! `CAGradientLayer` pinned to a host view.
//!
//! The layer is re-framed to the view's bounds on every layout pass and the
//! gradient type maps one-to-one onto the layer's three kinds (`Mesh` is an
//! authoring error, as the Swift `fatalError` was).

use cocoa_ui::gradient::{GradientKind, GradientLayer, GradientStop};
use waterui::graphics::Gradient;
use waterui::graphics::cherenkov::Paint;
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
    color: &waterui::graphics::cherenkov::WorkingColor,
) -> cocoa_ui::objc2_core_foundation::CFRetained<cocoa_ui::objc2_core_graphics::CGColor> {
    let [red, green, blue, alpha] = color.components;
    cocoa_ui::color::cg_extended_linear_display_p3(
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    )
}

/// The gradient's shape and normalized geometry on the layer.
fn configure(layer: &GradientLayer, gradient: &Gradient) {
    let (kind, start, end) = match gradient.paint() {
        Paint::Linear(linear) => (
            GradientKind::Linear,
            cocoa_ui::Point::new(linear.start.x, linear.start.y),
            cocoa_ui::Point::new(linear.end.x, linear.end.y),
        ),
        Paint::Radial(radial) => (
            GradientKind::Radial,
            cocoa_ui::Point::new(radial.start_center.x, radial.start_center.y),
            cocoa_ui::Point::new(radial.end_center.x + radial.end_radius, radial.end_center.y),
        ),
        Paint::Sweep(sweep) => (
            GradientKind::Angular,
            cocoa_ui::Point::new(sweep.center.x, sweep.center.y),
            cocoa_ui::Point::new(sweep.center.x, sweep.center.y),
        ),
        Paint::Mesh(_) => panic!("a mesh gradient is rendered by the scene engine"),
        _ => panic!("a native gradient must carry a gradient paint"),
    };
    layer.configure(kind, start, end);
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
        configure(&layer, &gradient);
        layer.set_frame(cocoa_ui::view::bounds(&view));

        // `gradientLayer.frame = bounds` on every layout pass — the Swift
        // `layout()` override.
        view.set_layout_handler(move |view| {
            layer.set_frame(cocoa_ui::view::bounds(view));
        });

        NativeLeaf::new(&*view, GradientSubView)
    });
}
