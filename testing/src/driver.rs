use std::time::{Duration, Instant};

use accesskit::{
    ActionRequest as AccessibilityActionRequest, TreeUpdate as AccessibilityTreeUpdate,
};
use hydrolysis::{
    FrameProfile, HeadlessRuntime, InputEvent, KeyCode, KeyState, Modifiers, PointerButton,
    PointerKind, SemanticRuntime, TouchPhase,
};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, get_current_pid};

use crate::semantics::NodeId;
use crate::snapshot::Snapshot;

const TEST_POINTER_ID: u64 = 0;

/// Virtual frame step applied per pump.
///
/// The animation clock advances by exactly this much on every pump, so
/// transition sampling is deterministic regardless of how fast the host
/// executes pumps — wall-clock scheduling jitter never leaks into captures.
pub const VIRTUAL_FRAME: Duration = Duration::from_millis(16);

/// The runtime contract a [`crate::SemanticApp`] drives.
///
/// Implemented by Hydrolysis's two headless runtimes: [`SemanticRuntime`], the
/// semantic pipeline a [`crate::UiBuilder`] mounts with [`crate::UiBuilder::mount`],
/// and [`HeadlessRuntime`], the rendered pipeline a styled builder mounts with
/// `UiBuilder<Styled<S>>::mount_offscreen`. The trait is closed to this crate's
/// use: every method is a pass-through to the runtime's inherent API, so a
/// downstream crate never needs to name it — [`crate::SemanticApp`] is
/// parameterized by the runtime itself.
pub trait RuntimeDriver {
    /// Pumps one frame at `at`. `capture_snapshot` requests a framebuffer
    /// readback; runtimes that render honor it, the semantic runtime ignores
    /// it and returns no snapshot.
    fn pump_at(&mut self, at: Instant, capture_snapshot: bool) -> DriverPumpResult;
    /// Whether the mounted runtime is quiescent: no queued input, no spawned
    /// work awaiting a drain, and no renderer-scheduled semantic work.
    fn is_settled(&self) -> bool;
    /// Whether a state change has been requested but not yet flushed, so the
    /// tree the last pump produced no longer reflects the app's state.
    ///
    /// Narrower than [`Self::is_settled`]: work that continues over future
    /// frames of its own accord (animations, gliding scrolls) does not count,
    /// so waiting on this terminates even in an app that never comes to rest.
    fn has_pending_semantic_update(&self) -> bool;
    /// Returns whether the runtime handled the accessibility action.
    fn perform_accessibility_action(&mut self, request: AccessibilityActionRequest) -> bool;
    /// Queues an input event for the next pump.
    ///
    /// Only keyboard and IME input has a semantic target — the focused node —
    /// so the semantic runtime drops geometry-routed events; a session that
    /// needs them mounts the rendered runtime, where this is meaningful.
    fn push_input_event(&mut self, event: InputEvent);
    /// Requests a re-emit on the next pump, as a platform's redraw request
    /// would.
    fn request_redraw(&mut self);
    /// Returns whether anything held UI focus to clear.
    fn clear_ui_focus(&mut self) -> bool;
}

/// What one pump produced: whether it rebuilt, the phase timings, the
/// accessibility tree update, the captured snapshot when one was requested,
/// and the UI focus target when the runtime tracks one.
#[derive(Debug)]
pub struct DriverPumpResult {
    pub(crate) rebuilt: bool,
    pub(crate) profile: FrameProfile,
    pub(crate) tree_update: Option<AccessibilityTreeUpdate>,
    pub(crate) snapshot: Option<Snapshot>,
    pub(crate) ui_focus: Option<NodeId>,
}

impl RuntimeDriver for SemanticRuntime {
    fn pump_at(&mut self, at: Instant, _capture_snapshot: bool) -> DriverPumpResult {
        let result = Self::pump_at(self, at);
        DriverPumpResult {
            rebuilt: result.rebuilt,
            profile: result.profile,
            tree_update: result.tree_update,
            snapshot: None,
            ui_focus: result.ui_focus.map(NodeId::from),
        }
    }

    fn is_settled(&self) -> bool {
        Self::is_settled(self)
    }

    fn has_pending_semantic_update(&self) -> bool {
        Self::has_pending_semantic_update(self)
    }

    fn perform_accessibility_action(&mut self, request: AccessibilityActionRequest) -> bool {
        Self::perform_accessibility_action(self, request)
    }

    fn push_input_event(&mut self, event: InputEvent) {
        Self::push_input_event(self, event);
    }

    fn request_redraw(&mut self) {
        Self::request_redraw(self);
    }

    fn clear_ui_focus(&mut self) -> bool {
        Self::clear_ui_focus(self)
    }
}

impl RuntimeDriver for HeadlessRuntime {
    fn pump_at(&mut self, at: Instant, capture_snapshot: bool) -> DriverPumpResult {
        let result = Self::pump_at(self, capture_snapshot, at);
        DriverPumpResult {
            rebuilt: result.rebuilt,
            profile: result.profile,
            tree_update: result.tree_update,
            snapshot: result.snapshot.map(|snapshot| Snapshot {
                width: snapshot.width,
                height: snapshot.height,
                rgba8: snapshot.rgba8,
            }),
            ui_focus: result.ui_focus.map(NodeId::from),
        }
    }

    fn is_settled(&self) -> bool {
        Self::is_settled(self)
    }

    fn has_pending_semantic_update(&self) -> bool {
        Self::has_pending_semantic_update(self)
    }

    fn perform_accessibility_action(&mut self, request: AccessibilityActionRequest) -> bool {
        Self::perform_accessibility_action(self, request)
    }

    fn push_input_event(&mut self, event: InputEvent) {
        Self::push_input_event(self, event);
    }

    fn request_redraw(&mut self) {
        Self::request_redraw(self);
    }

    fn clear_ui_focus(&mut self) -> bool {
        Self::clear_ui_focus(self)
    }
}

/// Timing collected for one complete offscreen Hydrolysis frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameTiming {
    /// Wall-clock duration spent advancing one offscreen Hydrolysis frame.
    pub total: std::time::Duration,
    /// Whether the frame rebuilt scene/layout state.
    pub rebuilt: bool,
    /// Detailed Hydrolysis phase timings and counters.
    pub profile: FrameProfile,
    /// Process CPU / memory sample collected immediately after the frame.
    pub resources: ResourceSample,
}

/// Host process resource sample captured during a perf run.
#[derive(Clone, Copy, Debug, Default)]
pub struct ResourceSample {
    /// Process CPU usage percentage reported by the operating system.
    pub cpu_percent: f32,
    /// Resident memory in bytes.
    pub memory_bytes: u64,
}

/// Samples process CPU and memory for perf frames.
pub struct ResourceSampler {
    system: Option<System>,
    pid: Option<sysinfo::Pid>,
}

impl ResourceSampler {
    pub const fn new() -> Self {
        Self {
            system: None,
            pid: None,
        }
    }

    pub fn sample(&mut self) -> ResourceSample {
        let pid = *self.pid.get_or_insert_with(|| {
            get_current_pid().expect("waterui-testing perf: failed to resolve current process id")
        });
        let system = self.system.get_or_insert_with(|| {
            let mut system = System::new();
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                true,
                ProcessRefreshKind::nothing().with_cpu().with_memory(),
            );
            system
        });
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        let process = system
            .process(pid)
            .expect("waterui-testing perf: current process disappeared during sampling");
        ResourceSample {
            cpu_percent: process.cpu_usage(),
            memory_bytes: process.memory(),
        }
    }
}

pub const fn pointer_move_event(x: f32, y: f32) -> InputEvent {
    InputEvent::PointerMove {
        id: TEST_POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
    }
}

pub const fn pointer_down_event(x: f32, y: f32) -> InputEvent {
    InputEvent::PointerDown {
        id: TEST_POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    }
}

pub const fn pointer_up_event(x: f32, y: f32) -> InputEvent {
    InputEvent::PointerUp {
        id: TEST_POINTER_ID,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    }
}

/// Presses and releases the secondary button, which is what opens a context
/// menu.
pub const fn secondary_click_events(x: f32, y: f32) -> [InputEvent; 2] {
    [
        InputEvent::PointerDown {
            id: TEST_POINTER_ID,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Secondary,
        },
        InputEvent::PointerUp {
            id: TEST_POINTER_ID,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Secondary,
        },
    ]
}

pub const fn scroll_event(x: f32, y: f32, dx: f32, dy: f32, is_line_delta: bool) -> InputEvent {
    InputEvent::Scroll {
        x,
        y,
        dx,
        dy,
        is_line_delta,
    }
}

pub const fn text_input_event(text: String) -> InputEvent {
    InputEvent::TextInput { text }
}

pub fn key_press_event(key: KeyCode, modifiers: Modifiers) -> InputEvent {
    InputEvent::Key {
        logical_key: key.to_w3c_key(),
        // A synthesized keystroke has no physical key behind it.
        physical_code: hydrolysis::keyboard_types::Code::Unidentified,
        repeat: false,
        key,
        state: KeyState::Pressed,
        modifiers,
    }
}

/// The `Started`/`Moved`/`Ended` sequence one magnification (pinch) gesture
/// dispatches; `factor` is the gesture's cumulative scale.
pub fn magnification_events(x: f32, y: f32, factor: f32) -> [InputEvent; 3] {
    [
        InputEvent::Magnification {
            x,
            y,
            delta: 0.0,
            phase: TouchPhase::Started,
        },
        InputEvent::Magnification {
            x,
            y,
            delta: factor - 1.0,
            phase: TouchPhase::Moved,
        },
        InputEvent::Magnification {
            x,
            y,
            delta: 0.0,
            phase: TouchPhase::Ended,
        },
    ]
}
