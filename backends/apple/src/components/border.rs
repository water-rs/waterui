//! The `border` metadata: `Metadata<Border>` wrapped around a child.
//!
//! Mirrors `WuiBorder`: a transparent `HostView` container that strokes its
//! own layer. A border covering every edge uses the layer's `borderWidth`,
//! `cornerRadius` and never clips its content; a partial edge set draws through a
//! `CAShapeLayer` sublayer stroked along only the selected edges. The color
//! resolves through the environment, so every change lands as an imperative
//! setter inside a watcher, then `invalidateCapturedRendering` fires so a
//! cached capture re-renders.

use alloc::rc::Rc;
use core::cell::Cell;

use cocoa_ui::layer;
use cocoa_ui::path::{EdgeMask, border_path};
use cocoa_ui::{PlatformView, Rect, Retained, view};
use waterui::border::Border;
use waterui::graphics::color::WorkingColor;
use waterui::layout::EdgeSet;
use waterui::reactive::{Computed, Signal};
use waterui_core::Metadata;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect as LayoutRect, ResolvedExtent, StretchAxis, SubView,
    ViewDimensions,
};

use crate::animation::with_platform_animation;
use crate::contract::{Held, Mounted, NativeLeaf};
use crate::proposal;

use crate::contract::RenderContext;
#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::colors;
    pub(super) use cocoa_ui::objc2_app_kit::NSColor as PlatformColor;
}
#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::objc2_ui_kit::UIColor as PlatformColor;
    pub(super) use cocoa_ui::uikit::colors;
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object — the
/// same conversion `resolved_color` applies.
#[cfg(target_os = "ios")]
fn platform_color(color: &WorkingColor) -> Retained<platform::PlatformColor> {
    {
        let [red, green, blue, alpha] = color.components;
        platform::colors::extended_linear_display_p3(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object, with HDR
/// headroom applied as a content-headroom multiplier — the `AppKit` variant.
#[cfg(target_os = "macos")]
fn platform_color(color: &WorkingColor) -> Retained<platform::PlatformColor> {
    {
        let [red, green, blue, alpha] = color.components;
        platform::colors::extended_linear_display_p3(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

/// The leaf's live state: the mounted child and the partial-edge stroke
/// layer, when the border does not cover all four edges.
struct BorderState {
    /// The mounted content.
    child: Mounted,
    /// The stroke layer for a partial edge set — `None` when the layer's
    /// own border properties do the drawing.
    border_layer: Option<layer::ShapeLayer>,
    /// Current border stroke width.
    width: Cell<f32>,
    /// Current border corner radius.
    corner_radius: Cell<f32>,
    /// Edges covered by this border.
    edges: EdgeSet,
}

impl core::fmt::Debug for BorderState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BorderState").finish_non_exhaustive()
    }
}

/// `applyBorderColor`: the resolved color lands on the layer's border and
/// the stroke layer, then any captured rendering is invalidated.
fn apply_border_color(
    color: &WorkingColor,
    host: &PlatformView,
    border_layer: Option<&layer::ShapeLayer>,
) {
    let platform = platform_color(color);
    let cg = platform::colors::cg(&platform);
    if let Some(layer) = layer::layer_of(host) {
        layer::set_border_color(&layer, Some(&cg));
    }
    if let Some(border) = border_layer {
        border.set_stroke(Some(&cg));
    }
    view::invalidate_captured_rendering(host);
}

fn configure_border_layer(
    host: &PlatformView,
    edges: EdgeSet,
    width: f32,
    corner_radius: f32,
) -> Option<layer::ShapeLayer> {
    let draws_all_edges = edges.top && edges.leading && edges.bottom && edges.trailing;
    if draws_all_edges {
        if let Some(host_layer) = layer::layer_of(host) {
            layer::set_border(
                &host_layer,
                f64::from(width.max(0.0)),
                f64::from(corner_radius.max(0.0)),
                false,
            );
        }
        return None;
    }
    let shape = layer::ShapeLayer::new();
    shape.set_fill(None);
    shape.set_line_width(f64::from(width.max(0.0)));
    shape.set_line_cap_butt();
    if let Some(host_layer) = layer::layer_of(host) {
        layer::add_sublayer(&host_layer, shape.layer());
    }
    Some(shape)
}

fn apply_border_metrics(host: &PlatformView, state: &BorderState) {
    let width = f64::from(state.width.get().max(0.0));
    let corner_radius = f64::from(state.corner_radius.get().max(0.0));
    if let Some(shape) = &state.border_layer {
        shape.set_line_width(width);
        let bounds = view::bounds(host);
        if bounds.size.width > 0.0 && bounds.size.height > 0.0 {
            shape.set_frame(bounds);
            let mask = EdgeMask {
                top: state.edges.top,
                leading: state.edges.leading,
                bottom: state.edges.bottom,
                trailing: state.edges.trailing,
            };
            let path = border_path(bounds, width, corner_radius, mask);
            shape.set_path(Some(&path));
        }
    } else if let Some(host_layer) = layer::layer_of(host) {
        layer::set_border(&host_layer, width, corner_radius, false);
    }
    view::invalidate_captured_rendering(host);
}

fn watch_border_metric(
    leaf: &mut NativeLeaf,
    signal: &Computed<f32>,
    slot: impl Fn(&BorderState) -> &Cell<f32> + 'static,
    state: &Rc<BorderState>,
    host: &Retained<HostView>,
) {
    let state = Rc::clone(state);
    let host = host.clone();
    leaf.watch(signal, move |wctx| {
        slot(&state).set(*wctx.value());
        let state = Rc::clone(&state);
        let host = host.clone();
        with_platform_animation(wctx.metadata(), move || {
            apply_border_metrics(&host, &state);
        });
    });
}

/// The wrapper's layout face: the content's answers everywhere.
struct BorderSubView {
    /// The leaf's state.
    state: Rc<BorderState>,
}

impl core::fmt::Debug for BorderSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BorderSubView").finish_non_exhaustive()
    }
}

impl SubView for BorderSubView {
    fn resolve_extent(
        &self,
        bounds: LayoutRect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        self.state.child.layout().resolve_extent(bounds, proposal)
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.child.layout().priority()
    }

    fn is_empty(&self) -> bool {
        self.state.child.layout().is_empty()
    }
}

/// The `border` realization.
pub fn realize_border_metadata(
    metadata: Metadata<Border, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let border = metadata.value;
    let edges = border.get_edges();
    let initial_width = border.width().snapshot();
    let initial_corner_radius = border.get_corner_radius().snapshot();

    let mtm = ctx.mtm();
    let host = HostView::new(mtm, Rect::ZERO);
    layer::ensure_layer(&host);
    let child_leaf = ctx.realize(metadata.content);
    let inherited_clip = child_leaf.clip_shape_handle();
    let mounted = child_leaf.mount(&host);
    crate::primary_content::forward(&host, mounted.view());
    view::set_translates_autoresizing(mounted.view(), true);

    // `configureBorderLayer`: all four edges go through the layer's own
    // border properties; a partial edge set draws through a stroke
    // sublayer.
    let border_layer = configure_border_layer(&host, edges, initial_width, initial_corner_radius);

    let state = Rc::new(BorderState {
        child: mounted,
        border_layer,
        width: Cell::new(initial_width),
        corner_radius: Cell::new(initial_corner_radius),
        edges,
    });

    // The content fills the wrapper; the stroke layer covers it and
    // traces the selected edges.
    host.set_layout_handler({
        let state = Rc::clone(&state);
        move |host_view| {
            let state = &*state;
            let bounds = view::bounds(host_view);
            view::set_frame(state.child.view(), bounds);
            if let Some(shape) = &state.border_layer {
                shape.set_frame(bounds);
                let mask = EdgeMask {
                    top: state.edges.top,
                    leading: state.edges.leading,
                    bottom: state.edges.bottom,
                    trailing: state.edges.trailing,
                };
                let path = border_path(
                    bounds,
                    f64::from(state.width.get().max(0.0)),
                    f64::from(state.corner_radius.get().max(0.0)),
                    mask,
                );
                shape.set_path(Some(&path));
            }
        }
    });

    // `setPlacementProposal`: the proposal selected for this wrapper is
    // the proposal its content was negotiated with.
    let sink_guard = proposal::register_sink(&host, {
        let state = Rc::clone(&state);
        move |selected| {
            proposal::deliver(state.child.view(), selected);
        }
    });

    let mut leaf = NativeLeaf::new(
        &*host,
        BorderSubView {
            state: Rc::clone(&state),
        },
    );
    leaf.keep(sink_guard);
    leaf.set_clip_shape_handle(inherited_clip);

    let resolved = border.color().resolve(ctx.env());
    apply_border_color(&resolved.snapshot(), &host, state.border_layer.as_ref());
    leaf.watch(&resolved, {
        let state = Rc::clone(&state);
        let host = host.clone();
        move |wctx| {
            let color = *wctx.value();
            let state = Rc::clone(&state);
            let host = host.clone();
            with_platform_animation(wctx.metadata(), move || {
                apply_border_color(&color, &host, state.border_layer.as_ref());
            });
        }
    });
    watch_border_metric(&mut leaf, border.width(), |s| &s.width, &state, &host);
    watch_border_metric(
        &mut leaf,
        border.get_corner_radius(),
        |s| &s.corner_radius,
        &state,
        &host,
    );
    leaf.keep(state);
    leaf
}
