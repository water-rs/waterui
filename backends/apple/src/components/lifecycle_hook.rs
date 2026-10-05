//! The `lifecycle_hook` metadata: `Metadata<LifeCycleHook>` wrapped around
//! a child, firing a one-shot handler on `Appear` or `Disappear`.
//!
//! Mirrors `WuiLifecycleHook`: a transparent `HostView` container —
//! measure, stretch, priority and the placement proposal all answer for the
//! mounted child — that watches its window membership. `Disappear` fires
//! inside the move-out-of-window callback; `Appear` instead waits for the
//! current implicit transaction's completion block, the commit boundary the
//! Swift leaf documented: a hook that animates a property away from its
//! initial value must not run before the first frame, or the view lands on
//! screen already at the animation's end state. The handler is one-shot —
//! consumed on firing, dropped unfired with the leaf.

use alloc::rc::Rc;
use core::cell::RefCell;

use cocoa_ui::view;
use cocoa_ui::{PlatformView, Rect};
use waterui_backend_core::Environment;
use waterui_core::Metadata;
use waterui_core::event::{LifeCycle, LifeCycleHook};
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The leaf's live state: the mounted child the layout face forwards to.
struct LifecycleState {
    /// The mounted content.
    child: Mounted,
}

impl core::fmt::Debug for LifecycleState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LifecycleState").finish_non_exhaustive()
    }
}

/// The one-shot hook and the environment it fires under.
struct HookState {
    /// The hook until it fires — `self.handler = nil` in the Swift leaf.
    hook: RefCell<Option<LifeCycleHook>>,
    /// The environment the hook is invoked with.
    env: Environment,
}

impl core::fmt::Debug for HookState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HookState").finish_non_exhaustive()
    }
}

/// `handle(_:)`: the hook fires only for its own event and only once —
/// firing consumes it.
fn fire(state: &HookState, event: LifeCycle) {
    let matches = state
        .hook
        .borrow()
        .as_ref()
        .is_some_and(|hook| hook.lifecycle() == event);
    if matches && let Some(hook) = state.hook.borrow_mut().take() {
        hook.handle(&state.env);
    }
}

/// `scheduleAppear`: `Appear` means presented, so it fires after the
/// transaction that inserted the view commits — the completion block of the
/// current implicit transaction is that boundary.
fn schedule_appear(state: &Rc<HookState>, host: &HostView) {
    let is_appear = state
        .hook
        .borrow()
        .as_ref()
        .is_some_and(|hook| hook.lifecycle() == LifeCycle::Appear);
    if !is_appear {
        return;
    }
    let state = Rc::downgrade(state);
    let host = view::retain_base(host);
    cocoa_ui::core_animation::on_commit(move || {
        let Some(state) = state.upgrade() else {
            return;
        };
        // The Swift leaf re-checked `window != nil` inside the completion
        // block: a view that left its window before commit is a disappear,
        // not an appear.
        if view::has_window(&host) {
            fire(&state, LifeCycle::Appear);
        }
    });
}

/// The wrapper's layout face: transparent — every answer the child's.
struct LifecycleSubView {
    /// The leaf's state.
    state: Rc<LifecycleState>,
}

impl core::fmt::Debug for LifecycleSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LifecycleSubView").finish_non_exhaustive()
    }
}

impl SubView for LifecycleSubView {
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

/// Installs the `lifecycle_hook` handler on the dispatcher:
/// `Metadata<LifeCycleHook>` maps to a transparent container that fires its
/// one-shot handler on the matching window-membership change.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<LifeCycleHook>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let host_view: &PlatformView = &host;

        let hook = Rc::new(HookState {
            hook: RefCell::new(Some(metadata.value)),
            env: ctx.env().clone(),
        });

        let mounted = ctx.render(metadata.content).mount(host_view);
        crate::primary_content::forward(&host, mounted.view());
        view::set_translates_autoresizing(mounted.view(), true);

        let state = Rc::new(LifecycleState { child: mounted });

        // `contentView.frame = bounds`.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host| {
                view::set_frame(state.child.view(), view::bounds(host));
            }
        });

        // `windowMembershipChanged`: leaving a window is `Disappear`;
        // entering one schedules `Appear` on the transaction's commit.
        host.set_window_handler({
            let hook = Rc::clone(&hook);
            move |host| {
                if view::has_window(host) {
                    schedule_appear(&hook, host);
                } else {
                    fire(&hook, LifeCycle::Disappear);
                }
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
            LifecycleSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);
        leaf.keep(state);
        leaf.keep(hook);
        leaf
    });
}
