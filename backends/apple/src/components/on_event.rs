//! The `on_event` metadata: `Metadata<OnEvent>` wrapped around a child.
//!
//! Mirrors `WuiOnEvent`: pointer enter / move / exit on the wrapper
//! (`NSTrackingArea` on `AppKit`, `UIHoverGestureRecognizer` on `UIKit`)
//! fire the handler whose subscribed event matches — enter and exit run
//! it against the view's environment, moves extend it with the
//! `HoverEvent` point first.

use alloc::rc::Rc;
use core::cell::RefCell;

use cocoa_ui::Rect;
use cocoa_ui::view;
use waterui_backend_core::Environment;
use waterui_core::Metadata;
use waterui_core::event::{Event, HoverEvent, OnEvent};
use waterui_core::layout::{Point, ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
use cocoa_ui::pointer::{PointerEvent, PointerEvents};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// The leaf's live state: the mounted child, the subscribed event, the
/// handler, and the environment it runs against.
struct OnEventState {
    /// The mounted content.
    child: Mounted,
    /// Which pointer event the handler answers.
    event: Event,
    /// The handler — `handle` borrows it mutably.
    handler: RefCell<OnEvent>,
    /// The environment the handler resolves through.
    env: Environment,
}

impl core::fmt::Debug for OnEventState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OnEventState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: the content's answers verbatim.
struct OnEventSubView {
    /// The leaf's state.
    state: Rc<RefCell<OnEventState>>,
}

impl core::fmt::Debug for OnEventSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OnEventSubView").finish_non_exhaustive()
    }
}

impl SubView for OnEventSubView {
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

/// `handleHoverEnter` / `handleHoverExit` / `handleHoverMove`: run the
/// handler only when the delivered event is the subscribed one.
fn fire(state: &OnEventState, event: Event, location: Option<Point>) {
    if state.event != event {
        return;
    }
    let env = location.map_or_else(
        || state.env.clone(),
        |point| state.env.extending(HoverEvent::new(point)),
    );
    state.handler.borrow_mut().handle(&env);
}

/// Installs the `on_event` handler on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<OnEvent>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let mounted = ctx.render(metadata.content).mount(&host);
        crate::primary_content::forward(&host, mounted.view());
        view::set_translates_autoresizing(mounted.view(), true);

        let state = Rc::new(RefCell::new(OnEventState {
            child: mounted,
            event: metadata.value.event(),
            handler: RefCell::new(metadata.value),
            env: ctx.env().clone(),
        }));

        // All three pointer channels: `fire` filters by the subscribed
        // event, so the mask is always `ALL`-minus-cursor.
        host.set_pointer_handler(
            PointerEvents::ENTERED
                .union(PointerEvents::MOVED)
                .union(PointerEvents::EXITED),
            {
                let state = Rc::clone(&state);
                move |_, event| {
                    let state = state.borrow();
                    #[allow(clippy::cast_possible_truncation)]
                    match event {
                        PointerEvent::Entered => fire(&state, Event::HoverEnter, None),
                        PointerEvent::Moved(point) => fire(
                            &state,
                            Event::HoverMove,
                            Some(waterui_backend_core::Point::new(
                                point.x as f32,
                                point.y as f32,
                            )),
                        ),
                        PointerEvent::Exited => fire(&state, Event::HoverExit, None),
                        PointerEvent::CursorUpdate => {}
                    }
                    false
                }
            },
        );

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
            OnEventSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);
        leaf.keep(state);
        leaf
    });
}
