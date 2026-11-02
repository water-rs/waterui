//! The `gesture` metadata: `Metadata<GestureObserver>` wrapped around a
//! child.
//!
//! Mirrors `WuiGesture`: a transparent `HostView` container — measure,
//! stretch and placement all answer for the mounted child — that attaches a
//! platform recognizer per gesture to itself, taps through
//! `UITapGestureRecognizer`/`NSClickGestureRecognizer`, long-presses through
//! `UILongPressGestureRecognizer`/`NSPressGestureRecognizer`, drags through
//! the pan recognizers, and pinch/rotate through the two-finger
//! recognizers. The observer's action receives the environment the shared
//! gesture contract defines — the observed `Gesture` plus the completing
//! recognizer's event (`TapEvent`, `LongPressEvent`, `DragEvent`,
//! `MagnificationEvent`, `RotationEvent`), with locations and centers in
//! the host's local space and translations and velocities as vectors. A
//! tap fires on `ended` and a long-press on `began`; the continuous
//! gestures fire on every recognizer state — `began`, `changed`, `ended`
//! and `cancelled` — as the matching event `phase`.
//!
//! Composed gestures mirror too: `Then` arms the second recognizer once the
//! first fires, `Simultaneous` fires on either, and `Exclusive` fires at
//! most once per 50 ms window.
//!
//! A composition containing a `Tap` also makes the host operable to
//! assistive clients — `AXPress` on an `AXGroup` on `AppKit`,
//! `accessibilityActivate` on a `Button`-trait element whose descendants'
//! accessibility merges into it on `UIKit` — and runs the same action
//! the tap recognizer fires.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use cocoa_ui::Rect;
use cocoa_ui::gesture::{ButtonMask, GestureState};
#[cfg(target_os = "macos")]
use cocoa_ui::objc2::MainThreadOnly as _;
#[cfg(target_os = "macos")]
use cocoa_ui::objc2_app_kit::{NSApplication, NSEventType};
use cocoa_ui::view;
use waterui::gesture::{
    DragEvent, Gesture, GestureObserver, GesturePhase, GesturePoint, LongPressEvent,
    MagnificationEvent, PointerButton, RotationEvent, TapEvent,
};
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

use cocoa_ui::accessibility::{self as a11y, Role};
#[cfg(target_os = "macos")]
use cocoa_ui::appkit::gesture::GestureAttachment;
#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HostView, gesture as kit};
#[cfg(target_os = "macos")]
use cocoa_ui::objc2::runtime::ProtocolObject;
#[cfg(target_os = "macos")]
use cocoa_ui::objc2_app_kit::{NSAccessibility, NSView};
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

/// The event a recognized gesture inserts into the action's environment —
/// the variant the completing recognizer produced. A composed gesture
/// delivers the child that completed it, the way `WuiGesture` did and the
/// shared gesture contract requires.
enum GesturePayload {
    Tap(TapEvent),
    LongPress(LongPressEvent),
    Drag(DragEvent),
    Magnification(MagnificationEvent),
    Rotation(RotationEvent),
}

type Recognized = Rc<dyn Fn(GesturePayload)>;

/// A point `AppKit`/`UIKit` reports — `NSPoint`/`CGPoint`, both `f64` pairs —
/// as a [`GesturePoint`].
#[expect(
    clippy::cast_possible_truncation,
    reason = "the gesture contract is f32; kit geometry is f64"
)]
const fn gesture_point(x: f64, y: f64) -> GesturePoint {
    GesturePoint::new(x as f32, y as f32)
}

/// The [`GesturePhase`] a recognizer's `state` stands for: `Began` opens a
/// continuous gesture, `Changed` updates it, `Ended` completes it and
/// `Cancelled` aborts it; `Possible` and `Failed` deliver nothing.
const fn phase_of(state: GestureState) -> Option<GesturePhase> {
    match state {
        GestureState::Began => Some(GesturePhase::Started),
        GestureState::Changed => Some(GesturePhase::Updated),
        GestureState::Ended => Some(GesturePhase::Ended),
        GestureState::Cancelled => Some(GesturePhase::Cancelled),
        _ => None,
    }
}

/// The [`DragEvent`] a pan recognizer's report stands for, or `None` while
/// the translation has not crossed `min_distance` — the threshold the
/// shared contract gates a drag on — and after an ended sequence.
fn pan_event(
    started: &Cell<bool>,
    min_distance: f64,
    state: GestureState,
    location: (f64, f64),
    translation: (f64, f64),
    velocity: (f64, f64),
    button: PointerButton,
) -> Option<GesturePayload> {
    let mut phase = phase_of(state)?;
    if !started.get() {
        if matches!(phase, GesturePhase::Ended | GesturePhase::Cancelled)
            || translation.0.hypot(translation.1) < min_distance
        {
            // Released before the threshold: the drag failed and delivers
            // nothing.
            return None;
        }
        started.set(true);
        phase = GesturePhase::Started;
    }
    if matches!(phase, GesturePhase::Ended | GesturePhase::Cancelled) {
        started.set(false);
    }
    // A cancelled drag reports no velocity, matching the shared contract.
    let velocity = if phase == GesturePhase::Cancelled {
        GesturePoint::new(0.0, 0.0)
    } else {
        gesture_point(velocity.0, velocity.1)
    };
    Some(GesturePayload::Drag(DragEvent::new(
        phase,
        gesture_point(location.0, location.1),
        gesture_point(translation.0, translation.1),
        velocity,
        button,
    )))
}

/// The lowest button in a `ButtonMask`'s DOM-ordered bits, the button a
/// recognizer configured with the mask was necessarily pressed by when the
/// press itself does not say.
const fn lowest_button(buttons: ButtonMask) -> PointerButton {
    let bits = buttons.bits();
    if bits & ButtonMask::PRIMARY.bits() != 0 {
        PointerButton::Primary
    } else if bits & ButtonMask::SECONDARY.bits() != 0 {
        PointerButton::Secondary
    } else if bits & ButtonMask::MIDDLE.bits() != 0 {
        PointerButton::Middle
    } else if bits & ButtonMask::BACK.bits() != 0 {
        PointerButton::Back
    } else {
        PointerButton::Forward
    }
}

/// The button the recognizer's press carried: `buttonMask` reports the
/// mask of the event that drove recognition in the same DOM order
/// [`ButtonMask`] uses — a touches event carries no button bits, and a
/// contact is a primary press.
#[cfg(target_os = "ios")]
fn pressed_button(recognizer: &cocoa_ui::objc2_ui_kit::UIGestureRecognizer) -> PointerButton {
    let mask = recognizer.buttonMask().0;
    if mask == 0 {
        // A touches contact presses as the primary button.
        PointerButton::Primary
    } else {
        lowest_button(ButtonMask::from_bits(
            u8::try_from(mask & 0xff).unwrap_or_default(),
        ))
    }
}

/// The button the recognizer's press carried: the current `AppKit` event
/// is the mouse event driving the recognizer, and `buttonNumber` names the
/// button (`0` primary, `1` secondary, `2` middle, `3`/`4` back/forward);
/// anything else falls back to the lowest button the recognizer accepts.
#[cfg(target_os = "macos")]
fn pressed_button(
    recognizer: &cocoa_ui::objc2_app_kit::NSGestureRecognizer,
    buttons: ButtonMask,
) -> PointerButton {
    NSApplication::sharedApplication(recognizer.mtm())
        .currentEvent()
        .filter(|event| {
            matches!(
                event.r#type(),
                NSEventType::LeftMouseDown
                    | NSEventType::LeftMouseUp
                    | NSEventType::RightMouseDown
                    | NSEventType::RightMouseUp
                    | NSEventType::OtherMouseDown
                    | NSEventType::OtherMouseUp
                    | NSEventType::LeftMouseDragged
                    | NSEventType::RightMouseDragged
                    | NSEventType::OtherMouseDragged
            )
        })
        .map_or_else(
            || lowest_button(buttons),
            |event| match event.buttonNumber() {
                1 => PointerButton::Secondary,
                2 => PointerButton::Middle,
                3 => PointerButton::Back,
                4 => PointerButton::Forward,
                _ => PointerButton::Primary,
            },
        )
}

/// Attaches the recognizers `gesture` describes to `host`, in `WuiGesture`'s
/// per-kind configurations; `on_recognized` runs with the recognized
/// gesture's event when the gesture fires. The returned attachments keep
/// the recognizers' targets alive.
#[expect(
    clippy::too_many_lines,
    reason = "the match mirrors the baseline's per-kind attach switch"
)]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "the gesture contract is f32; kit geometry is f64, and a press duration in milliseconds fits an f32 mantissa"
)]
fn attach(host: &HostView, gesture: &Gesture, on_recognized: Recognized) -> Vec<GestureAttachment> {
    match gesture {
        Gesture::Tap(tap) => {
            let buttons = button_mask(tap.get_buttons());
            let count = tap.count();
            #[cfg(target_os = "ios")]
            {
                vec![kit::tap(host, count as usize, buttons, {
                    move |state, recognizer| {
                        if state == GestureState::Ended {
                            let location = recognizer.locationInView(recognizer.view().as_deref());
                            on_recognized(GesturePayload::Tap(TapEvent::new(
                                gesture_point(location.x, location.y),
                                count,
                                pressed_button(recognizer),
                            )));
                        }
                    }
                })]
            }
            #[cfg(target_os = "macos")]
            {
                vec![kit::click(host, count as usize, buttons, {
                    move |state, recognizer| {
                        if state == GestureState::Ended {
                            let location = recognizer.locationInView(recognizer.view().as_deref());
                            on_recognized(GesturePayload::Tap(TapEvent::new(
                                gesture_point(location.x, location.y),
                                count,
                                pressed_button(recognizer, buttons),
                            )));
                        }
                    }
                })]
            }
        }
        Gesture::LongPress(long_press) => {
            let seconds = f64::from(long_press.duration()) / 1000.0;
            let duration = long_press.duration() as f32;
            let buttons = button_mask(long_press.get_buttons());
            #[cfg(target_os = "ios")]
            {
                vec![kit::long_press(host, seconds, buttons, {
                    move |state, recognizer| {
                        if state == GestureState::Began {
                            let location = recognizer.locationInView(recognizer.view().as_deref());
                            on_recognized(GesturePayload::LongPress(LongPressEvent::new(
                                gesture_point(location.x, location.y),
                                duration,
                                pressed_button(recognizer),
                            )));
                        }
                    }
                })]
            }
            #[cfg(target_os = "macos")]
            {
                vec![kit::press(host, seconds, buttons, {
                    move |state, recognizer| {
                        if state == GestureState::Began {
                            let location = recognizer.locationInView(recognizer.view().as_deref());
                            on_recognized(GesturePayload::LongPress(LongPressEvent::new(
                                gesture_point(location.x, location.y),
                                duration,
                                pressed_button(recognizer, buttons),
                            )));
                        }
                    }
                })]
            }
        }
        Gesture::Drag(drag) => {
            let min_distance = f64::from(drag.min_distance());
            let buttons = button_mask(drag.get_buttons());
            let started = Rc::new(Cell::new(false));
            macro_rules! pan_handler {
                ($rec:ident, $recognizer:ty, $button:expr) => {{
                    let started = Rc::clone(&started);
                    move |state, $rec: &$recognizer| {
                        let view = $rec.view();
                        let translation = $rec.translationInView(view.as_deref());
                        let location = $rec.locationInView(view.as_deref());
                        let velocity = $rec.velocityInView(view.as_deref());
                        if let Some(event) = pan_event(
                            &started,
                            min_distance,
                            state,
                            (location.x, location.y),
                            (translation.x, translation.y),
                            (velocity.x, velocity.y),
                            $button,
                        ) {
                            on_recognized(event);
                        }
                    }
                }};
            }
            #[cfg(target_os = "ios")]
            {
                vec![kit::pan(
                    host,
                    buttons,
                    pan_handler!(
                        recognizer,
                        cocoa_ui::objc2_ui_kit::UIPanGestureRecognizer,
                        pressed_button(recognizer)
                    ),
                )]
            }
            #[cfg(target_os = "macos")]
            {
                vec![kit::pan(
                    host,
                    buttons,
                    pan_handler!(
                        recognizer,
                        cocoa_ui::objc2_app_kit::NSPanGestureRecognizer,
                        pressed_button(recognizer, buttons)
                    ),
                )]
            }
        }
        Gesture::Magnification(magnification) => {
            let initial_scale = magnification.initial_scale();
            // Both platforms report a cumulative factor since the gesture
            // began — `UIPinchGestureRecognizer.scale` directly, and `1 +
            // NSMagnificationGestureRecognizer.magnification` on `AppKit`,
            // whose `magnification` is the current fractional
            // magnification, not an incremental delta. The shared contract
            // composes a relative per-update delta onto a running scale
            // with the `.01` minimum, so each update folds `current /
            // previous - 1` in exactly once; `dt` is the monotonic
            // `Instant` interval between callbacks.
            macro_rules! magnification_handler {
                ($rec:ident, $factor:expr, $velocity:expr) => {{
                    let sample = Cell::new((initial_scale, 1.0f64, std::time::Instant::now()));
                    move |state, $rec| {
                        let Some(phase) = phase_of(state) else {
                            return;
                        };
                        let native_factor: f64 = $factor;
                        let now = std::time::Instant::now();
                        let (previous_scale, previous_factor, previous_at) = sample.get();
                        let dt = now
                            .duration_since(previous_at)
                            .as_secs_f32()
                            .max(f32::EPSILON);
                        let (scale, velocity) = match phase {
                            GesturePhase::Started => (initial_scale, 0.0),
                            GesturePhase::Updated => {
                                let relative = (native_factor / previous_factor - 1.0) as f32;
                                (
                                    (previous_scale * (1.0 + relative)).max(0.01),
                                    $velocity(relative, dt),
                                )
                            }
                            _ => (previous_scale, 0.0),
                        };
                        sample.set((scale, native_factor, now));
                        let center = $rec.locationInView($rec.view().as_deref());
                        on_recognized(GesturePayload::Magnification(MagnificationEvent::new(
                            phase,
                            gesture_point(center.x, center.y),
                            scale,
                            velocity,
                        )));
                    }
                }};
            }
            #[cfg(target_os = "ios")]
            {
                // `UIPinchGestureRecognizer.velocity` is the scale's
                // per-second rate, so the contract's `delta / dt`
                // relative rate is `velocity / scale`.
                vec![kit::pinch(
                    host,
                    magnification_handler!(
                        recognizer,
                        recognizer.scale(),
                        |_relative: f32, _dt: f32| {
                            (recognizer.velocity() / recognizer.scale()) as f32
                        }
                    ),
                )]
            }
            #[cfg(target_os = "macos")]
            {
                vec![kit::magnification(
                    host,
                    magnification_handler!(
                        recognizer,
                        1.0 + recognizer.magnification(),
                        |relative: f32, dt: f32| relative / dt
                    ),
                )]
            }
        }
        Gesture::Rotation(_) => {
            // The contract's rotation angle accumulates the delta from the
            // gesture's start — what `UIRotationGestureRecognizer.rotation`
            // and `NSRotationGestureRecognizer.rotation` both already
            // report in radians (`rotationInDegrees` is the separate
            // degree getter).
            let last = Rc::new(Cell::new((0.0f32, std::time::Instant::now())));
            macro_rules! rotation_handler {
                ($rec:ident, $angle:expr, $velocity:expr) => {{
                    let last = Rc::clone(&last);
                    move |state, $rec| {
                        let Some(phase) = phase_of(state) else {
                            return;
                        };
                        let now = std::time::Instant::now();
                        let (previous, previous_at) = last.replace(($angle, now));
                        let velocity = if phase == GesturePhase::Updated {
                            $velocity(previous, now.duration_since(previous_at).as_secs_f32())
                        } else {
                            0.0
                        };
                        let center = $rec.locationInView($rec.view().as_deref());
                        on_recognized(GesturePayload::Rotation(RotationEvent::new(
                            phase,
                            gesture_point(center.x, center.y),
                            $angle,
                            velocity,
                        )));
                    }
                }};
            }
            #[cfg(target_os = "ios")]
            {
                vec![kit::rotation(
                    host,
                    rotation_handler!(
                        recognizer,
                        recognizer.rotation() as f32,
                        |_previous, _dt| recognizer.velocity() as f32
                    ),
                )]
            }
            #[cfg(target_os = "macos")]
            {
                // `NSRotationGestureRecognizer` reports radians and no
                // velocity: the angle delta between updates over their
                // wall-clock interval.
                vec![kit::rotation(
                    host,
                    rotation_handler!(
                        recognizer,
                        recognizer.rotation() as f32,
                        |previous: f32, dt: f32| {
                            (recognizer.rotation() as f32 - previous) / dt.max(f32::EPSILON)
                        }
                    ),
                )]
            }
        }
        Gesture::Then(then) => {
            let armed = Rc::new(Cell::new(false));
            let mut attachments = attach(host, then.first(), {
                let armed = Rc::clone(&armed);
                Rc::new(move |_| armed.set(true))
            });
            attachments.extend(attach(host, then.then(), {
                let armed = Rc::clone(&armed);
                let on_recognized = Rc::clone(&on_recognized);
                Rc::new(move |payload| {
                    if armed.replace(false) {
                        on_recognized(payload);
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
                Rc::new(move |payload| {
                    let now = std::time::Instant::now();
                    if last_fired_at.get().is_none_or(|last: std::time::Instant| {
                        now.duration_since(last).as_secs_f64() > 0.05
                    }) {
                        last_fired_at.set(Some(now));
                        on_recognized(payload);
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

/// Whether an assistive client's activation — `AXPress` on `AppKit`,
/// `accessibilityActivate` on `UIKit` — activates the gesture's action: a
/// `Tap` anywhere in the composition is what makes the host operable;
/// long-presses, drags and the two-finger gestures describe activations a
/// press does not stand in for.
fn activates_on_press(gesture: &Gesture) -> bool {
    match gesture {
        Gesture::Tap(_) => true,
        Gesture::Then(then) => activates_on_press(then.first()) || activates_on_press(then.then()),
        Gesture::Simultaneous(pair) => {
            activates_on_press(pair.first()) || activates_on_press(pair.second())
        }
        Gesture::Exclusive(pair) => {
            activates_on_press(pair.first()) || activates_on_press(pair.second())
        }
        _ => false,
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
        let (observer_gesture, action) = metadata.value.into_parts();
        let activates = activates_on_press(&observer_gesture);
        let attach_gesture = observer_gesture.clone();
        let action = Rc::new(RefCell::new(action));
        let on_recognized: Recognized = Rc::new(move |payload| {
            // The shared gesture contract: the observed `Gesture` and the
            // completing recognizer's event go into the action's
            // environment, so a handler extracts the same values on every
            // backend.
            let mut action_env = env.clone();
            action_env.insert(observer_gesture.clone());
            match payload {
                GesturePayload::Tap(event) => action_env.insert(event),
                GesturePayload::LongPress(event) => action_env.insert(event),
                GesturePayload::Drag(event) => action_env.insert(event),
                GesturePayload::Magnification(event) => action_env.insert(event),
                GesturePayload::Rotation(event) => action_env.insert(event),
            }
            (action.borrow_mut())(&action_env);
        });

        // An assistive client's activation — `AXPress` on `AppKit`,
        // `accessibilityActivate` on `UIKit` — on the wrapper runs the
        // gesture's action: a `Tap` anywhere in the composition is what
        // makes the host operable (water-rs/waterui#1490,
        // water-rs/waterui#2975). The synthesized event is a primary tap
        // at the host's center, the point an assistive activation stands
        // for.
        if activates {
            let on_recognized = Rc::clone(&on_recognized);
            let activate = move |activated_host: &HostView| {
                let bounds = view::bounds(activated_host);
                on_recognized(GesturePayload::Tap(TapEvent::new(
                    gesture_point(
                        bounds.origin.x + bounds.size.width / 2.0,
                        bounds.origin.y + bounds.size.height / 2.0,
                    ),
                    1,
                    PointerButton::Primary,
                )));
                true
            };
            #[cfg(target_os = "macos")]
            {
                host.set_accessibility_press_handler(activate);
                let host_view: &NSView = &host;
                a11y::set_role(
                    ProtocolObject::<dyn NSAccessibility>::from_ref(host_view),
                    Role::Group,
                );
            }
            #[cfg(target_os = "ios")]
            {
                host.set_accessibility_activate_handler(activate);
                a11y::set_role(&host, Role::Button);
                // A `Button` element reads as a leaf to `VoiceOver`, so
                // the content's labels and nested activatable elements
                // merge into the host (SwiftUI `children: .combine`).
                host.set_combines_accessibility_children(true);
            }
        }

        let attachments = attach(&host, &attach_gesture, on_recognized);

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
    use waterui::gesture::{DragGesture, LongPressGesture, PointerButtons, TapGesture};

    #[test]
    fn button_mask_carries_pointer_button_bits_verbatim() {
        assert_eq!(button_mask(PointerButtons::PRIMARY).bits(), 1);
        assert_eq!(button_mask(PointerButtons::SECONDARY).bits(), 2);
        assert_eq!(
            button_mask(PointerButtons::PRIMARY | PointerButtons::FORWARD).bits(),
            0b10001
        );
    }

    /// `Began` opens a phase, `Changed` continues it, `Ended` and
    /// `Cancelled` close it; `Possible` and `Failed` report nothing.
    #[test]
    fn phase_of_maps_the_recognizer_states() {
        assert_eq!(phase_of(GestureState::Began), Some(GesturePhase::Started));
        assert_eq!(phase_of(GestureState::Changed), Some(GesturePhase::Updated));
        assert_eq!(phase_of(GestureState::Ended), Some(GesturePhase::Ended));
        assert_eq!(
            phase_of(GestureState::Cancelled),
            Some(GesturePhase::Cancelled)
        );
        assert_eq!(phase_of(GestureState::Possible), None);
        assert_eq!(phase_of(GestureState::Failed), None);
    }

    /// The DOM-ordered mask falls to its lowest set button.
    #[test]
    fn lowest_button_walks_the_dom_bit_order() {
        let mask = |bits| ButtonMask::from_bits(bits);
        assert_eq!(lowest_button(mask(1)), PointerButton::Primary);
        assert_eq!(lowest_button(mask(0b10)), PointerButton::Secondary);
        assert_eq!(lowest_button(mask(0b110)), PointerButton::Secondary);
        assert_eq!(lowest_button(mask(0b100)), PointerButton::Middle);
        assert_eq!(lowest_button(mask(0b1000)), PointerButton::Back);
        assert_eq!(lowest_button(mask(0b11000)), PointerButton::Back);
        assert_eq!(lowest_button(mask(0b10000)), PointerButton::Forward);
    }

    /// The event a pan report resolves to — `None` while the drag is
    /// gated out.
    fn drag_of(payload: Option<GesturePayload>) -> Option<DragEvent> {
        Some(match payload? {
            GesturePayload::Drag(event) => event,
            _ => panic!("a pan report resolves to a drag event"),
        })
    }

    /// Below `min_distance` a changed report stays gated; crossing it
    /// opens the drag as `Started`; later changes are `Updated`; `Ended`
    /// closes the sequence and disarms the gate.
    #[test]
    fn pan_event_gates_on_min_distance_and_maps_phases() {
        let started = Cell::new(false);
        let report = |state, translation| {
            drag_of(pan_event(
                &started,
                10.0,
                state,
                (20.0, 30.0),
                translation,
                (100.0, -50.0),
                PointerButton::Primary,
            ))
        };
        assert!(
            report(GestureState::Changed, (5.0, 0.0)).is_none(),
            "a sub-threshold translation delivers nothing"
        );
        let opened = report(GestureState::Changed, (15.0, 0.0))
            .expect("crossing the threshold opens the drag");
        assert_eq!(opened.phase(), GesturePhase::Started);
        assert_eq!(opened.location(), GesturePoint::new(20.0, 30.0));
        assert_eq!(opened.translation(), GesturePoint::new(15.0, 0.0));
        assert_eq!(opened.velocity(), GesturePoint::new(100.0, -50.0));
        assert_eq!(opened.button(), PointerButton::Primary);

        let updated =
            report(GestureState::Changed, (25.0, 0.0)).expect("an open drag reports its changes");
        assert_eq!(updated.phase(), GesturePhase::Updated);

        let ended = report(GestureState::Ended, (25.0, 0.0)).expect("an open drag reports its end");
        assert_eq!(ended.phase(), GesturePhase::Ended);
        assert!(
            report(GestureState::Ended, (25.0, 0.0)).is_none(),
            "an ended sequence disarms the gate"
        );
    }

    /// A cancelled drag reports `Cancelled` with no velocity — the shared
    /// contract's cancelled report — then disarms.
    #[test]
    fn pan_event_cancels_without_velocity() {
        let started = Cell::new(false);
        let report = |state, translation, velocity| {
            drag_of(pan_event(
                &started,
                10.0,
                state,
                (0.0, 0.0),
                translation,
                velocity,
                PointerButton::Primary,
            ))
        };
        report(GestureState::Changed, (15.0, 0.0), (0.0, 0.0)).unwrap();
        let cancelled = report(GestureState::Cancelled, (15.0, 0.0), (500.0, 0.0))
            .expect("an open drag reports its cancellation");
        assert_eq!(cancelled.phase(), GesturePhase::Cancelled);
        assert_eq!(cancelled.velocity(), GesturePoint::new(0.0, 0.0));
        assert!(
            report(GestureState::Cancelled, (15.0, 0.0), (0.0, 0.0)).is_none(),
            "a cancelled sequence disarms the gate"
        );
    }

    /// A release before the threshold delivers no drag at all — the
    /// recognizer's `Ended` on an unopened sequence means nothing fired.
    #[test]
    fn pan_event_ignores_a_release_below_the_threshold() {
        let started = Cell::new(false);
        assert!(
            pan_event(
                &started,
                10.0,
                GestureState::Ended,
                (0.0, 0.0),
                (5.0, 0.0),
                (0.0, 0.0),
                PointerButton::Primary,
            )
            .is_none()
        );
        assert!(!started.get());
    }

    /// Only a `Tap` in the composition makes the host answer an assistive
    /// activation.
    #[test]
    fn activates_on_press_searches_the_composition_for_a_tap() {
        let tap: Gesture = TapGesture::new().into();
        let hold: Gesture = LongPressGesture::new(500).into();
        let drag: Gesture = DragGesture::new(0.0).into();
        assert!(activates_on_press(&tap));
        assert!(!activates_on_press(&hold));
        assert!(!activates_on_press(&drag));
        assert!(!activates_on_press(&hold.clone().then(drag.clone())));
        assert!(activates_on_press(&hold.clone().then(tap.clone())));
        assert!(activates_on_press(
            &hold.clone().simultaneously_with(tap.clone())
        ));
        assert!(!activates_on_press(
            &hold.clone().simultaneously_with(drag.clone())
        ));
        assert!(activates_on_press(&drag.clone().exclusively_before(tap)));
        assert!(!activates_on_press(&hold.exclusively_before(drag)));
    }
}
