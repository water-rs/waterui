//! Platform-agnostic gesture recognition fed by pointer phases.
//!
//! [`GestureEngine`] holds the gesture targets registered during view
//! dispatch (a hit-test rectangle plus a recognizer state machine per
//! `Gesture` modifier) and routes raw pointer-down/move/up/cancel input,
//! pinch/rotation phases, and frame ticks to the recognizers hit by the
//! pointer. Recognized gestures invoke the bound action with the event
//! (`TapEvent`, `LongPressEvent`, `DragEvent`, `MagnificationEvent`,
//! `RotationEvent`; composed gestures deliver the completing child's event)
//! inserted into the environment, with locations localized to the target's
//! bounds. All coordinates are logical pixels; timestamps come from
//! [`crate::time::Instant`].

use core::time::Duration;
use std::cell::RefCell;
use std::rc::Rc;

use num_traits::ToPrimitive;
use waterui::gesture::{
    DragEvent, Gesture, GesturePhase, GesturePoint, LongPressEvent, MagnificationEvent,
    PointerButton, PointerButtons, RotationEvent, TapEvent,
};
use waterui_core::Environment;
use waterui_core::handler::BoxedAction;

use crate::input::TouchPhase;
use crate::time::Instant;

const TAP_REPEAT_WINDOW: Duration = Duration::from_millis(320);
const TAP_SPATIAL_TOLERANCE: f64 = 24.0;
/// How far, in logical points, a long press may drift before it fails.
///
/// Backends that recognise a platform hold gesture outside the gesture engine
/// (a touch hold that opens a context menu) use the same slop.
pub const LONG_PRESS_SLOP: f64 = 10.0;
const EXCLUSIVE_RECOGNITION_WINDOW: Duration = Duration::from_millis(50);

type GestureRecognizerHandle = Rc<RefCell<GestureBinding>>;

/// One registered gesture region: a hit-test rectangle bound to a recognizer
/// state machine shared via `Rc`, so clones of a target feed the same
/// recognizer.
#[derive(Clone)]
pub struct GestureTarget {
    /// Hit-test rectangle in window coordinates (logical pixels); also the
    /// origin against which recognized event locations are localized.
    pub bounds: kurbo::Rect,
    /// Nesting depth in the view tree; deeper targets win hit-test priority.
    pub depth: usize,
    /// Z-order among siblings at the same depth; higher wins hit-test
    /// priority.
    pub order: usize,
    /// Identity of the hit-test group (e.g. one overlay layer); only targets
    /// in the topmost group under the pointer receive input.
    pub group_id: usize,
    recognizer: GestureRecognizerHandle,
}

impl core::fmt::Debug for GestureTarget {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GestureTarget")
            .field("bounds", &self.bounds)
            .field("depth", &self.depth)
            .field("order", &self.order)
            .field("group_id", &self.group_id)
            .finish_non_exhaustive()
    }
}

impl GestureTarget {
    /// Returns a copy of this target re-registered at new bounds, depth, and
    /// group while sharing the same recognizer state machine.
    ///
    /// Used when a retained subtree is replayed at a different placement, so
    /// in-flight recognition (e.g. a pending long press) survives the move.
    #[must_use]
    pub fn with_bounds_depth_and_group(
        &self,
        bounds: kurbo::Rect,
        depth: usize,
        group_id: usize,
    ) -> Self {
        Self {
            bounds,
            depth,
            order: self.order,
            group_id,
            recognizer: Rc::clone(&self.recognizer),
        }
    }
}

#[derive(Clone, Copy)]
enum GestureInput {
    PointerDown {
        point: kurbo::Point,
        at: Instant,
        button: PointerButton,
    },
    PointerMove {
        point: kurbo::Point,
        at: Instant,
    },
    PointerUp {
        point: kurbo::Point,
        at: Instant,
    },
    PointerCancel {
        at: Instant,
    },
    Tick {
        at: Instant,
    },
    Magnification {
        center: kurbo::Point,
        delta: f32,
        phase: TouchPhase,
        at: Instant,
    },
    Rotation {
        center: kurbo::Point,
        delta: f32,
        phase: TouchPhase,
        at: Instant,
    },
}

#[derive(Clone)]
enum GesturePayload {
    Tap(TapEvent),
    LongPress(LongPressEvent),
    Drag(DragEvent),
    Magnification(MagnificationEvent),
    Rotation(RotationEvent),
}

#[derive(Default)]
struct GestureDetection {
    recognized: Option<GesturePayload>,
    failed: bool,
}

impl GestureDetection {
    const fn recognized(payload: GesturePayload) -> Self {
        Self {
            recognized: Some(payload),
            failed: false,
        }
    }

    const fn failed() -> Self {
        Self {
            recognized: None,
            failed: true,
        }
    }
}

trait GestureDetector {
    fn input(&mut self, input: GestureInput) -> GestureDetection;
    fn next_deadline(&self) -> Option<Instant> {
        None
    }
    fn reset(&mut self) {}
}

struct GestureBinding {
    gesture: Gesture,
    action: Rc<RefCell<BoxedAction<()>>>,
    detector: Box<dyn GestureDetector>,
}

impl GestureBinding {
    fn new(gesture: Gesture, action: BoxedAction<()>) -> Self {
        Self {
            detector: build_gesture_detector(&gesture),
            gesture,
            action: Rc::new(RefCell::new(action)),
        }
    }

    fn input(&mut self, input: GestureInput, env: &Environment, bounds: kurbo::Rect) -> bool {
        let detection = self.detector.input(input);
        let Some(payload) = detection.recognized else {
            return false;
        };
        let mut local_env = env.clone();
        local_env.insert(self.gesture.clone());
        match localize_gesture_payload(payload, bounds) {
            GesturePayload::Tap(event) => local_env.insert(event),
            GesturePayload::LongPress(event) => local_env.insert(event),
            GesturePayload::Drag(event) => local_env.insert(event),
            GesturePayload::Magnification(event) => local_env.insert(event),
            GesturePayload::Rotation(event) => local_env.insert(event),
        }
        (self.action.borrow_mut())(&local_env);
        true
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.detector.next_deadline()
    }
}

fn local_gesture_point(point: GesturePoint, bounds: kurbo::Rect) -> GesturePoint {
    GesturePoint::new(
        point.x - logical_coordinate(bounds.x0),
        point.y - logical_coordinate(bounds.y0),
    )
}

fn logical_coordinate(value: f64) -> f32 {
    value
        .to_f32()
        .expect("gesture coordinate must be representable as f32")
}

fn gesture_point(point: kurbo::Point) -> GesturePoint {
    GesturePoint::new(logical_coordinate(point.x), logical_coordinate(point.y))
}

fn localize_gesture_payload(payload: GesturePayload, bounds: kurbo::Rect) -> GesturePayload {
    match payload {
        GesturePayload::Tap(mut event) => {
            event.location = local_gesture_point(event.location, bounds);
            GesturePayload::Tap(event)
        }
        GesturePayload::LongPress(mut event) => {
            event.location = local_gesture_point(event.location, bounds);
            GesturePayload::LongPress(event)
        }
        GesturePayload::Drag(mut event) => {
            event.location = local_gesture_point(event.location, bounds);
            GesturePayload::Drag(event)
        }
        GesturePayload::Magnification(mut event) => {
            event.center = local_gesture_point(event.center, bounds);
            GesturePayload::Magnification(event)
        }
        GesturePayload::Rotation(mut event) => {
            event.center = local_gesture_point(event.center, bounds);
            GesturePayload::Rotation(event)
        }
    }
}

/// Routes pointer input to the gesture targets registered during dispatch.
///
/// On pointer-down (or pinch/rotation start) the engine hit-tests the
/// registered targets, picks the topmost group under the pointer with at
/// least one recognizer accepting the press, and activates that group's
/// accepting recognizers ordered by depth, then z-order, then registration
/// index; subsequent moves, ticks, and the final up/cancel are dispatched to
/// that active set. The target list is rebuilt or truncated
/// around structural rebuilds while active recognizers persist across frames
/// as long as their registrations stay live.
#[derive(Debug, Default)]
pub struct GestureEngine {
    targets: Vec<GestureTarget>,
    active_recognizers: Vec<GestureTarget>,
    /// The button whose press opened the active pointer sequence; `None`
    /// while no pointer sequence is in flight. A press from a different
    /// button mid-sequence does not join it, and pinch/rotation activations
    /// — which carry no button — run with `None` so they are not filtered.
    active_button: Option<PointerButton>,
}

impl GestureEngine {
    /// Removes all registered targets; called at the begin of a structural
    /// rebuild before targets are re-registered.
    pub fn clear_targets(&mut self) {
        self.targets.clear();
    }

    /// Returns the number of currently registered targets; used as a
    /// truncation watermark when patching a subtree in isolation.
    #[must_use]
    pub const fn target_count(&self) -> usize {
        self.targets.len()
    }

    /// Returns whether a pointer sequence is currently driving at least one
    /// recognizer (between pointer-down and the final up/cancel).
    #[must_use]
    pub const fn has_active_recognizer(&self) -> bool {
        !self.active_recognizers.is_empty()
    }

    /// Drops targets registered after the `len` watermark.
    ///
    /// This runs mid-emit, while the walk is still re-registering targets
    /// after `clear_targets`: the list is transient, and an armed recognizer
    /// whose node has not re-registered yet is absent even though its target
    /// survives the emit. Reconciling the active set here would cancel those
    /// presses early; the settled check runs in [`Self::sync_after_layout`]
    /// once registration completes.
    pub fn truncate_targets(&mut self, len: usize) {
        self.targets.truncate(len);
    }

    /// Swaps the engine's target list with an externally captured one, used
    /// to splice subtree-captured targets back into the engine when replaying
    /// a retained subtree.
    pub const fn swap_targets(&mut self, external: &mut Vec<GestureTarget>) {
        core::mem::swap(&mut self.targets, external);
    }

    /// Registers a fresh gesture target: builds the recognizer state machine
    /// for `gesture` and binds it to `action` at the given hit-test bounds
    /// (window coordinates, logical pixels) and priority coordinates.
    ///
    /// Returns the registered target so a caller that owns retained state can
    /// keep it and re-register the same recognizer on later frames via
    /// [`Self::register_existing_target`], preserving in-flight recognition.
    pub fn register_target(
        &mut self,
        bounds: kurbo::Rect,
        gesture: Gesture,
        action: BoxedAction<()>,
        depth: usize,
        order: usize,
        group_id: usize,
    ) -> GestureTarget {
        self.register_target_recognizer(
            bounds,
            depth,
            order,
            group_id,
            Rc::new(RefCell::new(GestureBinding::new(gesture, action))),
        )
    }

    fn register_target_recognizer(
        &mut self,
        bounds: kurbo::Rect,
        depth: usize,
        order: usize,
        group_id: usize,
        recognizer: GestureRecognizerHandle,
    ) -> GestureTarget {
        let target = GestureTarget {
            bounds,
            depth,
            order,
            group_id,
            recognizer,
        };
        self.targets.push(target.clone());
        target
    }

    /// Re-registers a previously captured target, preserving its recognizer
    /// state machine (used when replaying retained subtrees).
    pub fn register_existing_target(&mut self, target: GestureTarget) {
        self.targets.push(target);
    }

    /// Handles a pointer press by `button`: cancels any recognizers left
    /// active from a previous sequence, activates the recognizers hit at
    /// `point` whose [`PointerButtons`] accept the button, and feeds them the
    /// down event. A pointer sequence belongs to one button — a press from a
    /// different button while a sequence is in flight does not join it.
    /// Returns whether any action fired.
    pub fn handle_pointer_down(
        &mut self,
        point: kurbo::Point,
        at: Instant,
        button: PointerButton,
        env: &Environment,
    ) -> bool {
        if self.active_button.is_some_and(|active| active != button) {
            return false;
        }
        self.active_button = Some(button);
        let mut changed = self.replace_active_recognizers(point, at, env);
        changed |= self
            .dispatch_to_active_recognizers(GestureInput::PointerDown { point, at, button }, env);
        changed
    }

    /// Feeds a pointer move from `button` to the active recognizers (drag
    /// updates, tap and long-press slop checks). Moves from a button other
    /// than the one that opened the sequence are ignored. Returns whether
    /// any action fired.
    pub fn handle_pointer_move(
        &mut self,
        point: kurbo::Point,
        at: Instant,
        button: PointerButton,
        env: &Environment,
    ) -> bool {
        if self.active_button.is_some_and(|active| active != button) {
            return false;
        }
        self.dispatch_to_active_recognizers(GestureInput::PointerMove { point, at }, env)
    }

    /// Feeds the release of `button` to the active recognizers and ends the
    /// sequence, deactivating them. A release of a button that did not open
    /// the sequence is ignored. Returns whether any action fired.
    pub fn handle_pointer_up(
        &mut self,
        point: kurbo::Point,
        at: Instant,
        button: PointerButton,
        env: &Environment,
    ) -> bool {
        if self.active_button != Some(button) {
            return false;
        }
        self.active_button = None;
        let active = core::mem::take(&mut self.active_recognizers);
        Self::dispatch_to_recognizers(&active, GestureInput::PointerUp { point, at }, env)
    }

    /// Cancels the active pointer sequence (window defocus, system gesture
    /// takeover): in-flight drags emit a `Cancelled` phase, pending taps and
    /// long presses fail. Returns whether any action fired.
    pub fn handle_pointer_cancel(&mut self, at: Instant, env: &Environment) -> bool {
        self.active_button = None;
        self.cancel_active_recognizers(at, env)
    }

    /// Feeds one pinch/magnification phase to the recognizers under `center`.
    ///
    /// A `Started` phase activates the recognizers hit at `center` (cancelling
    /// any previous active set); `Ended`/`Cancelled` deactivates them. `delta`
    /// is the relative scale change for this update (`scale *= 1 + delta`).
    /// Returns whether any action fired.
    pub fn handle_magnification(
        &mut self,
        center: kurbo::Point,
        delta: f32,
        phase: TouchPhase,
        at: Instant,
        env: &Environment,
    ) -> bool {
        let mut changed = false;
        if phase == TouchPhase::Started {
            // A pinch carries no button: any held pointer sequence is
            // superseded, and its button must not filter the recognizers the
            // pinch activates.
            self.active_button = None;
            changed |= self.replace_active_recognizers(center, at, env);
        }
        changed |= self.dispatch_to_active_recognizers(
            GestureInput::Magnification {
                center,
                delta,
                phase,
                at,
            },
            env,
        );
        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.active_recognizers.clear();
        }
        changed
    }

    /// Feeds one rotation phase to the recognizers under `center`, with the
    /// same activation lifecycle as
    /// [`handle_magnification`](Self::handle_magnification); `delta` is the
    /// angle change for this update. Returns whether any action fired.
    pub fn handle_rotation(
        &mut self,
        center: kurbo::Point,
        delta: f32,
        phase: TouchPhase,
        at: Instant,
        env: &Environment,
    ) -> bool {
        let mut changed = false;
        if phase == TouchPhase::Started {
            self.active_button = None;
            changed |= self.replace_active_recognizers(center, at, env);
        }
        changed |= self.dispatch_to_active_recognizers(
            GestureInput::Rotation {
                center,
                delta,
                phase,
                at,
            },
            env,
        );
        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.active_recognizers.clear();
        }
        changed
    }

    /// Advances time-driven recognition on the active recognizers (a long
    /// press fires once its hold deadline passes without movement); the frame
    /// pump calls this when [`next_deadline`](Self::next_deadline) elapses.
    /// Returns whether any action fired.
    pub fn handle_tick(&mut self, at: Instant, env: &Environment) -> bool {
        self.dispatch_to_active_recognizers(GestureInput::Tick { at }, env)
    }

    /// Reconciles the active set after targets were re-registered by a
    /// rebuild: if any active recognizer is no longer live, re-hit-tests at
    /// the current `pointer` position (clearing the set when the pointer left
    /// the window). Called after layout completes, while a pointer sequence
    /// may still be in flight.
    pub fn sync_after_layout(&mut self, pointer: Option<kurbo::Point>) {
        if self.active_recognizers_are_live() {
            return;
        }
        let Some(pointer) = pointer else {
            self.active_recognizers.clear();
            return;
        };
        self.active_recognizers = self.recognizers_at(pointer);
    }

    /// Returns the earliest instant at which an active recognizer needs a
    /// [`handle_tick`](Self::handle_tick) to make progress (e.g. a pending
    /// long-press hold deadline), or `None` when no timer is armed.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        self.active_recognizers
            .iter()
            .filter_map(|recognizer| recognizer.recognizer.borrow().next_deadline())
            .min()
    }

    fn replace_active_recognizers(
        &mut self,
        point: kurbo::Point,
        at: Instant,
        env: &Environment,
    ) -> bool {
        let changed = self.cancel_active_recognizers(at, env);
        self.active_recognizers = self.recognizers_at(point);
        changed
    }

    fn cancel_active_recognizers(&mut self, at: Instant, env: &Environment) -> bool {
        let active = core::mem::take(&mut self.active_recognizers);
        Self::dispatch_to_recognizers(&active, GestureInput::PointerCancel { at }, env)
    }

    fn dispatch_to_active_recognizers(&self, input: GestureInput, env: &Environment) -> bool {
        Self::dispatch_to_recognizers(&self.active_recognizers, input, env)
    }

    fn dispatch_to_recognizers(
        recognizers: &[GestureTarget],
        input: GestureInput,
        env: &Environment,
    ) -> bool {
        let mut changed = false;
        for target in recognizers {
            changed |= target
                .recognizer
                .borrow_mut()
                .input(input, env, target.bounds);
        }
        changed
    }

    const fn target_priority(target: &GestureTarget, index: usize) -> (usize, usize, usize) {
        (target.depth, target.order, index)
    }

    fn recognizers_at(&self, point: kurbo::Point) -> Vec<GestureTarget> {
        let Some(group_id) = self.top_group_id_at(point) else {
            return Vec::new();
        };
        let mut targets: Vec<_> = self
            .targets
            .iter()
            .enumerate()
            .filter(|(_, target)| {
                target.group_id == group_id
                    && target.bounds.contains(point)
                    && self.accepts_active_button(target)
            })
            .collect();
        targets.sort_by(|(left_index, left), (right_index, right)| {
            Self::target_priority(right, *right_index)
                .cmp(&Self::target_priority(left, *left_index))
        });
        let mut recognizers = Vec::with_capacity(targets.len());
        for (_, target) in targets {
            Self::push_unique_recognizer(&mut recognizers, target);
        }
        recognizers
    }

    /// The topmost hit-test group under `point` that can drive the current
    /// sequence. A button press only considers targets whose recognizer
    /// accepts the button, so a group that cannot use the pressed button is
    /// transparent to it; pinch and rotation carry no button and consider
    /// every target.
    fn top_group_id_at(&self, point: kurbo::Point) -> Option<usize> {
        self.targets
            .iter()
            .enumerate()
            .filter(|(_, target)| {
                target.bounds.contains(point) && self.accepts_active_button(target)
            })
            .max_by(|(left_index, left), (right_index, right)| {
                Self::target_priority(left, *left_index)
                    .cmp(&Self::target_priority(right, *right_index))
            })
            .map(|(_, target)| target.group_id)
    }

    /// Whether `target`'s recognizer may activate in the current sequence:
    /// every recognizer qualifies while no button is in flight
    /// (pinch/rotation), and a button press only reaches recognizers whose
    /// gesture accepts it.
    fn accepts_active_button(&self, target: &GestureTarget) -> bool {
        self.active_button.is_none_or(|button| {
            gesture_buttons(&target.recognizer.borrow().gesture).accepts(button)
        })
    }

    fn active_recognizers_are_live(&self) -> bool {
        self.active_recognizers
            .iter()
            .all(|recognizer| self.is_recognizer_live(recognizer))
    }

    fn is_recognizer_live(&self, recognizer: &GestureTarget) -> bool {
        self.targets
            .iter()
            .any(|target| Rc::ptr_eq(&target.recognizer, &recognizer.recognizer))
    }

    fn push_unique_recognizer(recognizers: &mut Vec<GestureTarget>, candidate: &GestureTarget) {
        if recognizers
            .iter()
            .any(|recognizer| Rc::ptr_eq(&recognizer.recognizer, &candidate.recognizer))
        {
            return;
        }
        recognizers.push(candidate.clone());
    }

    /// Returns `(depth, order, group_id)` for every registered target whose
    /// bounds contain `point`, in registration order.
    ///
    /// Read-only diagnostics query for backend tests asserting hit-test
    /// priority; not used in render paths.
    #[must_use]
    pub fn debug_targets_at(&self, point: kurbo::Point) -> Vec<(usize, usize, usize)> {
        self.targets
            .iter()
            .filter(|target| target.bounds.contains(point))
            .map(|target| (target.depth, target.order, target.group_id))
            .collect()
    }
}

struct TapDetector {
    required_count: u32,
    buttons: PointerButtons,
    /// The press in flight: its point and the button that pressed it. A down
    /// from a button `buttons` does not accept is not recorded at all.
    pressed: Option<(kurbo::Point, PointerButton)>,
    streak: u32,
    last_tap_at: Option<Instant>,
    last_tap_point: Option<kurbo::Point>,
}

impl TapDetector {
    fn new(required_count: u32, buttons: PointerButtons) -> Self {
        Self {
            required_count: required_count.max(1),
            buttons,
            pressed: None,
            streak: 0,
            last_tap_at: None,
            last_tap_point: None,
        }
    }
}

impl GestureDetector for TapDetector {
    fn input(&mut self, input: GestureInput) -> GestureDetection {
        match input {
            GestureInput::PointerDown { point, button, .. } => {
                if !self.buttons.accepts(button) {
                    return GestureDetection::default();
                }
                self.pressed = Some((point, button));
                GestureDetection::default()
            }
            GestureInput::PointerMove { point, .. } => {
                let Some((pressed_point, _)) = self.pressed else {
                    return GestureDetection::default();
                };
                if (point.x - pressed_point.x).hypot(point.y - pressed_point.y)
                    <= TAP_SPATIAL_TOLERANCE
                {
                    return GestureDetection::default();
                }
                self.reset();
                GestureDetection::failed()
            }
            GestureInput::PointerUp { point, at } => {
                let Some((_, button)) = self.pressed.take() else {
                    return GestureDetection::default();
                };

                let within_time = self
                    .last_tap_at
                    .is_some_and(|previous| at.duration_since(previous) <= TAP_REPEAT_WINDOW);
                let within_distance = self.last_tap_point.is_some_and(|previous| {
                    (point.x - previous.x).hypot(point.y - previous.y) <= TAP_SPATIAL_TOLERANCE
                });

                if within_time && within_distance {
                    self.streak = self
                        .streak
                        .checked_add(1)
                        .expect("tap streak counter overflow");
                } else {
                    self.streak = 1;
                }

                self.last_tap_at = Some(at);
                self.last_tap_point = Some(point);
                if self.streak < self.required_count {
                    return GestureDetection::default();
                }

                self.streak = 0;
                GestureDetection::recognized(GesturePayload::Tap(TapEvent {
                    location: gesture_point(point),
                    count: self.required_count,
                    button,
                }))
            }
            GestureInput::PointerCancel { .. } => {
                self.pressed = None;
                GestureDetection::failed()
            }
            _ => GestureDetection::default(),
        }
    }

    fn reset(&mut self) {
        self.pressed = None;
        self.streak = 0;
    }
}

struct LongPressDetector {
    duration: Duration,
    buttons: PointerButtons,
    /// The press in flight: when it started, where, and with which button.
    press: Option<(Instant, kurbo::Point, PointerButton)>,
    fired: bool,
}

impl LongPressDetector {
    const fn new(duration: Duration, buttons: PointerButtons) -> Self {
        Self {
            duration,
            buttons,
            press: None,
            fired: false,
        }
    }
}

impl GestureDetector for LongPressDetector {
    fn input(&mut self, input: GestureInput) -> GestureDetection {
        match input {
            GestureInput::PointerDown { point, at, button } => {
                if !self.buttons.accepts(button) {
                    return GestureDetection::default();
                }
                self.press = Some((at, point, button));
                self.fired = false;
                GestureDetection::default()
            }
            GestureInput::PointerMove { point, .. } => {
                if self.fired {
                    return GestureDetection::default();
                }
                let Some((_, start_point, _)) = self.press else {
                    return GestureDetection::default();
                };
                if (point.x - start_point.x).hypot(point.y - start_point.y) <= LONG_PRESS_SLOP {
                    return GestureDetection::default();
                }
                self.reset();
                GestureDetection::failed()
            }
            GestureInput::PointerUp { point, at } => {
                let Some((started_at, _, button)) = self.press else {
                    return GestureDetection::default();
                };
                if self.fired {
                    self.reset();
                    return GestureDetection::default();
                }
                if at.duration_since(started_at) < self.duration {
                    self.reset();
                    return GestureDetection::failed();
                }
                self.fired = true;
                let duration_ms = self.duration.as_secs_f32() * 1_000.0;
                let payload = GesturePayload::LongPress(LongPressEvent {
                    location: gesture_point(point),
                    duration: duration_ms,
                    button,
                });
                self.reset();
                GestureDetection::recognized(payload)
            }
            GestureInput::PointerCancel { .. } => {
                if self.press.is_some() && !self.fired {
                    self.reset();
                    return GestureDetection::failed();
                }
                self.reset();
                GestureDetection::default()
            }
            GestureInput::Tick { at } => {
                let Some((started_at, started_point, button)) = self.press else {
                    return GestureDetection::default();
                };
                if self.fired || at.duration_since(started_at) < self.duration {
                    return GestureDetection::default();
                }
                self.fired = true;
                let duration_ms = self.duration.as_secs_f32() * 1_000.0;
                GestureDetection::recognized(GesturePayload::LongPress(LongPressEvent {
                    location: gesture_point(started_point),
                    duration: duration_ms,
                    button,
                }))
            }
            _ => GestureDetection::default(),
        }
    }

    fn next_deadline(&self) -> Option<Instant> {
        if self.fired {
            return None;
        }
        self.press
            .map(|(started_at, _, _)| started_at + self.duration)
    }

    fn reset(&mut self) {
        self.press = None;
        self.fired = false;
    }
}

struct DragDetector {
    min_distance: f32,
    buttons: PointerButtons,
    /// The accepted press opening the in-flight drag: its point and button.
    start: Option<(kurbo::Point, PointerButton)>,
    last_point: Option<kurbo::Point>,
    last_at: Option<Instant>,
    started: bool,
}

impl DragDetector {
    const fn new(min_distance: f32, buttons: PointerButtons) -> Self {
        Self {
            min_distance,
            buttons,
            start: None,
            last_point: None,
            last_at: None,
            started: false,
        }
    }

    fn event(
        phase: GesturePhase,
        point: kurbo::Point,
        button: PointerButton,
        translation: GesturePoint,
        velocity: GesturePoint,
    ) -> GesturePayload {
        GesturePayload::Drag(DragEvent {
            phase,
            location: gesture_point(point),
            translation,
            velocity,
            button,
        })
    }

    fn velocity(
        point: kurbo::Point,
        previous_point: kurbo::Point,
        previous_at: Instant,
        at: Instant,
    ) -> GesturePoint {
        let dt = at
            .duration_since(previous_at)
            .as_secs_f32()
            .max(f32::EPSILON);
        GesturePoint::new(
            logical_coordinate(point.x - previous_point.x) / dt,
            logical_coordinate(point.y - previous_point.y) / dt,
        )
    }
}

impl GestureDetector for DragDetector {
    fn input(&mut self, input: GestureInput) -> GestureDetection {
        match input {
            GestureInput::PointerDown { point, at, button } => {
                if !self.buttons.accepts(button) {
                    return GestureDetection::default();
                }
                self.start = Some((point, button));
                self.last_point = Some(point);
                self.last_at = Some(at);
                self.started = false;
                GestureDetection::default()
            }
            GestureInput::PointerMove { point, at } => {
                let (Some((start_point, button)), Some(previous_point), Some(previous_at)) =
                    (self.start, self.last_point, self.last_at)
                else {
                    return GestureDetection::default();
                };

                let dx = logical_coordinate(point.x - start_point.x);
                let dy = logical_coordinate(point.y - start_point.y);
                let distance = dx.hypot(dy);
                let velocity = Self::velocity(point, previous_point, previous_at, at);

                self.last_point = Some(point);
                self.last_at = Some(at);
                if !self.started {
                    if distance < self.min_distance {
                        return GestureDetection::default();
                    }
                    self.started = true;
                    return GestureDetection::recognized(Self::event(
                        GesturePhase::Started,
                        point,
                        button,
                        GesturePoint::new(dx, dy),
                        velocity,
                    ));
                }

                GestureDetection::recognized(Self::event(
                    GesturePhase::Updated,
                    point,
                    button,
                    GesturePoint::new(dx, dy),
                    velocity,
                ))
            }
            GestureInput::PointerUp { point, at } => {
                let Some((start_point, button)) = self.start else {
                    return GestureDetection::default();
                };
                let previous_point = self.last_point.unwrap_or(start_point);
                let previous_at = self.last_at.unwrap_or(at);
                let dx = logical_coordinate(point.x - start_point.x);
                let dy = logical_coordinate(point.y - start_point.y);
                let velocity = Self::velocity(point, previous_point, previous_at, at);
                if !self.started {
                    self.reset();
                    return GestureDetection::failed();
                }
                self.reset();
                GestureDetection::recognized(Self::event(
                    GesturePhase::Ended,
                    point,
                    button,
                    GesturePoint::new(dx, dy),
                    velocity,
                ))
            }
            GestureInput::PointerCancel { .. } => {
                let Some((start_point, button)) = self.start else {
                    return GestureDetection::default();
                };
                if !self.started {
                    self.reset();
                    return GestureDetection::failed();
                }
                let point = self.last_point.unwrap_or(start_point);
                let dx = logical_coordinate(point.x - start_point.x);
                let dy = logical_coordinate(point.y - start_point.y);
                self.reset();
                GestureDetection::recognized(Self::event(
                    GesturePhase::Cancelled,
                    point,
                    button,
                    GesturePoint::new(dx, dy),
                    GesturePoint::new(0.0, 0.0),
                ))
            }
            _ => GestureDetection::default(),
        }
    }

    fn reset(&mut self) {
        self.start = None;
        self.last_point = None;
        self.last_at = None;
        self.started = false;
    }
}

struct MagnificationDetector {
    initial_scale: f32,
    scale: f32,
    last_at: Option<Instant>,
    active: bool,
}

impl MagnificationDetector {
    const fn new(initial_scale: f32) -> Self {
        Self {
            initial_scale,
            scale: initial_scale,
            last_at: None,
            active: false,
        }
    }
}

impl GestureDetector for MagnificationDetector {
    fn input(&mut self, input: GestureInput) -> GestureDetection {
        let GestureInput::Magnification {
            center,
            delta,
            phase,
            at,
        } = input
        else {
            return GestureDetection::default();
        };
        let mapped_phase = map_touch_phase_to_gesture_phase(phase);
        match phase {
            TouchPhase::Started => {
                self.scale = self.initial_scale;
                self.active = true;
                self.last_at = Some(at);
                GestureDetection::recognized(GesturePayload::Magnification(MagnificationEvent {
                    phase: mapped_phase,
                    center: gesture_point(center),
                    scale: self.scale,
                    velocity: 0.0,
                }))
            }
            TouchPhase::Moved => {
                if !self.active {
                    self.active = true;
                    self.scale = self.initial_scale;
                }
                let previous_at = self.last_at.unwrap_or(at);
                let dt = at
                    .duration_since(previous_at)
                    .as_secs_f32()
                    .max(f32::EPSILON);
                self.last_at = Some(at);
                self.scale = (self.scale * (1.0 + delta)).max(0.01);
                GestureDetection::recognized(GesturePayload::Magnification(MagnificationEvent {
                    phase: mapped_phase,
                    center: gesture_point(center),
                    scale: self.scale,
                    velocity: delta / dt,
                }))
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let payload = GestureDetection::recognized(GesturePayload::Magnification(
                    MagnificationEvent {
                        phase: mapped_phase,
                        center: gesture_point(center),
                        scale: self.scale,
                        velocity: 0.0,
                    },
                ));
                self.reset();
                payload
            }
        }
    }

    fn reset(&mut self) {
        self.scale = self.initial_scale;
        self.last_at = None;
        self.active = false;
    }
}

struct RotationDetector {
    active: bool,
    angle: f32,
    last_at: Option<Instant>,
}

impl RotationDetector {
    const fn new() -> Self {
        Self {
            active: false,
            angle: 0.0,
            last_at: None,
        }
    }
}

impl GestureDetector for RotationDetector {
    fn input(&mut self, input: GestureInput) -> GestureDetection {
        let GestureInput::Rotation {
            center,
            delta,
            phase,
            at,
        } = input
        else {
            return GestureDetection::default();
        };
        let mapped_phase = map_touch_phase_to_gesture_phase(phase);
        match phase {
            TouchPhase::Started => {
                self.active = true;
                self.angle = 0.0;
                self.last_at = Some(at);
                GestureDetection::recognized(GesturePayload::Rotation(RotationEvent {
                    phase: mapped_phase,
                    center: gesture_point(center),
                    angle: self.angle,
                    velocity: 0.0,
                }))
            }
            TouchPhase::Moved => {
                if !self.active {
                    return GestureDetection::default();
                }
                let previous_at = self.last_at.unwrap_or(at);
                let dt = at
                    .duration_since(previous_at)
                    .as_secs_f32()
                    .max(f32::EPSILON);
                self.last_at = Some(at);
                self.angle += delta;
                GestureDetection::recognized(GesturePayload::Rotation(RotationEvent {
                    phase: mapped_phase,
                    center: gesture_point(center),
                    angle: self.angle,
                    velocity: delta / dt,
                }))
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                if !self.active {
                    return GestureDetection::default();
                }
                self.angle += delta;
                let detection =
                    GestureDetection::recognized(GesturePayload::Rotation(RotationEvent {
                        phase: mapped_phase,
                        center: gesture_point(center),
                        angle: self.angle,
                        velocity: 0.0,
                    }));
                self.reset();
                detection
            }
        }
    }

    fn reset(&mut self) {
        self.active = false;
        self.angle = 0.0;
        self.last_at = None;
    }
}

struct ThenDetector {
    first: Box<dyn GestureDetector>,
    second: Box<dyn GestureDetector>,
    awaiting_second: bool,
}

impl ThenDetector {
    fn new(first: Box<dyn GestureDetector>, second: Box<dyn GestureDetector>) -> Self {
        Self {
            first,
            second,
            awaiting_second: false,
        }
    }
}

impl GestureDetector for ThenDetector {
    fn input(&mut self, input: GestureInput) -> GestureDetection {
        if !self.awaiting_second {
            let detection = self.first.input(input);
            if detection.recognized.is_some() {
                self.awaiting_second = true;
                self.second.reset();
            }
            return GestureDetection::default();
        }

        let detection = self.second.input(input);
        if let Some(payload) = detection.recognized {
            self.awaiting_second = false;
            self.first.reset();
            return GestureDetection::recognized(payload);
        }
        if detection.failed {
            self.awaiting_second = false;
            self.first.reset();
        }
        GestureDetection::default()
    }

    fn next_deadline(&self) -> Option<Instant> {
        if self.awaiting_second {
            return self.second.next_deadline();
        }
        self.first.next_deadline()
    }

    fn reset(&mut self) {
        self.awaiting_second = false;
        self.first.reset();
        self.second.reset();
    }
}

struct SimultaneousDetector {
    first: Box<dyn GestureDetector>,
    second: Box<dyn GestureDetector>,
}

impl SimultaneousDetector {
    fn new(first: Box<dyn GestureDetector>, second: Box<dyn GestureDetector>) -> Self {
        Self { first, second }
    }
}

impl GestureDetector for SimultaneousDetector {
    fn input(&mut self, input: GestureInput) -> GestureDetection {
        let first = self.first.input(input);
        if let Some(payload) = first.recognized {
            return GestureDetection::recognized(payload);
        }
        let second = self.second.input(input);
        if let Some(payload) = second.recognized {
            return GestureDetection::recognized(payload);
        }
        GestureDetection::default()
    }

    fn next_deadline(&self) -> Option<Instant> {
        match (self.first.next_deadline(), self.second.next_deadline()) {
            (Some(lhs), Some(rhs)) => Some(lhs.min(rhs)),
            (Some(value), None) | (None, Some(value)) => Some(value),
            (None, None) => None,
        }
    }

    fn reset(&mut self) {
        self.first.reset();
        self.second.reset();
    }
}

struct ExclusiveDetector {
    first: Box<dyn GestureDetector>,
    second: Box<dyn GestureDetector>,
    suppress_until: Option<Instant>,
}

impl ExclusiveDetector {
    fn new(first: Box<dyn GestureDetector>, second: Box<dyn GestureDetector>) -> Self {
        Self {
            first,
            second,
            suppress_until: None,
        }
    }
}

impl GestureDetector for ExclusiveDetector {
    fn input(&mut self, input: GestureInput) -> GestureDetection {
        let now = gesture_input_instant(input);
        match self.suppress_until {
            Some(deadline) if now < deadline => return GestureDetection::default(),
            // The window has elapsed: forget it, or `next_deadline` keeps
            // reporting a time in the past and the host wakes every frame.
            Some(_) => self.suppress_until = None,
            None => {}
        }

        let first = self.first.input(input);
        if let Some(payload) = first.recognized {
            self.second.reset();
            self.suppress_until = Some(now + EXCLUSIVE_RECOGNITION_WINDOW);
            return GestureDetection::recognized(payload);
        }

        let second = self.second.input(input);
        if let Some(payload) = second.recognized {
            self.first.reset();
            self.suppress_until = Some(now + EXCLUSIVE_RECOGNITION_WINDOW);
            return GestureDetection::recognized(payload);
        }
        GestureDetection::default()
    }

    fn next_deadline(&self) -> Option<Instant> {
        let composed = match (self.first.next_deadline(), self.second.next_deadline()) {
            (Some(lhs), Some(rhs)) => Some(lhs.min(rhs)),
            (Some(value), None) | (None, Some(value)) => Some(value),
            (None, None) => None,
        };
        match (self.suppress_until, composed) {
            (Some(lhs), Some(rhs)) => Some(lhs.min(rhs)),
            (Some(value), None) | (None, Some(value)) => Some(value),
            (None, None) => None,
        }
    }

    fn reset(&mut self) {
        self.suppress_until = None;
        self.first.reset();
        self.second.reset();
    }
}

const fn map_touch_phase_to_gesture_phase(phase: TouchPhase) -> GesturePhase {
    match phase {
        TouchPhase::Started => GesturePhase::Started,
        TouchPhase::Moved => GesturePhase::Updated,
        TouchPhase::Ended => GesturePhase::Ended,
        TouchPhase::Cancelled => GesturePhase::Cancelled,
    }
}

/// The union of the buttons a gesture's leaf recognizers respond to; a
/// recognizer activates on a press only when this set accepts its button.
/// Pinch and rotation gestures are not driven by buttons, so they report an
/// empty set — they activate through `handle_magnification`/`handle_rotation`,
/// which run the hit-test with no button filter.
fn gesture_buttons(gesture: &Gesture) -> PointerButtons {
    match gesture {
        Gesture::Tap(tap) => tap.buttons,
        Gesture::LongPress(long_press) => long_press.buttons,
        Gesture::Drag(drag) => drag.buttons,
        Gesture::Then(pair) => gesture_buttons(pair.first()) | gesture_buttons(pair.then()),
        Gesture::Simultaneous(pair) => {
            gesture_buttons(pair.first()) | gesture_buttons(pair.second())
        }
        Gesture::Exclusive(pair) => gesture_buttons(pair.first()) | gesture_buttons(pair.second()),
        // Magnification and rotation carry no button set, as does any leaf
        // added later that does not derive from a press.
        _ => PointerButtons::empty(),
    }
}

const fn gesture_input_instant(input: GestureInput) -> Instant {
    match input {
        GestureInput::PointerDown { at, .. }
        | GestureInput::PointerMove { at, .. }
        | GestureInput::PointerUp { at, .. }
        | GestureInput::PointerCancel { at }
        | GestureInput::Tick { at }
        | GestureInput::Magnification { at, .. }
        | GestureInput::Rotation { at, .. } => at,
    }
}

fn build_gesture_detector(gesture: &Gesture) -> Box<dyn GestureDetector> {
    match gesture {
        Gesture::Tap(tap) => Box::new(TapDetector::new(tap.count, tap.buttons)),
        Gesture::LongPress(long_press) => Box::new(LongPressDetector::new(
            Duration::from_millis(u64::from(long_press.duration)),
            long_press.buttons,
        )),
        Gesture::Drag(drag) => Box::new(DragDetector::new(drag.min_distance, drag.buttons)),
        Gesture::Magnification(magnification) => {
            Box::new(MagnificationDetector::new(magnification.initial_scale))
        }
        Gesture::Rotation(_) => Box::new(RotationDetector::new()),
        Gesture::Then(pair) => Box::new(ThenDetector::new(
            build_gesture_detector(pair.first()),
            build_gesture_detector(pair.then()),
        )),
        Gesture::Simultaneous(pair) => Box::new(SimultaneousDetector::new(
            build_gesture_detector(pair.first()),
            build_gesture_detector(pair.second()),
        )),
        Gesture::Exclusive(pair) => Box::new(ExclusiveDetector::new(
            build_gesture_detector(pair.first()),
            build_gesture_detector(pair.second()),
        )),
        _ => panic!("hydrolysis gesture variant is not implemented"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_press_fires_after_tick_deadline() {
        let mut detector =
            LongPressDetector::new(Duration::from_millis(300), PointerButtons::PRIMARY);
        let start = Instant::now();
        let point = kurbo::Point::new(12.0, 24.0);

        let down = detector.input(GestureInput::PointerDown {
            point,
            at: start,
            button: PointerButton::Primary,
        });
        assert!(down.recognized.is_none());

        let before_deadline = detector.input(GestureInput::Tick {
            at: start + Duration::from_millis(200),
        });
        assert!(before_deadline.recognized.is_none());

        let at_deadline = detector.input(GestureInput::Tick {
            at: start + Duration::from_millis(300),
        });
        assert!(matches!(
            at_deadline.recognized,
            Some(GesturePayload::LongPress(_))
        ));
    }

    #[test]
    fn drag_waits_for_min_distance_then_emits_phases() {
        let mut detector = DragDetector::new(10.0, PointerButtons::PRIMARY);
        let start = Instant::now();
        let origin = kurbo::Point::new(0.0, 0.0);

        detector.input(GestureInput::PointerDown {
            point: origin,
            at: start,
            button: PointerButton::Primary,
        });

        let below_threshold = detector.input(GestureInput::PointerMove {
            point: kurbo::Point::new(6.0, 2.0),
            at: start + Duration::from_millis(16),
        });
        assert!(below_threshold.recognized.is_none());

        let started = detector.input(GestureInput::PointerMove {
            point: kurbo::Point::new(12.0, 0.0),
            at: start + Duration::from_millis(32),
        });
        assert!(matches!(
            started.recognized,
            Some(GesturePayload::Drag(DragEvent {
                phase: GesturePhase::Started,
                ..
            }))
        ));

        let updated = detector.input(GestureInput::PointerMove {
            point: kurbo::Point::new(24.0, 6.0),
            at: start + Duration::from_millis(48),
        });
        assert!(matches!(
            updated.recognized,
            Some(GesturePayload::Drag(DragEvent {
                phase: GesturePhase::Updated,
                ..
            }))
        ));

        let ended = detector.input(GestureInput::PointerUp {
            point: kurbo::Point::new(30.0, 8.0),
            at: start + Duration::from_millis(64),
        });
        assert!(matches!(
            ended.recognized,
            Some(GesturePayload::Drag(DragEvent {
                phase: GesturePhase::Ended,
                ..
            }))
        ));
    }

    #[test]
    fn magnification_accumulates_scale() {
        let mut detector = MagnificationDetector::new(1.0);
        let start = Instant::now();
        let center = kurbo::Point::new(10.0, 20.0);

        let started = detector.input(GestureInput::Magnification {
            center,
            delta: 0.0,
            phase: TouchPhase::Started,
            at: start,
        });
        assert!(matches!(
            started.recognized,
            Some(GesturePayload::Magnification(MagnificationEvent { scale, .. }))
                if (scale - 1.0).abs() < f32::EPSILON
        ));

        let updated = detector.input(GestureInput::Magnification {
            center,
            delta: 0.1,
            phase: TouchPhase::Moved,
            at: start + Duration::from_millis(16),
        });
        assert!(matches!(
            updated.recognized,
            Some(GesturePayload::Magnification(MagnificationEvent { scale, .. }))
                if (scale - 1.1).abs() < 0.0001
        ));

        let ended = detector.input(GestureInput::Magnification {
            center,
            delta: 0.0,
            phase: TouchPhase::Ended,
            at: start + Duration::from_millis(32),
        });
        assert!(matches!(
            ended.recognized,
            Some(GesturePayload::Magnification(MagnificationEvent {
                phase: GesturePhase::Ended,
                ..
            }))
        ));
    }

    #[test]
    fn then_detector_requires_second_gesture_after_first() {
        let mut detector = ThenDetector::new(
            Box::new(TapDetector::new(1, PointerButtons::PRIMARY)),
            Box::new(LongPressDetector::new(
                Duration::from_millis(100),
                PointerButtons::PRIMARY,
            )),
        );
        let start = Instant::now();
        let point = kurbo::Point::new(5.0, 7.0);

        detector.input(GestureInput::PointerDown {
            point,
            at: start,
            button: PointerButton::Primary,
        });
        let first = detector.input(GestureInput::PointerUp {
            point,
            at: start + Duration::from_millis(10),
        });
        assert!(first.recognized.is_none());

        detector.input(GestureInput::PointerDown {
            point,
            at: start + Duration::from_millis(20),
            button: PointerButton::Primary,
        });
        let second = detector.input(GestureInput::Tick {
            at: start + Duration::from_millis(120),
        });
        // The completing child's event is forwarded so `Use<LongPressEvent>`
        // handlers on a composed gesture find their payload.
        assert!(matches!(
            second.recognized,
            Some(GesturePayload::LongPress(_))
        ));
    }

    #[test]
    fn exclusive_detector_drops_its_suppression_deadline_once_elapsed() {
        let mut detector = ExclusiveDetector::new(
            Box::new(TapDetector::new(1, PointerButtons::PRIMARY)),
            Box::new(LongPressDetector::new(
                Duration::from_millis(300),
                PointerButtons::PRIMARY,
            )),
        );
        let start = Instant::now();
        let point = kurbo::Point::new(5.0, 7.0);

        detector.input(GestureInput::PointerDown {
            point,
            at: start,
            button: PointerButton::Primary,
        });
        let tap = detector.input(GestureInput::PointerUp {
            point,
            at: start + Duration::from_millis(10),
        });
        assert!(matches!(tap.recognized, Some(GesturePayload::Tap(_))));
        let window_end = start + Duration::from_millis(10) + EXCLUSIVE_RECOGNITION_WINDOW;
        assert_eq!(detector.next_deadline(), Some(window_end));

        // The host ticks at the reported deadline; after that the detector is
        // idle and must stop asking to be woken.
        detector.input(GestureInput::Tick { at: window_end });
        assert_eq!(detector.next_deadline(), None);
    }

    #[test]
    fn simultaneous_detector_fires_when_any_child_recognizes() {
        let mut detector = SimultaneousDetector::new(
            Box::new(TapDetector::new(1, PointerButtons::PRIMARY)),
            Box::new(LongPressDetector::new(
                Duration::from_millis(100),
                PointerButtons::PRIMARY,
            )),
        );
        let start = Instant::now();
        let point = kurbo::Point::new(2.0, 3.0);

        detector.input(GestureInput::PointerDown {
            point,
            at: start,
            button: PointerButton::Primary,
        });
        let recognized = detector.input(GestureInput::PointerUp {
            point,
            at: start + Duration::from_millis(10),
        });
        // The recognizing child's own event rides along with the recognition.
        assert!(matches!(
            recognized.recognized,
            Some(GesturePayload::Tap(_))
        ));
    }

    #[test]
    fn rotation_detector_delivers_accumulated_angle_events() {
        let mut detector = RotationDetector::new();
        let start = Instant::now();
        let center = kurbo::Point::new(4.0, 6.0);

        let started = detector.input(GestureInput::Rotation {
            center,
            delta: 0.0,
            phase: TouchPhase::Started,
            at: start,
        });
        assert!(matches!(
            started.recognized,
            Some(GesturePayload::Rotation(event)) if event.angle == 0.0
        ));

        let moved = detector.input(GestureInput::Rotation {
            center,
            delta: 0.5,
            phase: TouchPhase::Moved,
            at: start + Duration::from_millis(16),
        });
        match moved.recognized {
            Some(GesturePayload::Rotation(event)) => {
                assert!((event.angle - 0.5).abs() < 1e-6);
                assert!(event.velocity > 0.0);
            }
            other => panic!("expected a rotation payload, got {:?}", other.is_some()),
        }

        let ended = detector.input(GestureInput::Rotation {
            center,
            delta: 0.25,
            phase: TouchPhase::Ended,
            at: start + Duration::from_millis(32),
        });
        assert!(matches!(
            ended.recognized,
            Some(GesturePayload::Rotation(event)) if (event.angle - 0.75).abs() < 1e-6
        ));
    }

    #[test]
    fn tap_fails_after_pointer_moves_beyond_spatial_tolerance() {
        let mut detector = TapDetector::new(1, PointerButtons::PRIMARY);
        let start = Instant::now();
        let origin = kurbo::Point::new(0.0, 0.0);
        let moved_point = kurbo::Point::new(TAP_SPATIAL_TOLERANCE + 1.0, 0.0);

        detector.input(GestureInput::PointerDown {
            point: origin,
            at: start,
            button: PointerButton::Primary,
        });
        let moved = detector.input(GestureInput::PointerMove {
            point: moved_point,
            at: start + Duration::from_millis(16),
        });
        assert!(moved.recognized.is_none());
        assert!(moved.failed);

        let ended = detector.input(GestureInput::PointerUp {
            point: moved_point,
            at: start + Duration::from_millis(32),
        });
        assert!(ended.recognized.is_none());
    }

    #[test]
    fn gesture_engine_dispatches_pointer_input_to_same_group_recognizers() {
        use std::{cell::Cell, rc::Rc};
        use waterui::gesture::{DragGesture, TapGesture};
        use waterui_core::handler::boxed_action;

        let mut engine = GestureEngine::default();
        let env = Environment::new();
        let bounds = kurbo::Rect::new(0.0, 0.0, 128.0, 128.0);
        let tap_hits = Rc::new(Cell::new(0u32));
        let drag_hits = Rc::new(Cell::new(0u32));

        {
            let tap_hits = Rc::clone(&tap_hits);
            engine.register_target(
                bounds,
                Gesture::Tap(TapGesture::new()),
                boxed_action(move |env: Environment| {
                    env.get::<TapEvent>()
                        .expect("tap action missing TapEvent in environment");
                    tap_hits.set(tap_hits.get() + 1);
                }),
                0,
                3,
                7,
            );
        }
        {
            let drag_hits = Rc::clone(&drag_hits);
            engine.register_target(
                bounds,
                Gesture::Drag(DragGesture::new(8.0)),
                boxed_action(move |env: Environment| {
                    env.get::<DragEvent>()
                        .expect("drag action missing DragEvent in environment");
                    drag_hits.set(drag_hits.get() + 1);
                }),
                0,
                2,
                7,
            );
        }

        let start = Instant::now();
        let origin = kurbo::Point::new(16.0, 16.0);
        let moved = kurbo::Point::new(48.0, 16.0);
        assert!(!engine.handle_pointer_down(origin, start, PointerButton::Primary, &env));
        assert!(engine.handle_pointer_move(
            moved,
            start + Duration::from_millis(16),
            PointerButton::Primary,
            &env
        ));
        assert!(engine.handle_pointer_up(
            moved,
            start + Duration::from_millis(32),
            PointerButton::Primary,
            &env
        ));
        assert_eq!(tap_hits.get(), 0);
        assert_eq!(drag_hits.get(), 2);
    }

    #[test]
    fn gesture_engine_dispatches_magnification_to_same_group_recognizers() {
        use std::{cell::Cell, rc::Rc};
        use waterui::gesture::{DragGesture, MagnificationGesture};
        use waterui_core::handler::boxed_action;

        let mut engine = GestureEngine::default();
        let env = Environment::new();
        let bounds = kurbo::Rect::new(0.0, 0.0, 128.0, 128.0);
        let drag_hits = Rc::new(Cell::new(0u32));
        let magnify_hits = Rc::new(Cell::new(0u32));

        {
            let drag_hits = Rc::clone(&drag_hits);
            engine.register_target(
                bounds,
                Gesture::Drag(DragGesture::new(0.0)),
                boxed_action(move |env: Environment| {
                    env.get::<DragEvent>()
                        .expect("drag action missing DragEvent in environment");
                    drag_hits.set(drag_hits.get() + 1);
                }),
                0,
                3,
                11,
            );
        }
        {
            let magnify_hits = Rc::clone(&magnify_hits);
            engine.register_target(
                bounds,
                Gesture::Magnification(MagnificationGesture::new(1.0)),
                boxed_action(move |env: Environment| {
                    env.get::<MagnificationEvent>()
                        .expect("magnification action missing MagnificationEvent in environment");
                    magnify_hits.set(magnify_hits.get() + 1);
                }),
                0,
                2,
                11,
            );
        }

        let start = Instant::now();
        let center = kurbo::Point::new(32.0, 32.0);
        assert!(engine.handle_magnification(center, 0.0, TouchPhase::Started, start, &env));
        assert!(engine.handle_magnification(
            center,
            0.1,
            TouchPhase::Moved,
            start + Duration::from_millis(16),
            &env,
        ));
        assert!(engine.handle_magnification(
            center,
            0.0,
            TouchPhase::Ended,
            start + Duration::from_millis(32),
            &env,
        ));
        assert_eq!(drag_hits.get(), 0);
        assert_eq!(magnify_hits.get(), 3);
    }

    #[test]
    fn middle_button_fires_middle_tap_but_not_default_tap() {
        use std::{cell::Cell, rc::Rc};
        use waterui::gesture::TapGesture;
        use waterui_core::handler::boxed_action;

        let mut engine = GestureEngine::default();
        let env = Environment::new();
        let bounds = kurbo::Rect::new(0.0, 0.0, 128.0, 128.0);
        let default_hits = Rc::new(Cell::new(0u32));
        let middle_hits = Rc::new(Cell::new(0u32));
        let middle_button = Rc::new(Cell::new(Option::<PointerButton>::None));

        {
            let default_hits = Rc::clone(&default_hits);
            engine.register_target(
                bounds,
                Gesture::Tap(TapGesture::new()),
                boxed_action(move |env: Environment| {
                    env.get::<TapEvent>()
                        .expect("tap action missing TapEvent in environment");
                    default_hits.set(default_hits.get() + 1);
                }),
                0,
                2,
                7,
            );
        }
        {
            let middle_hits = Rc::clone(&middle_hits);
            let middle_button = Rc::clone(&middle_button);
            engine.register_target(
                bounds,
                Gesture::Tap(TapGesture::new().buttons(PointerButtons::MIDDLE)),
                boxed_action(move |env: Environment| {
                    let event = env
                        .get::<TapEvent>()
                        .expect("tap action missing TapEvent in environment");
                    middle_button.set(Some(event.button));
                    middle_hits.set(middle_hits.get() + 1);
                }),
                0,
                1,
                7,
            );
        }

        let start = Instant::now();
        let point = kurbo::Point::new(16.0, 16.0);
        engine.handle_pointer_down(point, start, PointerButton::Middle, &env);
        assert!(engine.handle_pointer_up(
            point,
            start + Duration::from_millis(16),
            PointerButton::Middle,
            &env
        ));
        assert_eq!(default_hits.get(), 0);
        assert_eq!(middle_hits.get(), 1);
        assert_eq!(middle_button.get(), Some(PointerButton::Middle));
    }

    #[test]
    fn primary_button_does_not_fire_middle_only_tap() {
        use std::{cell::Cell, rc::Rc};
        use waterui::gesture::TapGesture;
        use waterui_core::handler::boxed_action;

        let mut engine = GestureEngine::default();
        let env = Environment::new();
        let bounds = kurbo::Rect::new(0.0, 0.0, 128.0, 128.0);
        let middle_hits = Rc::new(Cell::new(0u32));

        {
            let middle_hits = Rc::clone(&middle_hits);
            engine.register_target(
                bounds,
                Gesture::Tap(TapGesture::new().buttons(PointerButtons::MIDDLE)),
                boxed_action(move |env: Environment| {
                    env.get::<TapEvent>()
                        .expect("tap action missing TapEvent in environment");
                    middle_hits.set(middle_hits.get() + 1);
                }),
                0,
                1,
                7,
            );
        }

        let start = Instant::now();
        let point = kurbo::Point::new(16.0, 16.0);
        assert!(!engine.handle_pointer_down(point, start, PointerButton::Primary, &env));
        assert!(!engine.handle_pointer_up(
            point,
            start + Duration::from_millis(16),
            PointerButton::Primary,
            &env
        ));
        assert_eq!(middle_hits.get(), 0);
    }

    #[test]
    fn second_button_mid_sequence_does_not_join() {
        use std::{cell::Cell, rc::Rc};
        use waterui::gesture::TapGesture;
        use waterui_core::handler::boxed_action;

        let mut engine = GestureEngine::default();
        let env = Environment::new();
        let bounds = kurbo::Rect::new(0.0, 0.0, 128.0, 128.0);
        let hits = Rc::new(Cell::new(0u32));

        {
            let hits = Rc::clone(&hits);
            engine.register_target(
                bounds,
                Gesture::Tap(TapGesture::new()),
                boxed_action(move |env: Environment| {
                    env.get::<TapEvent>()
                        .expect("tap action missing TapEvent in environment");
                    hits.set(hits.get() + 1);
                }),
                0,
                1,
                7,
            );
        }

        let start = Instant::now();
        let point = kurbo::Point::new(16.0, 16.0);
        engine.handle_pointer_down(point, start, PointerButton::Primary, &env);
        // A press from another button mid-sequence is not part of it.
        assert!(!engine.handle_pointer_down(
            point,
            start + Duration::from_millis(8),
            PointerButton::Middle,
            &env
        ));
        assert!(!engine.handle_pointer_up(
            point,
            start + Duration::from_millis(16),
            PointerButton::Middle,
            &env
        ));
        assert!(engine.handle_pointer_up(
            point,
            start + Duration::from_millis(24),
            PointerButton::Primary,
            &env
        ));
        assert_eq!(hits.get(), 1);
    }

    #[test]
    fn press_passes_through_group_that_rejects_its_button() {
        use std::{cell::Cell, rc::Rc};
        use waterui::gesture::TapGesture;
        use waterui_core::handler::boxed_action;

        let mut engine = GestureEngine::default();
        let env = Environment::new();
        let parent_hits = Rc::new(Cell::new(0u32));
        let child_hits = Rc::new(Cell::new(0u32));

        {
            let parent_hits = Rc::clone(&parent_hits);
            engine.register_target(
                kurbo::Rect::new(0.0, 0.0, 200.0, 40.0),
                Gesture::Tap(TapGesture::new()),
                boxed_action(move |env: Environment| {
                    env.get::<TapEvent>()
                        .expect("tap action missing TapEvent in environment");
                    parent_hits.set(parent_hits.get() + 1);
                }),
                0,
                0,
                1,
            );
        }
        {
            let child_hits = Rc::clone(&child_hits);
            engine.register_target(
                kurbo::Rect::new(0.0, 0.0, 120.0, 40.0),
                Gesture::Tap(TapGesture::new().buttons(PointerButtons::MIDDLE)),
                boxed_action(move |env: Environment| {
                    env.get::<TapEvent>()
                        .expect("tap action missing TapEvent in environment");
                    child_hits.set(child_hits.get() + 1);
                }),
                1,
                0,
                2,
            );
        }

        let start = Instant::now();
        let point = kurbo::Point::new(50.0, 20.0);

        // The nested child is topmost but accepts only middle presses, so a
        // primary press must fall through to the parent's group.
        engine.handle_pointer_down(point, start, PointerButton::Primary, &env);
        assert!(engine.handle_pointer_up(
            point,
            start + Duration::from_millis(16),
            PointerButton::Primary,
            &env
        ));
        assert_eq!(parent_hits.get(), 1);
        assert_eq!(child_hits.get(), 0);

        // A middle press activates the child's group normally.
        let middle = start + Duration::from_millis(400);
        engine.handle_pointer_down(point, middle, PointerButton::Middle, &env);
        assert!(engine.handle_pointer_up(
            point,
            middle + Duration::from_millis(16),
            PointerButton::Middle,
            &env
        ));
        assert_eq!(parent_hits.get(), 1);
        assert_eq!(child_hits.get(), 1);
    }

    #[test]
    fn drag_event_reports_the_pressing_button() {
        use std::{cell::Cell, rc::Rc};
        use waterui::gesture::DragGesture;
        use waterui_core::handler::boxed_action;

        let mut engine = GestureEngine::default();
        let env = Environment::new();
        let bounds = kurbo::Rect::new(0.0, 0.0, 128.0, 128.0);
        let last_button = Rc::new(Cell::new(Option::<PointerButton>::None));

        {
            let last_button = Rc::clone(&last_button);
            engine.register_target(
                bounds,
                Gesture::Drag(
                    DragGesture::new(8.0).buttons(PointerButtons::PRIMARY | PointerButtons::MIDDLE),
                ),
                boxed_action(move |env: Environment| {
                    let event = env
                        .get::<DragEvent>()
                        .expect("drag action missing DragEvent in environment");
                    last_button.set(Some(event.button));
                }),
                0,
                1,
                7,
            );
        }

        let start = Instant::now();
        let origin = kurbo::Point::new(16.0, 16.0);
        let moved = kurbo::Point::new(48.0, 16.0);
        engine.handle_pointer_down(origin, start, PointerButton::Middle, &env);
        engine.handle_pointer_move(
            moved,
            start + Duration::from_millis(16),
            PointerButton::Middle,
            &env,
        );
        assert_eq!(last_button.get(), Some(PointerButton::Middle));
        engine.handle_pointer_up(
            moved,
            start + Duration::from_millis(32),
            PointerButton::Middle,
            &env,
        );
        assert_eq!(last_button.get(), Some(PointerButton::Middle));
    }

    #[test]
    fn tap_survives_reemitted_targets_during_press() {
        // Regression: every scene emit rebuilds the target list under
        // `clear_targets` and re-registers during the walk; a mid-walk
        // `truncate_targets` watermark checked recognizer liveness against
        // the transient list and cleared presses whose retained node had
        // not re-registered yet (`.on_tap` presses died under emit storms).
        use std::{cell::Cell, rc::Rc};
        use waterui::gesture::TapGesture;
        use waterui_core::handler::boxed_action;

        let mut engine = GestureEngine::default();
        let env = Environment::new();
        let bounds = kurbo::Rect::new(0.0, 0.0, 128.0, 128.0);
        let tap_hits = Rc::new(Cell::new(0u32));

        let tap_target = {
            let tap_hits = Rc::clone(&tap_hits);
            engine.register_target(
                bounds,
                Gesture::Tap(TapGesture::new()),
                boxed_action(move |env: Environment| {
                    env.get::<TapEvent>()
                        .expect("tap action missing TapEvent in environment");
                    tap_hits.set(tap_hits.get() + 1);
                }),
                0,
                0,
                7,
            )
        };

        let start = Instant::now();
        let point = kurbo::Point::new(16.0, 16.0);
        assert!(!engine.handle_pointer_down(point, start, PointerButton::Primary, &env));

        // Emits between press and release: the list is cleared and the walk
        // re-registers nodes in order. Other nodes land first; a mid-walk
        // suppression watermark truncates while the armed tap target is
        // still absent; the retained node then re-registers it (same
        // recognizer) later in the walk.
        for _ in 0..3 {
            engine.clear_targets();
            engine.register_target(
                bounds,
                Gesture::Tap(TapGesture::new()),
                boxed_action(|_env: Environment| {}),
                0,
                1,
                7,
            );
            engine.register_target(
                bounds,
                Gesture::Tap(TapGesture::new()),
                boxed_action(|_env: Environment| {}),
                0,
                2,
                7,
            );
            engine.truncate_targets(1);
            engine.register_existing_target(tap_target.clone());
            engine.sync_after_layout(Some(point));
        }

        assert!(engine.handle_pointer_up(
            point,
            start + Duration::from_millis(32),
            PointerButton::Primary,
            &env
        ));
        assert_eq!(tap_hits.get(), 1);
    }
}
