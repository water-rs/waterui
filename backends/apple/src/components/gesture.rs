//! The `gesture` metadata: `Metadata<GestureObserver>` wrapped around a
//! child.
//!
//! Mirrors `WuiGesture`: a transparent `HostView` container — measure,
//! stretch and placement all answer for the mounted child — that attaches a
//! platform recognizer per gesture to itself, taps through
//! `UITapGestureRecognizer`/`NSClickGestureRecognizer`, long-presses through
//! `UILongPressGestureRecognizer`/`NSPressGestureRecognizer`, drags through
//! the pan recognizers, and pinch/rotate through the two-finger
//! recognizers. The observer's action fires on the same state transitions
//! `WuiGesture` used: taps, drags, pinches and rotations on `ended`,
//! long-presses on `began`.
//!
//! Composed gestures mirror too: `Then` arms the second recognizer once the
//! first fires, `Simultaneous` fires on either, and `Exclusive` fires at
//! most once per 50 ms window.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use cocoa_ui::Rect;
use cocoa_ui::gesture::{ButtonMask, GestureState};
use cocoa_ui::view;
use waterui::gesture::{Gesture, GestureObserver};
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::gesture::GestureAttachment;
#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HostView, gesture as kit};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::gesture::GestureAttachment;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{HostView, gesture as kit};

/// The `WATERUI_POINTER_BUTTON_*` mask a gesture's `buttons` field already
/// is: `PointerButtons` and [`ButtonMask`] both follow the DOM `buttons`
/// bit order, so the mask applies verbatim.
const fn button_mask(buttons: waterui::gesture::PointerButtons) -> ButtonMask {
    ButtonMask::from_bits(buttons.bits())
}

/// The leaf's live state: the mounted child the layout face forwards to.
struct GestureLeafState {
    /// The mounted content.
    child: Mounted,
}

impl core::fmt::Debug for GestureLeafState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GestureLeafState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: the content's own answers everywhere.
struct GestureSubView {
    /// The leaf's state.
    state: Rc<RefCell<GestureLeafState>>,
}

impl core::fmt::Debug for GestureSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GestureSubView").finish_non_exhaustive()
    }
}

impl SubView for GestureSubView {
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

type Recognized = Rc<dyn Fn()>;

/// A recognizer handler firing `on_recognized` only on `state`.
fn fire_on(state: GestureState, on_recognized: &Recognized) -> impl Fn(GestureState) + 'static {
    let on_recognized = Rc::clone(on_recognized);
    move |current| {
        if current == state {
            on_recognized();
        }
    }
}

/// Attaches the recognizers `gesture` describes to `host`, in `WuiGesture`'s
/// per-kind configurations; `on_recognized` runs when the gesture fires.
/// The returned attachments keep the recognizers' targets alive.
#[expect(
    clippy::too_many_lines,
    reason = "the match mirrors the baseline's per-kind attach switch"
)]
fn attach(host: &HostView, gesture: &Gesture, on_recognized: Recognized) -> Vec<GestureAttachment> {
    match gesture {
        Gesture::Tap(tap) => {
            #[cfg(target_os = "ios")]
            {
                vec![kit::tap(
                    host,
                    tap.count as usize,
                    button_mask(tap.buttons),
                    fire_on(GestureState::Ended, &on_recognized),
                )]
            }
            #[cfg(target_os = "macos")]
            {
                vec![kit::click(
                    host,
                    tap.count as usize,
                    button_mask(tap.buttons),
                    fire_on(GestureState::Ended, &on_recognized),
                )]
            }
        }
        Gesture::LongPress(long_press) => {
            let seconds = f64::from(long_press.duration) / 1000.0;
            let buttons = button_mask(long_press.buttons);
            #[cfg(target_os = "ios")]
            {
                vec![kit::long_press(
                    host,
                    seconds,
                    buttons,
                    fire_on(GestureState::Began, &on_recognized),
                )]
            }
            #[cfg(target_os = "macos")]
            {
                vec![kit::press(
                    host,
                    seconds,
                    buttons,
                    fire_on(GestureState::Began, &on_recognized),
                )]
            }
        }
        Gesture::Drag(drag) => vec![kit::pan(
            host,
            button_mask(drag.buttons),
            fire_on(GestureState::Ended, &on_recognized),
        )],
        Gesture::Magnification(_) => {
            #[cfg(target_os = "ios")]
            {
                vec![kit::pinch(
                    host,
                    fire_on(GestureState::Ended, &on_recognized),
                )]
            }
            #[cfg(target_os = "macos")]
            {
                vec![kit::magnification(
                    host,
                    fire_on(GestureState::Ended, &on_recognized),
                )]
            }
        }
        Gesture::Rotation(_) => vec![kit::rotation(
            host,
            fire_on(GestureState::Ended, &on_recognized),
        )],
        Gesture::Then(then) => {
            let armed = Rc::new(Cell::new(false));
            let mut attachments = attach(host, then.first(), {
                let armed = Rc::clone(&armed);
                Rc::new(move || armed.set(true))
            });
            attachments.extend(attach(host, then.then(), {
                let armed = Rc::clone(&armed);
                let on_recognized = Rc::clone(&on_recognized);
                Rc::new(move || {
                    if armed.replace(false) {
                        on_recognized();
                    }
                })
            }));
            attachments
        }
        Gesture::Simultaneous(pair) => {
            let mut attachments = attach(host, pair.first(), Rc::clone(&on_recognized));
            attachments.extend(attach(host, pair.second(), on_recognized));
            attachments
        }
        Gesture::Exclusive(pair) => {
            // `WuiGesture` resolves an exclusive pair at most once per 50 ms
            // window: the first recognizer to fire in a window wins it.
            let last_fired_at = Rc::new(Cell::new(None));
            let resolve = {
                let last_fired_at = Rc::clone(&last_fired_at);
                Rc::new(move || {
                    let now = std::time::Instant::now();
                    if last_fired_at.get().is_none_or(|last: std::time::Instant| {
                        now.duration_since(last).as_secs_f64() > 0.05
                    }) {
                        last_fired_at.set(Some(now));
                        on_recognized();
                    }
                }) as Recognized
            };
            let mut attachments = attach(host, pair.first(), Rc::clone(&resolve));
            attachments.extend(attach(host, pair.second(), resolve));
            attachments
        }
        _ => Vec::new(),
    }
}

/// Installs the `gesture` handler on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<GestureObserver>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let mounted = ctx.render(metadata.content).mount(&host);
        crate::primary_content::forward(&host, mounted.view());
        view::set_translates_autoresizing(mounted.view(), true);

        let state = Rc::new(RefCell::new(GestureLeafState { child: mounted }));

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

        let env = ctx.env().clone();
        let action = Rc::new(RefCell::new(metadata.value.action));
        let on_recognized: Recognized = Rc::new(move || {
            (action.borrow_mut())(&env);
        });
        let attachments = attach(&host, &metadata.value.gesture, on_recognized);

        let mut leaf = NativeLeaf::new(
            &*host,
            GestureSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);
        leaf.keep(state);
        leaf.keep(attachments);
        leaf
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui::gesture::PointerButtons;

    #[test]
    fn button_mask_carries_pointer_button_bits_verbatim() {
        assert_eq!(button_mask(PointerButtons::PRIMARY).bits(), 1);
        assert_eq!(button_mask(PointerButtons::SECONDARY).bits(), 2);
        assert_eq!(
            button_mask(PointerButtons::PRIMARY | PointerButtons::FORWARD).bits(),
            0b10001
        );
    }
}
