use core::time::Duration;
use std::cell::RefCell;
use std::rc::Rc;

use waterui::gesture::{
    DragEvent, Gesture, GesturePhase, GesturePoint, LongPressEvent, MagnificationEvent, TapEvent,
};
use waterui_core::Environment;
use waterui_core::handler::BoxedAction;

use crate::platform::TouchPhase;
use crate::time::Instant;

const TAP_REPEAT_WINDOW: Duration = Duration::from_millis(320);
const TAP_SPATIAL_TOLERANCE: f64 = 24.0;
const LONG_PRESS_SLOP: f64 = 10.0;
const EXCLUSIVE_RECOGNITION_WINDOW: Duration = Duration::from_millis(50);

type GestureRecognizerHandle = Rc<RefCell<GestureBinding>>;

#[derive(Clone)]
pub(crate) struct GestureTarget {
    pub bounds: vello::kurbo::Rect,
    pub depth: usize,
    pub order: usize,
    pub group_id: usize,
    recognizer: GestureRecognizerHandle,
}

impl GestureTarget {
    pub(crate) fn with_bounds_depth_and_group(
        &self,
        bounds: vello::kurbo::Rect,
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
        point: vello::kurbo::Point,
        at: Instant,
    },
    PointerMove {
        point: vello::kurbo::Point,
        at: Instant,
    },
    PointerUp {
        point: vello::kurbo::Point,
        at: Instant,
    },
    PointerCancel {
        at: Instant,
    },
    Tick {
        at: Instant,
    },
    Magnification {
        center: vello::kurbo::Point,
        delta: f32,
        phase: TouchPhase,
        at: Instant,
    },
    Rotation {
        center: vello::kurbo::Point,
        delta: f32,
        phase: TouchPhase,
        at: Instant,
    },
}

#[derive(Clone)]
enum GesturePayload {
    None,
    Tap(TapEvent),
    LongPress(LongPressEvent),
    Drag(DragEvent),
    Magnification(MagnificationEvent),
}

#[derive(Default)]
struct GestureDetection {
    recognized: Option<GesturePayload>,
    failed: bool,
}

impl GestureDetection {
    fn recognized(payload: GesturePayload) -> Self {
        Self {
            recognized: Some(payload),
            failed: false,
        }
    }

    fn failed() -> Self {
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

    fn input(
        &mut self,
        input: GestureInput,
        env: &Environment,
        bounds: vello::kurbo::Rect,
    ) -> bool {
        let detection = self.detector.input(input);
        let Some(payload) = detection.recognized else {
            return false;
        };
        let mut local_env = env.clone();
        local_env.insert(self.gesture.clone());
        match localize_gesture_payload(payload, bounds) {
            GesturePayload::None => {}
            GesturePayload::Tap(event) => local_env.insert(event),
            GesturePayload::LongPress(event) => local_env.insert(event),
            GesturePayload::Drag(event) => local_env.insert(event),
            GesturePayload::Magnification(event) => local_env.insert(event),
        }
        (self.action.borrow_mut())(&local_env);
        true
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.detector.next_deadline()
    }
}

fn local_gesture_point(point: GesturePoint, bounds: vello::kurbo::Rect) -> GesturePoint {
    GesturePoint::new(point.x - bounds.x0 as f32, point.y - bounds.y0 as f32)
}

fn localize_gesture_payload(payload: GesturePayload, bounds: vello::kurbo::Rect) -> GesturePayload {
    match payload {
        GesturePayload::None => GesturePayload::None,
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
    }
}

#[derive(Default)]
pub(crate) struct GestureEngine {
    targets: Vec<GestureTarget>,
    active_recognizers: Vec<GestureTarget>,
}

impl GestureEngine {
    pub(crate) fn clear_targets(&mut self) {
        self.targets.clear();
    }

    pub(crate) fn target_count(&self) -> usize {
        self.targets.len()
    }

    pub(crate) fn has_active_recognizer(&self) -> bool {
        !self.active_recognizers.is_empty()
    }

    pub(crate) fn truncate_targets(&mut self, len: usize) {
        self.targets.truncate(len);
        self.ensure_active_recognizers_are_live();
    }

    pub(crate) fn swap_targets(&mut self, external: &mut Vec<GestureTarget>) {
        core::mem::swap(&mut self.targets, external);
    }

    pub(crate) fn register_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        gesture: Gesture,
        action: BoxedAction<()>,
        depth: usize,
        order: usize,
        group_id: usize,
    ) {
        self.register_target_recognizer(
            bounds,
            depth,
            order,
            group_id,
            Rc::new(RefCell::new(GestureBinding::new(gesture, action))),
        );
    }

    fn register_target_recognizer(
        &mut self,
        bounds: vello::kurbo::Rect,
        depth: usize,
        order: usize,
        group_id: usize,
        recognizer: GestureRecognizerHandle,
    ) {
        self.targets.push(GestureTarget {
            bounds,
            depth,
            order,
            group_id,
            recognizer,
        });
    }

    pub(crate) fn register_existing_target(&mut self, target: GestureTarget) {
        self.targets.push(target);
    }

    pub(crate) fn handle_pointer_down(
        &mut self,
        point: vello::kurbo::Point,
        at: Instant,
        env: &Environment,
    ) -> bool {
        let mut changed = self.replace_active_recognizers(point, at, env);
        changed |=
            self.dispatch_to_active_recognizers(GestureInput::PointerDown { point, at }, env);
        changed
    }

    pub(crate) fn handle_pointer_move(
        &mut self,
        point: vello::kurbo::Point,
        at: Instant,
        env: &Environment,
    ) -> bool {
        self.dispatch_to_active_recognizers(GestureInput::PointerMove { point, at }, env)
    }

    pub(crate) fn handle_pointer_up(
        &mut self,
        point: vello::kurbo::Point,
        at: Instant,
        env: &Environment,
    ) -> bool {
        let active = core::mem::take(&mut self.active_recognizers);
        Self::dispatch_to_recognizers(&active, GestureInput::PointerUp { point, at }, env)
    }

    pub(crate) fn handle_pointer_cancel(&mut self, at: Instant, env: &Environment) -> bool {
        self.cancel_active_recognizers(at, env)
    }

    pub(crate) fn handle_magnification(
        &mut self,
        center: vello::kurbo::Point,
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

    pub(crate) fn handle_rotation(
        &mut self,
        center: vello::kurbo::Point,
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

    pub(crate) fn handle_tick(&mut self, at: Instant, env: &Environment) -> bool {
        self.dispatch_to_active_recognizers(GestureInput::Tick { at }, env)
    }

    pub(crate) fn sync_after_layout(&mut self, pointer: Option<vello::kurbo::Point>) {
        if self.active_recognizers_are_live() {
            return;
        }
        let Some(pointer) = pointer else {
            self.active_recognizers.clear();
            return;
        };
        self.active_recognizers = self.recognizers_at(pointer);
    }

    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.active_recognizers
            .iter()
            .filter_map(|recognizer| recognizer.recognizer.borrow().next_deadline())
            .min()
    }

    fn replace_active_recognizers(
        &mut self,
        point: vello::kurbo::Point,
        at: Instant,
        env: &Environment,
    ) -> bool {
        let mut changed = self.cancel_active_recognizers(at, env);
        self.active_recognizers = self.recognizers_at(point);
        changed
    }

    fn cancel_active_recognizers(&mut self, at: Instant, env: &Environment) -> bool {
        let active = core::mem::take(&mut self.active_recognizers);
        Self::dispatch_to_recognizers(&active, GestureInput::PointerCancel { at }, env)
    }

    fn dispatch_to_active_recognizers(&mut self, input: GestureInput, env: &Environment) -> bool {
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

    fn target_priority(target: &GestureTarget, index: usize) -> (usize, usize, usize) {
        (target.depth, target.order, index)
    }

    fn recognizers_at(&self, point: vello::kurbo::Point) -> Vec<GestureTarget> {
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

    fn top_group_id_at(&self, point: vello::kurbo::Point) -> Option<usize> {
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

    #[cfg(test)]
    pub(crate) fn debug_targets_at(
        &self,
        point: vello::kurbo::Point,
    ) -> Vec<(usize, usize, usize)> {
        self.targets
            .iter()
            .filter(|target| target.bounds.contains(point))
            .map(|target| (target.depth, target.order, target.group_id))
            .collect()
    }
}

struct TapDetector {
    required_count: u32,
    pressed_point: Option<vello::kurbo::Point>,
    streak: u32,
    last_tap_at: Option<Instant>,
    last_tap_point: Option<vello::kurbo::Point>,
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
                    location: GesturePoint::new(point.x as f32, point.y as f32),
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
    started_point: Option<vello::kurbo::Point>,
    fired: bool,
}

impl LongPressDetector {
    fn new(duration: Duration) -> Self {
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
                    location: GesturePoint::new(point.x as f32, point.y as f32),
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
                    location: GesturePoint::new(started_point.x as f32, started_point.y as f32),
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
    start_point: Option<vello::kurbo::Point>,
    last_point: Option<vello::kurbo::Point>,
    last_at: Option<Instant>,
    started: bool,
}

impl DragDetector {
    fn new(min_distance: f32) -> Self {
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

                let dx = (point.x - start_point.x) as f32;
                let dy = (point.y - start_point.y) as f32;
                let distance = dx.hypot(dy);
                let dt = at
                    .duration_since(previous_at)
                    .as_secs_f32()
                    .max(f32::EPSILON);
                let velocity = GesturePoint::new(
                    ((point.x - previous_point.x) as f32) / dt,
                    ((point.y - previous_point.y) as f32) / dt,
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
                        location: GesturePoint::new(point.x as f32, point.y as f32),
                        translation: GesturePoint::new(dx, dy),
                        velocity,
                    }));
                }

                GestureDetection::recognized(GesturePayload::Drag(DragEvent {
                    phase: GesturePhase::Updated,
                    location: GesturePoint::new(point.x as f32, point.y as f32),
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
                let dx = (point.x - start_point.x) as f32;
                let dy = (point.y - start_point.y) as f32;
                let dt = at
                    .duration_since(previous_at)
                    .as_secs_f32()
                    .max(f32::EPSILON);
                let velocity = GesturePoint::new(
                    ((point.x - previous_point.x) as f32) / dt,
                    ((point.y - previous_point.y) as f32) / dt,
                );
                if !self.started {
                    self.reset();
                    return GestureDetection::failed();
                }
                self.reset();
                GestureDetection::recognized(GesturePayload::Drag(DragEvent {
                    phase: GesturePhase::Ended,
                    location: GesturePoint::new(point.x as f32, point.y as f32),
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
                let dx = (point.x - start_point.x) as f32;
                let dy = (point.y - start_point.y) as f32;
                self.reset();
                GestureDetection::recognized(GesturePayload::Drag(DragEvent {
                    phase: GesturePhase::Cancelled,
                    location: GesturePoint::new(point.x as f32, point.y as f32),
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
    fn new(initial_scale: f32) -> Self {
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
                    center: GesturePoint::new(center.x as f32, center.y as f32),
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
                    center: GesturePoint::new(center.x as f32, center.y as f32),
                    scale: self.scale,
                    velocity: delta / dt,
                }))
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                let payload = GestureDetection::recognized(GesturePayload::Magnification(
                    MagnificationEvent {
                        phase: mapped_phase,
                        center: GesturePoint::new(center.x as f32, center.y as f32),
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
}

impl RotationDetector {
    fn new() -> Self {
        Self {
            active: false,
            angle: 0.0,
        }
    }
}

impl GestureDetector for RotationDetector {
    fn input(&mut self, input: GestureInput) -> GestureDetection {
        let GestureInput::Rotation {
            center,
            delta,
            phase,
            ..
        } = input
        else {
            return GestureDetection::default();
        };
        let _ = center;
        match phase {
            TouchPhase::Started => {
                self.active = true;
                self.angle = 0.0;
                GestureDetection::recognized(GesturePayload::None)
            }
            TouchPhase::Moved => {
                if !self.active {
                    return GestureDetection::default();
                }
                self.angle += delta;
                GestureDetection::recognized(GesturePayload::None)
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                if !self.active {
                    return GestureDetection::default();
                }
                self.active = false;
                self.angle += delta;
                GestureDetection::recognized(GesturePayload::None)
            }
        }
    }

    fn reset(&mut self) {
        self.active = false;
        self.angle = 0.0;
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
        if detection.recognized.is_some() {
            self.awaiting_second = false;
            self.first.reset();
            return GestureDetection::recognized(GesturePayload::None);
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
        if first.recognized.is_some() {
            return GestureDetection::recognized(GesturePayload::None);
        }
        let second = self.second.input(input);
        if second.recognized.is_some() {
            return GestureDetection::recognized(GesturePayload::None);
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
        if first.recognized.is_some() {
            self.second.reset();
            self.suppress_until = Some(now + EXCLUSIVE_RECOGNITION_WINDOW);
            return GestureDetection::recognized(GesturePayload::None);
        }

        let second = self.second.input(input);
        if second.recognized.is_some() {
            self.first.reset();
            self.suppress_until = Some(now + EXCLUSIVE_RECOGNITION_WINDOW);
            return GestureDetection::recognized(GesturePayload::None);
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

fn map_touch_phase_to_gesture_phase(phase: TouchPhase) -> GesturePhase {
    match phase {
        TouchPhase::Started => GesturePhase::Started,
        TouchPhase::Moved => GesturePhase::Updated,
        TouchPhase::Ended => GesturePhase::Ended,
        TouchPhase::Cancelled => GesturePhase::Cancelled,
    }
}

fn gesture_input_instant(input: GestureInput) -> Instant {
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
        let point = vello::kurbo::Point::new(12.0, 24.0);

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
        let origin = vello::kurbo::Point::new(0.0, 0.0);

        detector.input(GestureInput::PointerDown {
            point: origin,
            at: start,
        });

        let below_threshold = detector.input(GestureInput::PointerMove {
            point: vello::kurbo::Point::new(6.0, 2.0),
            at: start + Duration::from_millis(16),
        });
        assert!(below_threshold.recognized.is_none());

        let started = detector.input(GestureInput::PointerMove {
            point: vello::kurbo::Point::new(12.0, 0.0),
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
            point: vello::kurbo::Point::new(24.0, 6.0),
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
            point: vello::kurbo::Point::new(30.0, 8.0),
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
        let center = vello::kurbo::Point::new(10.0, 20.0);

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
        let point = vello::kurbo::Point::new(5.0, 7.0);

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
        assert!(matches!(second.recognized, Some(GesturePayload::None)));
    }

    #[test]
    fn simultaneous_detector_fires_when_any_child_recognizes() {
        let mut detector = SimultaneousDetector::new(
            Box::new(TapDetector::new(1)),
            Box::new(LongPressDetector::new(Duration::from_millis(100))),
        );
        let start = Instant::now();
        let point = vello::kurbo::Point::new(2.0, 3.0);

        detector.input(GestureInput::PointerDown { point, at: start });
        let recognized = detector.input(GestureInput::PointerUp {
            point,
            at: start + Duration::from_millis(10),
        });
        assert!(matches!(recognized.recognized, Some(GesturePayload::None)));
    }

    #[test]
    fn tap_fails_after_pointer_moves_beyond_spatial_tolerance() {
        let mut detector = TapDetector::new(1);
        let start = Instant::now();
        let origin = vello::kurbo::Point::new(0.0, 0.0);
        let moved_point = vello::kurbo::Point::new(TAP_SPATIAL_TOLERANCE + 1.0, 0.0);

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
        let bounds = vello::kurbo::Rect::new(0.0, 0.0, 128.0, 128.0);
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
        let origin = vello::kurbo::Point::new(16.0, 16.0);
        let moved = vello::kurbo::Point::new(48.0, 16.0);
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
        let bounds = vello::kurbo::Rect::new(0.0, 0.0, 128.0, 128.0);
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
        let center = vello::kurbo::Point::new(32.0, 32.0);
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
