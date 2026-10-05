//! The `on_key_press` metadata: `Metadata<OnKeyPress>` wrapped around a
//! child.
//!
//! Mirrors `WuiOnKeyPress`: a key arriving at the wrapper (`keyDown` on
//! `AppKit`, `pressesBegan` on `UIKit`) is translated to a `KeyPress`,
//! extended into the handler's environment, and `Handled` consumes it —
//! on `UIKit` only the leftover presses reach `super`.

use alloc::rc::Rc;
use core::cell::RefCell;

use cocoa_ui::Rect;
use cocoa_ui::keys::KeyEvent;
use cocoa_ui::view;
use waterui_backend_core::Environment;
use waterui_core::Metadata;
use waterui_core::key::{KeyHandling, KeyPress, OnKeyPress};
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The leaf's live state: the mounted child, the handler and the
/// environment it runs against.
struct OnKeyPressState {
    /// The mounted content.
    child: Mounted,
    /// The handler — `handle` borrows it mutably.
    handler: RefCell<OnKeyPress>,
    /// The environment the handler resolves through.
    env: Environment,
}

impl core::fmt::Debug for OnKeyPressState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OnKeyPressState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: the content's answers verbatim.
struct OnKeyPressSubView {
    /// The leaf's state.
    state: Rc<RefCell<OnKeyPressState>>,
}

impl core::fmt::Debug for OnKeyPressSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OnKeyPressSubView").finish_non_exhaustive()
    }
}

impl SubView for OnKeyPressSubView {
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

/// Installs the `on_key_press` handler on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<OnKeyPress>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let mounted = ctx.render(metadata.content).mount(&host);
        crate::primary_content::forward(&host, mounted.view());
        view::set_translates_autoresizing(mounted.view(), true);

        let state = Rc::new(RefCell::new(OnKeyPressState {
            child: mounted,
            handler: RefCell::new(metadata.value),
            env: ctx.env().clone(),
        }));

        // Each press carries the `KeyPress` in an extended environment;
        // `Handled` consumes it so it never reaches `super`.
        host.set_key_handler({
            let state = Rc::clone(&state);
            move |_, event: &KeyEvent| {
                let state = state.borrow();
                let env = state.env.extending(KeyPress {
                    key: event.key.clone(),
                    code: event.code,
                    modifiers: event.modifiers,
                    repeat: event.repeat,
                });
                matches!(
                    state.handler.borrow_mut().handle(&env),
                    KeyHandling::Handled
                )
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
            OnKeyPressSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);
        leaf.keep(state);
        leaf
    });
}
