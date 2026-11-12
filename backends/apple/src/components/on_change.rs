//! The `on_change` metadata: `Metadata<OnChange>` wrapped around a child,
//! running a handler whenever a signal's value changes.
//!
//! A transparent `HostView` container — measure, stretch, priority and the
//! placement proposal all answer for the mounted child — that subscribes the
//! handler once, against the environment its content's modifier chain
//! resolves in, and holds the subscription in the leaf's `KeepAlive`: the
//! handler runs exactly as long as the view is mounted.

use alloc::rc::Rc;

use cocoa_ui::view;
use cocoa_ui::{PlatformView, Rect};
use waterui_core::Metadata;
use waterui_core::event::OnChange;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect as LayoutRect, ResolvedExtent, StretchAxis, SubView,
    ViewDimensions,
};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The leaf's live state: the mounted child the layout face forwards to.
struct OnChangeState {
    /// The mounted content.
    child: Mounted,
}

impl core::fmt::Debug for OnChangeState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OnChangeState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: transparent — every answer the child's.
struct OnChangeSubView {
    /// The leaf's state.
    state: Rc<OnChangeState>,
}

impl core::fmt::Debug for OnChangeSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OnChangeSubView").finish_non_exhaustive()
    }
}

impl SubView for OnChangeSubView {
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

/// Installs the `on_change` handler on the dispatcher: `Metadata<OnChange>`
/// maps to a transparent container that holds the handler's subscription for
/// the leaf's life.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<OnChange>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let host_view: &PlatformView = &host;

        let child = ctx.render(metadata.content);
        // The handler resolves in the environment its content's modifier
        // chain resolves in, as an `on_event` handler does.
        let env = child.handler_env(ctx.env());
        let mounted = child.mount(host_view);

        view::set_translates_autoresizing(mounted.view(), true);

        let state = Rc::new(OnChangeState { child: mounted });

        // The content always fills the wrapper.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host| {
                view::set_frame(state.child.view(), view::bounds(host));
            }
        });

        // The primary-content chain descends into the content — a scroll
        // surface under the wrapper keeps managing its own safe area.
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
            OnChangeSubView {
                state: Rc::clone(&state),
            },
        );
        // Subscribe once the content is mounted, so a handler that runs on a
        // later change finds the content's subscriptions in place.
        leaf.keep(metadata.value.subscribe(&env));
        leaf.set_handler_env(env);
        leaf.keep(sink_guard);
        leaf.keep(state);
        leaf
    });
}
