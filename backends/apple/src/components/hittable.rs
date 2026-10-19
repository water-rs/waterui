//! The `hittable` metadata: `Metadata<Hittable>` wrapped around a child.
//!
//! Mirrors `WuiHittable`: a transparent `HostView` whose hit test returns
//! nothing while `enabled` is false, so touches fall through to whatever
//! lies beneath. On `UIKit` the content's `isUserInteractionEnabled`
//! follows the same flag; on `AppKit` the override alone decides.

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

use cocoa_ui::Rect;
use cocoa_ui::view;
use waterui::interaction::Hittable;
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HitTest, HostView};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{HitTest, HostView};

/// The leaf's live state: the mounted child and the current flag.
struct HittableState {
    /// The mounted content.
    child: Mounted,
    /// Whether hits reach the content — `currentEnabled`.
    enabled: Cell<bool>,
}

impl core::fmt::Debug for HittableState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HittableState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: the content's answers verbatim.
struct HittableSubView {
    /// The leaf's state.
    state: Rc<RefCell<HittableState>>,
}

impl core::fmt::Debug for HittableSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HittableSubView").finish_non_exhaustive()
    }
}

impl SubView for HittableSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.borrow().child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.borrow().child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.borrow().child.layout().priority()
    }
}

/// Installs the `hittable` handler on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<Hittable>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let mounted = ctx.render(metadata.content).mount(&host);
        crate::primary_content::forward(&host, mounted.view());
        view::set_translates_autoresizing(mounted.view(), true);

        let state = Rc::new(RefCell::new(HittableState {
            child: mounted,
            enabled: Cell::new(true),
        }));

        // `hitTest`: disabled means the hit passes through the wrapper.
        host.set_hit_test_handler({
            let state = Rc::clone(&state);
            move |_, _| {
                if state.borrow().enabled.get() {
                    HitTest::Default
                } else {
                    HitTest::Pass
                }
            }
        });

        // The content always fills the wrapper.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |host| {
                let state = state.borrow();
                view::set_frame(state.child.view(), view::bounds(host));
            }
        });

        // `setPlacementProposal` forwards to the content.
        let sink_guard = proposal::register_sink(&host, {
            let state = Rc::clone(&state);
            move |selected| {
                let state = state.borrow();
                proposal::deliver(state.child.view(), selected);
            }
        });

        let mut leaf = NativeLeaf::new(
            &*host,
            HittableSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);

        // `enabled` watcher → `applyHitTesting`.
        leaf.bind(&metadata.value.enabled, {
            let state = Rc::clone(&state);
            move |enabled| {
                let state = state.borrow();
                state.enabled.set(enabled);
                // `isUserInteractionEnabled` on the content — `UIKit` only.
                #[cfg(target_os = "ios")]
                view::set_user_interaction_enabled(state.child.view(), enabled);
            }
        });

        leaf.keep(state);
        leaf
    });
}
