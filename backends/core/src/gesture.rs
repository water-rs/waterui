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
    RotationEvent, TapEvent,
};
use waterui_core::Environment;
use waterui_core::handler::BoxedAction;

use crate::input::TouchPhase;
use crate::time::Instant;

const TAP_REPEAT_WINDOW: Duration = Duration::from_millis(320);
const TAP_SPATIAL_TOLERANCE: f64 = 24.0;
const LONG_PRESS_SLOP: f64 = 10.0;
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
/// registered targets, picks the topmost group under the pointer, and
/// activates its recognizers ordered by depth, then z-order, then
/// registration index; subsequent moves, ticks, and the final up/cancel are
/// dispatched to that active set. The target list is rebuilt or truncated
/// around structural rebuilds while active recognizers persist across frames
/// as long as their registrations stay live.
#[derive(Debug, Default)]
pub struct GestureEngine {
    targets: Vec<GestureTarget>,
    active_recognizers: Vec<GestureTarget>,
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

    /// Drops targets registered after the `len` watermark, then clears the
    /// active set if any active recognizer lost its live registration.
    pub fn truncate_targets(&mut self, len: usize) {
        self.targets.truncate(len);
        self.ensure_active_recognizers_are_live();
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

    /// Handles a pointer press: cancels any recognizers left active from a
    /// previous sequence, activates the recognizers hit at `point`, and feeds
    /// them the down event. Returns whether any action fired.
    pub fn handle_pointer_down(
        &mut self,
        point: kurbo::Point,
        at: Instant,
        env: &Environment,
    ) -> bool {
        let mut changed = self.replace_active_recognizers(point, at, env);
        changed |=
            self.dispatch_to_active_recognizers(GestureInput::PointerDown { point, at }, env);
        changed
    }

    /// Feeds a pointer move to the active recognizers (drag updates, tap and
    /// long-press slop checks). Returns whether any action fired.
    pub fn handle_pointer_move(
        &mut self,
        point: kurbo::Point,
        at: Instant,
        env: &Environment,
    ) -> bool {
        self.dispatch_to_active_recognizers(GestureInput::PointerMove { point, at }, env)
    }

    /// Feeds the pointer release to the active recognizers and ends the
    /// sequence, deactivating them. Returns whether any action fired.
    pub fn handle_pointer_up(
        &mut self,
        point: kurbo::Point,
        at: Instant,
        env: &Environment,
    ) -> bool {
        let active = core::mem::take(&mut self.active_recognizers);
        Self::dispatch_to_recognizers(&active, GestureInput::PointerUp { point, at }, env)
    }

    /// Cancels the active pointer sequence (window defocus, system gesture
    /// takeover): in-flight drags emit a `Cancelled` phase, pending taps and
    /// long presses fail. Returns whether any action fired.
    pub fn handle_pointer_cancel(&mut self, at: Instant, env: &Environment) -> bool {
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
            .filter(|(_, target)| target.group_id == group_id && target.bounds.contains(point))
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

    fn top_group_id_at(&self, point: kurbo::Point) -> Option<usize> {
        self.targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.bounds.contains(point))
            .max_by(|(left_index, left), (right_index, right)| {
                Self::target_priority(left, *left_index)
                    .cmp(&Self::target_priority(right, *right_index))
            })
            .map(|(_, target)| target.group_id)
    }

    fn active_recognizers_are_live(&self) -> bool {
        self.active_recognizers
            .iter()
            .all(|recognizer| self.is_recognizer_live(recognizer))
    }

    fn ensure_active_recognizers_are_live(&mut self) {
        if !self.active_recognizers_are_live() {
            self.active_recognizers.clear();
        }
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
    pressed_point: Option<kurbo::Point>,
    streak: u32,
    last_tap_at: Option<Instant>,
    last_tap_point: Option<kurbo::Point>,
}

impl TapDetector {
    fn new(required_count: u32) -> Self {
        Self {
            required_count: required_count.max(1),
            pressed_point: None,
            streak: 0,
            last_tap_at: None,
            last_tap_point: None,
        }
    }
}

impl GestureDetector for TapDetector {
    fn input(&mut self, input: GestureInput) -> GestureDetection {
        match input {
            GestureInput::PointerDown { point, .. } => {
                self.pressed_point = Some(point);
                GestureDetection::default()
            }
            GestureInput::PointerMove { point, .. } => {
                let Some(pressed_point) = self.pressed_point else {
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
                if self.pressed_point.take().is_none() {
                    return GestureDetection::default();
                }

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
                }))
            }
            GestureInput::PointerCancel { .. } => {
                self.pressed_point = None;
                GestureDetection::failed()
            }
            _ => GestureDetection::default(),
        }
    }

    fn reset(&mut self) {
        self.pressed_point = None;
        self.streak = 0;
    }
}

struct LongPressDetector {
    duration: Duration,
    started_at: Option<Instant>,
    started_point: Option<kurbo::Point>,
    fired: bool,
}

impl LongPressDetector {
    const fn new(duration: Duration) -> Self {
        Self {
            duration,
            started_at: None,
            started_point: None,
            fired: false,
        }
    }
}

impl GestureDetector for LongPressDetector {
    fn input(&mut self, input: GestureInput) -> GestureDetection {
        match input {
            GestureInput::PointerDown { point, at } => {
                self.started_at = Some(at);
                self.started_point = Some(point);
                self.fired = false;
                GestureDetection::default()
            }
            GestureInput::PointerMove { point, .. } => {
                if self.fired {
                    return GestureDetection::default();
                }
                let Some(start_point) = self.started_point else {
                    return GestureDetection::default();
                };
                if (point.x - start_point.x).hypot(point.y - start_point.y) <= LONG_PRESS_SLOP {
                    return GestureDetection::default();
                }
                self.reset();
                GestureDetection::failed()
            }
            GestureInput::PointerUp { point, at } => {
                let Some(started_at) = self.started_at else {
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
                });
                self.reset();
                GestureDetection::recognized(payload)
            }
            GestureInput::PointerCancel { .. } => {
                if self.started_at.is_some() && !self.fired {
                    self.reset();
                    return GestureDetection::failed();
                }
                self.reset();
                GestureDetection::default()
            }
            GestureInput::Tick { at } => {
                let (Some(started_at), Some(started_point)) = (self.started_at, self.started_point)
                else {
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
                }))
            }
            _ => GestureDetection::default(),
        }
    }

    fn next_deadline(&self) -> Option<Instant> {
        if self.fired {
            return None;
        }
        self.started_at.map(|started_at| started_at + self.duration)
    }

    fn reset(&mut self) {
        self.started_at = None;
        self.started_point = None;
        self.fired = false;
    }
}

struct DragDetector {
    min_distance: f32,
    start_point: Option<kurbo::Point>,
    last_point: Option<kurbo::Point>,
    last_at: Option<Instant>,
    started: bool,
}

impl DragDetector {
    const fn new(min_distance: f32) -> Self {
        Self {
            min_distance,
            start_point: None,
            last_point: None,
            last_at: None,
            started: false,
        }
    }
}

impl GestureDetector for DragDetector {
    fn input(&mut self, input: GestureInput) -> GestureDetection {
        match input {
            GestureInput::PointerDown { point, at } => {
                self.start_point = Some(point);
                self.last_point = Some(point);
                self.last_at = Some(at);
                self.started = false;
                GestureDetection::default()
            }
            GestureInput::PointerMove { point, at } => {
                let (Some(start_point), Some(previous_point), Some(previous_at)) =
                    (self.start_point, self.last_point, self.last_at)
                else {
                    return GestureDetection::default();
                };

                let dx = logical_coordinate(point.x - start_point.x);
                let dy = logical_coordinate(point.y - start_point.y);
                let distance = dx.hypot(dy);
                let dt = at
                    .duration_since(previous_at)
                    .as_secs_f32()
                    .max(f32::EPSILON);
                let velocity = GesturePoint::new(
                    logical_coordinate(point.x - previous_point.x) / dt,
                    logical_coordinate(point.y - previous_point.y) / dt,
                );

                self.last_point = Some(point);
                self.last_at = Some(at);
                if !self.started {
                    if distance < self.min_distance {
                        return GestureDetection::default();
                    }
                    self.started = true;
                    return GestureDetection::recognized(GesturePayload::Drag(DragEvent {
                        phase: GesturePhase::Started,
                        location: gesture_point(point),
                        translation: GesturePoint::new(dx, dy),
                        velocity,
                    }));
                }

                GestureDetection::recognized(GesturePayload::Drag(DragEvent {
                    phase: GesturePhase::Updated,
                    location: gesture_point(point),
                    translation: GesturePoint::new(dx, dy),
                    velocity,
                }))
            }
            GestureInput::PointerUp { point, at } => {
                let Some(start_point) = self.start_point else {
                    return GestureDetection::default();
                };
                let previous_point = self.last_point.unwrap_or(start_point);
                let previous_at = self.last_at.unwrap_or(at);
                let dx = logical_coordinate(point.x - start_point.x);
                let dy = logical_coordinate(point.y - start_point.y);
                let dt = at
                    .duration_since(previous_at)
                    .as_secs_f32()
                    .max(f32::EPSILON);
                let velocity = GesturePoint::new(
                    logical_coordinate(point.x - previous_point.x) / dt,
                    logical_coordinate(point.y - previous_point.y) / dt,
                );
                if !self.started {
                    self.reset();
                    return GestureDetection::failed();
                }
                self.reset();
                GestureDetection::recognized(GesturePayload::Drag(DragEvent {
                    phase: GesturePhase::Ended,
                    location: gesture_point(point),
                    translation: GesturePoint::new(dx, dy),
                    velocity,
                }))
            }
            GestureInput::PointerCancel { .. } => {
                let Some(start_point) = self.start_point else {
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
                GestureDetection::recognized(GesturePayload::Drag(DragEvent {
                    phase: GesturePhase::Cancelled,
                    location: gesture_point(point),
                    translation: GesturePoint::new(dx, dy),
                    velocity: GesturePoint::new(0.0, 0.0),
                }))
            }
            _ => GestureDetection::default(),
        }
    }

    fn reset(&mut self) {
        self.start_point = None;
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
        if self.suppress_until.is_some_and(|deadline| now < deadline) {
            return GestureDetection::default();
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
        Gesture::Tap(tap) => Box::new(TapDetector::new(tap.count)),
        Gesture::LongPress(long_press) => Box::new(LongPressDetector::new(Duration::from_millis(
            u64::from(long_press.duration),
        ))),
        Gesture::Drag(drag) => Box::new(DragDetector::new(drag.min_distance)),
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
        let mut detector = LongPressDetector::new(Duration::from_millis(300));
        let start = Instant::now();
        let point = kurbo::Point::new(12.0, 24.0);

        let down = detector.input(GestureInput::PointerDown { point, at: start });
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
        let mut detector = DragDetector::new(10.0);
        let start = Instant::now();
        let origin = kurbo::Point::new(0.0, 0.0);

        detector.input(GestureInput::PointerDown {
            point: origin,
            at: start,
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
            Box::new(TapDetector::new(1)),
            Box::new(LongPressDetector::new(Duration::from_millis(100))),
        );
        let start = Instant::now();
        let point = kurbo::Point::new(5.0, 7.0);

        detector.input(GestureInput::PointerDown { point, at: start });
        let first = detector.input(GestureInput::PointerUp {
            point,
            at: start + Duration::from_millis(10),
        });
        assert!(first.recognized.is_none());

        detector.input(GestureInput::PointerDown {
            point,
            at: start + Duration::from_millis(20),
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
    fn simultaneous_detector_fires_when_any_child_recognizes() {
        let mut detector = SimultaneousDetector::new(
            Box::new(TapDetector::new(1)),
            Box::new(LongPressDetector::new(Duration::from_millis(100))),
        );
        let start = Instant::now();
        let point = kurbo::Point::new(2.0, 3.0);

        detector.input(GestureInput::PointerDown { point, at: start });
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
        let mut detector = TapDetector::new(1);
        let start = Instant::now();
        let origin = kurbo::Point::new(0.0, 0.0);
        let moved_point = kurbo::Point::new(TAP_SPATIAL_TOLERANCE + 1.0, 0.0);

        detector.input(GestureInput::PointerDown {
            point: origin,
            at: start,
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
        assert!(!engine.handle_pointer_down(origin, start, &env));
        assert!(engine.handle_pointer_move(moved, start + Duration::from_millis(16), &env));
        assert!(engine.handle_pointer_up(moved, start + Duration::from_millis(32), &env));
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
}
