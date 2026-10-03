//! The `ignore_safe_area` metadata: `Metadata<IgnoreSafeArea>` wrapped
//! around a child that extends into the screen edges its edges name.
//!
//! Mirrors `WuiIgnoreSafeArea`: a transparent `HostView` container —
//! measure, stretch, priority and the placement proposal all answer for the
//! mounted child — that answers `cocoaUiManagesSafeArea`, the
//! `WuiSafeAreaManaging` conformance the Swift leaf declared, so a holder
//! hands it the full bounds rather than the safe-area part. On `UIKit` the
//! wrapper reports its ignored edges through `cocoaUiIgnoredSafeAreaEdges`,
//! which the fallback's `wuiSafeAreaRect` erases on its ancestor walk —
//! the `erasingIgnoredEdges(from:)` the Swift leaf answered — and stops
//! `UIKit` from double-insetting its layout margins.

use alloc::rc::Rc;

use cocoa_ui::view;
use cocoa_ui::{PlatformView, Rect};
use waterui::layout::IgnoreSafeArea;
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The leaf's live state: the mounted child the layout face forwards to.
struct IgnoreState {
    /// The mounted content.
    child: Mounted,
}

impl core::fmt::Debug for IgnoreState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IgnoreState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: transparent — every answer the child's.
struct IgnoreSubView {
    /// The leaf's state.
    state: Rc<IgnoreState>,
}

impl core::fmt::Debug for IgnoreSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IgnoreSubView").finish_non_exhaustive()
    }
}

impl SubView for IgnoreSubView {
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

/// Installs the `ignore_safe_area` handler on the dispatcher:
/// `Metadata<IgnoreSafeArea>` maps to a transparent container that manages
/// its safe area and erases the named edges for its subtree.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<IgnoreSafeArea>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let host_view: &PlatformView = &host;

        let mounted = ctx.render(metadata.content).mount(host_view);
        view::set_translates_autoresizing(mounted.view(), true);

        // `WuiSafeAreaManaging`: the window root must hand this view the
        // full bounds — escaping the safe area is the component's purpose.
        host.set_manages_safe_area(true);

        #[cfg(target_os = "ios")]
        {
            let edges = metadata.value.edges;
            host.set_ignored_safe_area_edges(cocoa_ui::geometry::Edges::new(
                edges.top,
                edges.leading,
                edges.bottom,
                edges.trailing,
            ));
            // The wrapper manages the safe area itself; `UIKit`'s own margin
            // inset would double-count it.
            view::set_insets_layout_margins_from_safe_area(host_view, false);
        }
        #[cfg(target_os = "macos")]
        let _ = metadata.value;

        let state = Rc::new(IgnoreState { child: mounted });

        // iOS: `contentView.frame = wuiContentFrame(of:in:)` — the holder
        // extended this view to the edges it touches; the content lays out
        // against the insets that remain once the ignored edges are erased.
        #[cfg(target_os = "ios")]
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host| {
                let host_view: &PlatformView = host;
                let frame = crate::native_layout::content_frame(state.child.view(), host_view);
                view::set_frame(state.child.view(), frame);
            }
        });

        // macOS: safe area is less of a concern — `contentView.frame = bounds`.
        #[cfg(target_os = "macos")]
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host| {
                view::set_frame(state.child.view(), view::bounds(host));
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
            IgnoreSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);
        leaf.keep(state);
        leaf
    });
}
