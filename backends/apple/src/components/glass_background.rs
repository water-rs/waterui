//! The `glass_background` metadata: `IgnorableMetadata<GlassBackground>`
//! wrapped around content Liquid Glass sits behind.
//!
//! Mirrors `WuiGlassBackground`: a transparent `HostView` container —
//! measure, stretch, priority and the placement proposal all answer for the
//! mounted child — with the platform's glass surface pinned to its bounds
//! (`UIVisualEffectView` driven by a `UIGlassEffect` on `UIKit`,
//! `NSGlassEffectView` on `AppKit`) and the content hosted inside the
//! effect's own content view. The outline is the effect's, not a mask's: a
//! layer mask over glass takes its refraction and highlights with it, so
//! the shape arrives as a corner configuration rebuilt on every layout
//! pass. Tint is a resolved-color signal pushed imperatively onto the
//! effect.

use alloc::rc::Rc;

use cocoa_ui::glass::{GlassStyle, GlassView};
use cocoa_ui::view;
use cocoa_ui::{PlatformView, Rect, Retained};
use waterui::background::{Glass, GlassBackground};
use waterui::graphics::color::WorkingColor;
use waterui::shape::ShapeKind;
use waterui_core::IgnorableMetadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The `Glass` style as the kit's neutral style.
const fn style(glass: &Glass) -> GlassStyle {
    match glass.style() {
        waterui::background::GlassStyle::Regular => GlassStyle::Regular,
        waterui::background::GlassStyle::Clear => GlassStyle::Clear,
    }
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object.
#[cfg(target_os = "ios")]
fn platform_color(color: &WorkingColor) -> Retained<cocoa_ui::objc2_ui_kit::UIColor> {
    {
        let [red, green, blue, alpha] = color.components;
        cocoa_ui::uikit::colors::extended_linear_display_p3(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object, with HDR
/// headroom applied as a content-headroom multiplier — the `AppKit`
/// variant.
#[cfg(target_os = "macos")]
fn platform_color(color: &WorkingColor) -> Retained<cocoa_ui::objc2_app_kit::NSColor> {
    {
        let [red, green, blue, alpha] = color.components;
        cocoa_ui::appkit::colors::extended_linear_display_p3(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

/// The outline as the glass view's corner configuration, rebuilt against
/// the current bounds — `WuiGlassBackground.cornerConfiguration` on
/// `UIKit`.
#[cfg(target_os = "ios")]
fn apply_outline(glass: &GlassView, shape: ShapeKind, bounds: Rect) {
    let shorter = bounds.size.width.min(bounds.size.height);
    let limit = shorter / 2.0;
    match shape {
        ShapeKind::Rect => glass.set_corner_radius(0.0),
        ShapeKind::Circle | ShapeKind::Capsule => glass.set_corner_capsule(),
        ShapeKind::RoundedRect { corner_radius } => {
            glass.set_corner_radius((f64::from(corner_radius) * shorter).min(limit));
        }
        ShapeKind::FixedRoundedRect { corner_radius } => {
            glass.set_corner_radius(f64::from(corner_radius).min(limit));
        }
        ShapeKind::UnevenRoundedRect {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        } => {
            glass.set_corner_radii(
                (f64::from(top_left) * shorter).min(limit),
                (f64::from(top_right) * shorter).min(limit),
                (f64::from(bottom_left) * shorter).min(limit),
                (f64::from(bottom_right) * shorter).min(limit),
            );
        }
        ShapeKind::FixedUnevenRoundedRect {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        } => {
            glass.set_corner_radii(
                f64::from(top_left).min(limit),
                f64::from(top_right).min(limit),
                f64::from(bottom_left).min(limit),
                f64::from(bottom_right).min(limit),
            );
        }
        ShapeKind::Ellipse | ShapeKind::CustomPath => {
            panic!(
                "WaterUI glass takes its outline from a corner configuration; {shape:?} (ellipse or custom path) cannot be a glass outline"
            );
        }
    }
}

/// The outline as the glass view's uniform corner radius —
/// `NSGlassEffectView` draws one radius for all four corners.
#[cfg(target_os = "macos")]
fn apply_outline(glass: &GlassView, shape: ShapeKind, bounds: Rect) {
    let shorter = bounds.size.width.min(bounds.size.height);
    let limit = shorter / 2.0;
    match shape {
        ShapeKind::Rect => glass.set_corner_radius(0.0),
        ShapeKind::Circle | ShapeKind::Capsule => glass.set_corner_radius(limit),
        ShapeKind::RoundedRect { corner_radius } => {
            glass.set_corner_radius((f64::from(corner_radius) * shorter).min(limit));
        }
        ShapeKind::FixedRoundedRect { corner_radius } => {
            glass.set_corner_radius(f64::from(corner_radius).min(limit));
        }
        ShapeKind::Ellipse
        | ShapeKind::UnevenRoundedRect { .. }
        | ShapeKind::FixedUnevenRoundedRect { .. }
        | ShapeKind::CustomPath => {
            panic!(
                "WaterUI glass on macOS takes one corner radius; {shape:?} (ellipse, per-corner radii, or custom path) cannot be a glass outline"
            );
        }
    }
}

/// The leaf's live state: the mounted child the layout face forwards to.
struct GlassBackgroundState {
    /// The mounted content.
    child: Mounted,
}

impl core::fmt::Debug for GlassBackgroundState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GlassBackgroundState")
            .finish_non_exhaustive()
    }
}

/// The wrapper's layout face: transparent — every answer the child's.
struct GlassBackgroundSubView {
    /// The leaf's state.
    state: Rc<GlassBackgroundState>,
}

impl core::fmt::Debug for GlassBackgroundSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GlassBackgroundSubView")
            .finish_non_exhaustive()
    }
}

impl SubView for GlassBackgroundSubView {
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

/// Installs the `glass_background` handler on the dispatcher:
/// `IgnorableMetadata<GlassBackground>` maps to a transparent container
/// with the platform's glass surface behind the content.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<IgnorableMetadata<GlassBackground>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let host_view: &PlatformView = &host;

        let glass_config = &metadata.value.0;
        let shape = glass_config.shape_kind();
        let glass = Rc::new(GlassView::new(
            mtm,
            style(glass_config),
            glass_config.is_interactive(),
        ));

        #[cfg(target_os = "macos")]
        let _backing_layer = cocoa_ui::shape::layer(host_view);

        // The glass pinned to the wrapper's bounds, the content inside the
        // effect's own content view — the platform expects it there; a
        // subview beside the effect gets none of the glass's treatment.
        let glass_view = glass.platform_view();
        view::set_translates_autoresizing(glass_view, false);
        view::add_subview(host_view, glass_view);
        view::pin_edges(mtm, host_view, glass_view);

        let child_leaf = ctx.render(metadata.content);
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

        let state = Rc::new(GlassBackgroundState { child: mounted });

        // Each layout pass re-resolves the corner configuration against the
        // current bounds — the outline is the effect's, so it must follow
        // the frame like `WuiGlassBackground.layoutSubviews`/`layout`.
        host.set_layout_handler({
            let glass = Rc::clone(&glass);
            move |host| {
                apply_outline(&glass, shape, view::bounds(host));
            }
        });

        // `setPlacementProposal`: the proposal selected for this wrapper is
        // the proposal its content was negotiated with.
        let sink_guard = proposal::register_sink(host_view, {
            let state = Rc::clone(&state);
            move |selected| {
                proposal::deliver(state.child.view(), selected);
            }
        });

        let mut leaf = NativeLeaf::new(
            host_view,
            GlassBackgroundSubView {
                state: Rc::clone(&state),
            },
        );

        // The tint is scheme-aware: resolve through the environment and
        // push each resolved color onto the effect imperatively.
        if let Some(tint) = glass_config.tint_color() {
            let resolved = tint.clone().resolve(ctx.env());
            let glass = Rc::clone(&glass);
            leaf.bind(&resolved, move |color| {
                glass.set_tint_color(&platform_color(&color));
            });
        }

        leaf.keep(sink_guard);
        leaf.keep(state);
        leaf.keep(glass);
        leaf
    });
}
