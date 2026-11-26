//! The `floating` metadata: `Metadata<Floating>` wrapped around elevated
//! interactive content.
//!
//! On Apple platforms a floating surface is a native elevated glass container:
//! a layer-backed `HostView` casting a subtle continuous-corner drop shadow,
//! holding a `GlassView` (`NSGlassEffectView` on `AppKit`, `UIVisualEffectView`
//! driven by an interactive `UIGlassEffect` on `UIKit`) with the content hosted
//! inside the glass view's own content view. The corner radius adapts to the
//! laid-out bounds — a capsule on compact controls (such as 44pt floating map
//! buttons) capped at 22pt continuous corners on larger containers — and the
//! host's shadow path tracks the same outline every layout pass.

use alloc::rc::Rc;
use core::fmt;

use cocoa_ui::glass::{GlassStyle, GlassView};
use cocoa_ui::layer;
use cocoa_ui::path::{CornerRadius as CocoaCornerRadius, ShapeKind as CocoaShapeKind, shape_path};
use cocoa_ui::{PlatformView, Rect, Size, view};
use waterui::floating::Floating;
use waterui_core::Metadata;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect as LayoutRect, ResolvedExtent, StretchAxis, SubView,
    ViewDimensions,
};

use crate::contract::{Held, Mounted, NativeLeaf, RenderContext};
use crate::primary_content;
use crate::proposal;
#[cfg(target_os = "macos")]
use crate::theme::SystemLabelSlots;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::colors;
}
#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::colors;
}

/// Maximum continuous corner radius for a native floating container (22pt =
/// full pill radius for a 44pt standard floating control).
const MAX_FLOATING_CORNER_RADIUS: f64 = 22.0;

/// Subtle ambient elevation shadow parameters complementing the native glass
/// rim on flat backgrounds.
const FLOATING_SHADOW_OFFSET_Y: f64 = 2.0;
const FLOATING_SHADOW_RADIUS: f64 = 8.0;
const FLOATING_SHADOW_OPACITY: f32 = 0.14;

/// Computes the continuous corner radius for a floating surface of `bounds`.
fn floating_corner_radius(bounds: Rect) -> f64 {
    let shorter = bounds.size.width.min(bounds.size.height).max(0.0);
    (shorter / 2.0).min(MAX_FLOATING_CORNER_RADIUS)
}

/// Updates both the glass view's continuous corner radius and the host layer's
/// matching shadow path for `bounds`.
fn apply_floating_geometry(host: &PlatformView, glass: &GlassView) {
    let bounds = view::bounds(host);
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return;
    }
    let radius = floating_corner_radius(bounds);
    glass.set_corner_radius(radius);
    if let Some(host_layer) = layer::layer_of(host) {
        let path = shape_path(
            CocoaShapeKind::RoundedRect {
                radii: [CocoaCornerRadius::Points(radius); 4],
            },
            &[],
            bounds,
        );
        layer::set_shadow_path(&host_layer, Some(&path));
    }
}

/// The leaf's live state: the mounted child the layout face forwards to.
struct FloatingState {
    /// The mounted content.
    child: Mounted,
}

impl fmt::Debug for FloatingState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FloatingState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: transparent — every answer the child's.
struct FloatingSubView {
    /// The leaf's state.
    state: Rc<FloatingState>,
}

impl fmt::Debug for FloatingSubView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FloatingSubView").finish_non_exhaustive()
    }
}

impl SubView for FloatingSubView {
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

/// Realizes `Metadata<Floating>` as a native elevated Apple glass container.
pub fn realize_floating_metadata(
    metadata: Metadata<Floating, Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let mtm = ctx.mtm();
    let host = HostView::new(mtm, Rect::ZERO);
    let host_view: &PlatformView = &host;

    layer::ensure_layer(host_view);
    if let Some(host_layer) = layer::layer_of(host_view) {
        let shadow_color = platform::colors::extended_linear_display_p3(0.0, 0.0, 0.0, 1.0);
        layer::set_masks_to_bounds(&host_layer, false);
        layer::set_shadow(
            &host_layer,
            Size::new(0.0, FLOATING_SHADOW_OFFSET_Y),
            FLOATING_SHADOW_RADIUS,
        );
        layer::set_shadow_color(&host_layer, Some(&platform::colors::cg(&shadow_color)));
        layer::set_shadow_opacity(&host_layer, FLOATING_SHADOW_OPACITY);
    }

    let glass = Rc::new(GlassView::new(mtm, GlassStyle::Regular, true));
    let glass_view = glass.platform_view();
    view::set_translates_autoresizing(glass_view, false);
    view::add_subview(host_view, glass_view);
    view::pin_edges(mtm, host_view, glass_view);

    #[cfg(target_os = "macos")]
    let child_leaf = if let Some(slots) = ctx.env().get::<SystemLabelSlots>().copied() {
        let mut env = ctx.env().clone();
        env.insert(slots.in_glass());
        ctx.with_env(&env).realize(metadata.content)
    } else {
        ctx.realize(metadata.content)
    };
    #[cfg(target_os = "ios")]
    let child_leaf = ctx.realize(metadata.content);

    let mounted = {
        #[cfg(target_os = "ios")]
        {
            let content_view = glass.content_view();
            let mounted = child_leaf.mount(&content_view);
            view::set_translates_autoresizing(mounted.view(), false);
            view::pin_edges(mtm, &content_view, mounted.view());
            mounted
        }
        #[cfg(target_os = "macos")]
        {
            let mounted = child_leaf.mount(glass_view);
            view::set_translates_autoresizing(mounted.view(), false);
            glass.set_content_view(mounted.view());
            view::pin_edges(mtm, glass_view, mounted.view());
            mounted
        }
    };
    primary_content::forward(&host, mounted.view());

    let state = Rc::new(FloatingState { child: mounted });

    host.set_layout_handler({
        let glass = Rc::clone(&glass);
        move |host_view| {
            apply_floating_geometry(host_view, &glass);
        }
    });

    let sink_guard = proposal::register_sink(host_view, {
        let state = Rc::clone(&state);
        move |selected| {
            proposal::deliver(state.child.view(), selected);
        }
    });

    let mut leaf = NativeLeaf::new(
        host_view,
        FloatingSubView {
            state: Rc::clone(&state),
        },
    );
    leaf.keep(sink_guard);
    leaf.keep(state);
    leaf.keep(glass);
    leaf
}
