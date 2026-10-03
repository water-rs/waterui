//! The `focused` metadata: `Metadata<Focused>` wrapped around a child that
//! owns exactly one focus anchor.
//!
//! Mirrors `WuiFocused`: a transparent container (layout, measure and
//! priority all answer for the mounted child) whose `Binding<bool>` is
//! two-way with the platform — writes become `request`/`clear` first
//! responder calls on the child's single [`FocusTarget`], and the
//! platform's own focus changes write back into the binding.
//!
//! Sync is window-gated and coalesced: a request made before the view is
//! in a window — or fired from inside another notification — is deferred
//! onto the main queue, the same `WuiFocusedBindingController` model, and
//! re-driven when the container next moves into a window.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::Cell;

use cocoa_ui::view::bounds;
use cocoa_ui::{PlatformView, Rect, Retained, focus, view};
use waterui::component::focus::Focused;
use waterui::reactive::Signal;
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The leaf's live state: the mounted child the layout face forwards to.
struct FocusedState {
    child: Mounted,
}

/// The two-way focus controller `WuiFocusedBindingController` played: the
/// container the sync gates on, the anchor it drives, the binding it
/// mirrors, and the flag coalescing deferred syncs.
struct FocusSync {
    /// The wrapper's own view; both it and the target must be in a window.
    container: Retained<PlatformView>,
    /// The child's single focus anchor.
    target: focus::FocusTarget,
    /// The `Focused` binding: requested focus out, platform focus back in.
    binding: waterui::reactive::Binding<bool>,
    /// Whether a deferred sync is already queued on the main queue.
    scheduled: Cell<bool>,
}

impl core::fmt::Debug for FocusedState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FocusedState").finish_non_exhaustive()
    }
}

/// Schedules a coalesced sync on the main queue — `syncRequestedFocusState`:
/// the flag dedups re-entrant requests and the deferral lets the current
/// responder transition settle before the next one is asked for.
fn request_sync(sync: &Rc<FocusSync>, mtm: cocoa_ui::MainThreadMarker) {
    if sync.scheduled.replace(true) {
        return;
    }
    let sync = Rc::downgrade(sync);
    cocoa_ui::main_queue::enqueue_local(mtm, move |_mtm| {
        let Some(sync) = sync.upgrade() else {
            return;
        };
        sync.scheduled.set(false);
        perform_sync(&sync);
    });
}

/// Drives the platform toward the binding's value — `performScheduledSync`:
/// nothing happens while either the container or the anchor is outside a
/// window; a refused request is the same `fatalError` the Swift port raised.
fn perform_sync(sync: &FocusSync) {
    if !focus::in_window(&sync.container) || !focus::in_window(sync.target.view()) {
        return;
    }
    if sync.binding.snapshot() {
        if !sync.target.has_focus() {
            assert!(
                sync.target.request_focus(),
                "Metadata<Focused> failed to focus its resolved TextField/SecureField anchor."
            );
        }
    } else if sync.target.has_focus() {
        assert!(
            sync.target.clear_focus(),
            "Metadata<Focused> failed to blur its resolved TextField/SecureField anchor."
        );
    }
}

/// The wrapper's layout face: transparent, every answer the child's —
/// `WuiFocused` forwards `stretchAxis`, `layoutPriority` and `measure`
/// unchanged.
struct FocusedSubView {
    state: Rc<FocusedState>,
}

impl core::fmt::Debug for FocusedSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FocusedSubView").finish_non_exhaustive()
    }
}

impl SubView for FocusedSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.child.layout().priority()
    }
}

/// Installs the `focused` handler on the dispatcher: `Metadata<Focused>`
/// maps to a transparent container that keeps the child's focus anchor
/// and the `Binding<bool>` in sync both ways.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<Focused>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let host_view: &PlatformView = &host;
        let mounted = ctx.render(metadata.content).mount(host_view);
        crate::primary_content::forward(&host, mounted.view());

        // The wrapper lays its child out over its full bounds —
        // `WuiFocused`'s `contentView.frame = bounds`.
        host.set_layout_handler({
            let child = view::retain_base(mounted.view());
            move |view| view::set_frame(&child, bounds(view))
        });

        // `requireSingleWuiFocusTarget`: exactly one anchor in the subtree.
        let target = require_single_target(focus::targets_in(host_view));

        let sync = Rc::new(FocusSync {
            container: view::retain_base(host_view),
            target,
            binding: metadata.value.0,
            scheduled: Cell::new(false),
        });

        // `didMoveToWindow`/`viewDidMoveToWindow` re-drives the sync: a
        // request that arrived before the window did is replayed on attach.
        host.set_window_handler({
            let sync = Rc::clone(&sync);
            move |_| request_sync(&sync, mtm)
        });

        let state = Rc::new(FocusedState { child: mounted });

        let mut leaf = NativeLeaf::new(
            host_view,
            FocusedSubView {
                state: Rc::clone(&state),
            },
        );

        // The objects the watchers fire on are kept first so the guards
        // — kept after — drop before them (reverse insertion order).
        leaf.keep(state);
        leaf.keep(Rc::clone(&sync));

        // Platform → binding: an editing-begin/end on the anchor writes
        // back when it disagrees, so a user-driven focus change lands in
        // the binding instead of being overwritten.
        leaf.keep(sync.target.on_change({
            let sync = Rc::clone(&sync);
            move |has_focus| {
                if sync.binding.snapshot() != has_focus {
                    sync.binding.set(has_focus);
                }
            }
        }));

        // Binding → platform: every external write schedules a sync.
        let binding = sync.binding.clone();
        leaf.watch(&binding, {
            let sync = Rc::clone(&sync);
            move |_ctx| request_sync(&sync, mtm)
        });

        // The initial value's sync: deferred until the view reaches a
        // window, as `syncRequestedFocusState` already is.
        request_sync(&sync, mtm);

        leaf
    });
}

/// `requireSingleWuiFocusTarget`: `Metadata<Focused>` wraps exactly one
/// focus anchor — none, or several, is the same `fatalError` the Swift
/// wrapper raised.
fn require_single_target(mut anchors: Vec<focus::FocusTarget>) -> focus::FocusTarget {
    match anchors.len() {
        1 => anchors.pop().unwrap_or_else(|| unreachable!()),
        0 => panic!(
            "Metadata<Focused> requires exactly one TextField or SecureField focus anchor in its subtree, found 0."
        ),
        count => panic!(
            "Metadata<Focused> requires exactly one TextField or SecureField focus anchor in its subtree, found {count}."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No anchor in the subtree is the first `requireSingleWuiFocusTarget`
    /// trap.
    #[test]
    #[should_panic(expected = "found 0")]
    fn zero_anchors_panic() {
        let _ = require_single_target(Vec::new());
    }
}
