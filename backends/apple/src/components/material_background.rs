//! The `material_background` metadata: `IgnorableMetadata<MaterialBackground>`
//! wrapped around content a blur effect sits behind.
//!
//! Mirrors `WuiMaterialBackground`: a transparent `HostView` container —
//! measure, stretch, priority and the placement proposal all answer for the
//! mounted child — holding the platform's effect view (`UIVisualEffectView`
//! with a `UIBlurEffect` on `UIKit`, `NSVisualEffectView` on `AppKit`) plus
//! the content. On `AppKit` the content is a subview of the effect view, so
//! text in a system label color draws vibrant, and a thick material follows
//! the Reduce Transparency setting live; on `UIKit` both are direct
//! subviews of the container. Each layout pass centers the effect and
//! the content over the content's negotiated size: a width-constrained
//! content keeps its narrower material column while the wrapper may be
//! stamped wider.

use alloc::rc::Rc;
use core::cell::Cell;

use cocoa_ui::material;
use cocoa_ui::view;
use cocoa_ui::{PlatformView, Rect, Retained};
use waterui::background::MaterialBackground;
use waterui_core::IgnorableMetadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal::{self, Placement};
use crate::window_background::level;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The leaf's live state: the mounted child and the effect view, plus the
/// last placement the wrapper was given — its proposal is the offer the
/// layout pass echoes back when the host stamps the frame without
/// delivering a proposal.
struct MaterialBackgroundState {
    /// The mounted content.
    child: Mounted,
    /// The effect view, stamped over the content's frame each layout.
    blur: Retained<PlatformView>,
    /// The placement given to this wrapper, forwarded to the child.
    placement: Cell<Placement>,
}

impl core::fmt::Debug for MaterialBackgroundState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MaterialBackgroundState")
            .finish_non_exhaustive()
    }
}

/// The wrapper's layout face: transparent — every answer the child's.
struct MaterialBackgroundSubView {
    /// The leaf's state.
    state: Rc<MaterialBackgroundState>,
}

impl core::fmt::Debug for MaterialBackgroundSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MaterialBackgroundSubView")
            .finish_non_exhaustive()
    }
}

impl SubView for MaterialBackgroundSubView {
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

/// Installs the `material_background` handler on the dispatcher:
/// `IgnorableMetadata<MaterialBackground>` maps to a transparent container
/// with the platform's blur effect behind the content.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<IgnorableMetadata<MaterialBackground>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let host_view: &PlatformView = &host;

        let material = material::material_view(mtm, level(metadata.value.0));
        let blur = view::retain_base(material.view());

        // The thick material's flat fill follows the system's Reduce
        // Transparency setting live; the guard lives with the leaf.
        #[cfg(target_os = "macos")]
        let reduce_transparency = material.follow_system_reduce_transparency(mtm);

        // `wantsLayer = true`: `NSVisualEffectView` expects a layer-backed
        // host on `AppKit`.
        #[cfg(target_os = "macos")]
        let _backing_layer = cocoa_ui::shape::layer(host_view);

        // The blur behind the content, laid out by the layout handler, as
        // `translatesAutoresizingMaskIntoConstraints` on
        // `WuiMaterialBackground`. On `AppKit` the content is a subview of
        // the effect view, so text drawn in a system label color is
        // vibrant; `UIKit` keeps both as siblings.
        view::add_subview(host_view, &blur);
        #[cfg(target_os = "macos")]
        let content_parent: &PlatformView = &blur;
        #[cfg(target_os = "ios")]
        let content_parent: &PlatformView = host_view;
        let mounted = ctx.render(metadata.content).mount(content_parent);
        view::set_translates_autoresizing(&blur, true);
        view::set_translates_autoresizing(mounted.view(), true);

        // A material's whole painted surface is the blur — one fill.
        // `UIKit`: the fill rule extends it into the touched edge's regions
        // when a background slot holds it. `AppKit`: it stays a safe-area
        // manager so the blur still runs behind the window chrome.
        host.set_is_fill(true);
        #[cfg(target_os = "macos")]
        host.set_manages_safe_area(true);

        let state = Rc::new(MaterialBackgroundState {
            child: mounted,
            blur,
            placement: Cell::new(Placement::UNPLACED),
        });

        // Scroll discovery traverses the content without changing the
        // material's safe-area boundary.
        #[cfg(target_os = "ios")]
        host.set_scroll_surface_handler({
            let state = Rc::clone(&state);
            move |_host| vec![view::retain_base(state.child.view())]
        });

        // `.background(material)` covers exactly the view it backs: the
        // content's own measurement under the delivered proposal is the
        // frame both the effect and the content get, centered in bounds.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "AppKit measures in `CGFloat`; the leaf speaks `f32`"
            )]
            move |host| {
                let bounds = view::bounds(host);
                let last = state
                    .placement
                    .get()
                    .proposal
                    .unwrap_or(ProposalSize::UNSPECIFIED);
                let size = state
                    .child
                    .layout()
                    .measure(ProposalSize::new(
                        last.width.or(Some(bounds.size.width as f32)),
                        last.height.or(Some(bounds.size.height as f32)),
                    ))
                    .size;
                let rect = Rect::new(
                    (bounds.size.width - f64::from(size.width)) / 2.0,
                    (bounds.size.height - f64::from(size.height)) / 2.0,
                    f64::from(size.width),
                    f64::from(size.height),
                );
                // The content centers on its negotiated size; the blur is
                // the fill surface and covers the whole extended frame.
                view::set_frame(state.child.view(), rect);
                view::set_frame(&state.blur, bounds);
            }
        });

        // `setPlacementProposal`: remember the placement and forward it to
        // the content, as `WuiMaterialBackground.lastProposal`.
        let sink_guard = proposal::register_sink(host_view, {
            let state = Rc::clone(&state);
            move |placement| {
                state.placement.set(placement);
                proposal::deliver(state.child.view(), placement);
            }
        });

        let mut leaf = NativeLeaf::new(
            host_view,
            MaterialBackgroundSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);
        #[cfg(target_os = "macos")]
        leaf.keep(reduce_transparency);
        leaf.keep(state);
        leaf
    });
}
