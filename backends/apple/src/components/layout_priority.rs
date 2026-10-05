//! The `layout_priority` metadata: `Metadata<LayoutPriority>` wrapped
//! around a child.
//!
//! Mirrors `WuiLayoutPriority`: a transparent `HostView` container whose
//! measure, stretch and placement all answer for the mounted child — only
//! the priority reported to the parent's layout differs, the metadata
//! value replacing the child's own when space is distributed between
//! siblings.

use alloc::rc::Rc;
use core::cell::RefCell;

use cocoa_ui::Rect;
use cocoa_ui::view;
use waterui_core::Metadata;
use waterui_core::layout::{LayoutPriority, ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The leaf's live state: the mounted child the layout face forwards to.
struct PriorityState {
    /// The mounted content.
    child: Mounted,
}

impl core::fmt::Debug for PriorityState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PriorityState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: the content's answers everywhere but
/// priority.
struct PrioritySubView {
    /// The leaf's state.
    state: Rc<RefCell<PriorityState>>,
    /// The override — the metadata's own value.
    priority: i32,
}

impl core::fmt::Debug for PrioritySubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PrioritySubView").finish_non_exhaustive()
    }
}

impl SubView for PrioritySubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.borrow().child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.borrow().child.layout().stretch_axis()
    }

    /// `layoutPriority()`: the override, not the content's own.
    fn priority(&self) -> i32 {
        self.priority
    }
}

/// Installs the `layout_priority` handler on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<LayoutPriority>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let mounted = ctx.render(metadata.content).mount(&host);
        crate::primary_content::forward(&host, mounted.view());
        view::set_translates_autoresizing(mounted.view(), true);

        let state = Rc::new(RefCell::new(PriorityState { child: mounted }));

        // The content always fills the wrapper — `contentView.frame = bounds`.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host| {
                let state = state.borrow();
                view::set_frame(state.child.view(), view::bounds(host));
            }
        });

        // `setPlacementProposal`: the proposal selected for this wrapper is
        // the proposal its content was negotiated with.
        let sink_guard = proposal::register_sink(&host, {
            let state = Rc::clone(&state);
            move |selected| {
                let state = state.borrow();
                proposal::deliver(state.child.view(), selected);
            }
        });

        let mut leaf = NativeLeaf::new(
            &*host,
            PrioritySubView {
                state: Rc::clone(&state),
                priority: metadata.value.get(),
            },
        );
        leaf.keep(sink_guard);
        leaf.keep(state);
        leaf
    });
}
