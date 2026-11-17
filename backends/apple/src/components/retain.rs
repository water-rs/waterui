//! The `retain` metadata: `Metadata<Retain>` wrapped around a child,
//! keeping an opaque value alive for the view's lifetime.
//!
//! Mirrors `WuiRetain`: a transparent `HostView` container — measure,
//! stretch, priority and the placement proposal all answer for the mounted
//! child — whose only effect is holding the metadata's value in the leaf's
//! `KeepAlive`, the `WuiRetainValue`/`waterui_drop_retain` pair the Swift
//! leaf ran: the value lives exactly as long as the view does.

use alloc::rc::Rc;

use cocoa_ui::view;
use cocoa_ui::{PlatformView, Rect};
use waterui_core::Metadata;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect as LayoutRect, ResolvedExtent, StretchAxis, SubView,
    ViewDimensions,
};
use waterui_core::metadata::Retain;

use crate::contract::{Mounted, NativeLeaf};
use crate::proposal;

use crate::contract::RenderContext;
#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The leaf's live state: the mounted child the layout face forwards to.
struct RetainState {
    /// The mounted content.
    child: Mounted,
}

impl core::fmt::Debug for RetainState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RetainState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: transparent — every answer the child's.
struct RetainSubView {
    /// The leaf's state.
    state: Rc<RetainState>,
}

impl core::fmt::Debug for RetainSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RetainSubView").finish_non_exhaustive()
    }
}

impl SubView for RetainSubView {
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

/// The `retain` realization: `Metadata<Retain>` maps
/// to a transparent container that holds the retained value for the leaf's
/// life.
pub fn realize_retain_metadata(metadata: Metadata<Retain>, ctx: &RenderContext<'_>) -> NativeLeaf {
    let mtm = ctx.mtm();
    let host = HostView::new(mtm, Rect::ZERO);
    let host_view: &PlatformView = &host;

    // The guard is transparent to a transition's adoption: the conditional
    // a `watch` wraps in it adopts the declaration over the guard.
    if let Some(offer) = ctx.offer() {
        offer.pass_on();
    }
    let child = ctx.render(metadata.content);
    // A `Retain::resolved` value is built from the environment its
    // content's modifier chain resolves in, as a handler's is.
    let env = child.handler_env(ctx.env());
    let mut retained = metadata.value;
    retained.resolve(&env);
    let mounted = child.mount(host_view);

    view::set_translates_autoresizing(mounted.view(), true);

    let state = Rc::new(RetainState { child: mounted });

    // `contentView.frame = bounds`.
    host.set_layout_handler({
        let state = Rc::clone(&state);
        move |host| {
            view::set_frame(state.child.view(), view::bounds(host));
        }
    });

    // `WuiPrimaryContentProviding`: the primary-content chain descends
    // into the content — a scroll surface under a retain wrapper keeps
    // managing its own safe area instead of being framed inside it.
    crate::primary_content::forward(&host, state.child.view());

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
        RetainSubView {
            state: Rc::clone(&state),
        },
    );
    leaf.keep(sink_guard);
    leaf.keep(state);
    // `WuiRetainValue`: the opaque value dies with the view.
    leaf.keep(retained);
    // Transparent to handler resolution: a handler wrapped around this
    // leaf resolves through its content's chain.
    leaf.set_handler_env(env);
    leaf
}
