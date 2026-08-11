use std::time::{Duration, Instant};

use accesskit::{
    ActionRequest as AccessibilityActionRequest, TreeUpdate as AccessibilityTreeUpdate,
};
use hydrolysis::{
    FrameProfile, HeadlessRuntime, InputEvent, KeyCode, KeyState, Modifiers, PointerButton,
    PointerKind, TouchPhase,
};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, get_current_pid};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{AnyView, Environment};

use crate::app::DriverMode;
use crate::semantics::NodeId;
use crate::snapshot::Snapshot;

const TEST_POINTER_ID: u64 = 0;

/// Virtual frame step applied per pump.
///
/// The animation clock advances by exactly this much on every pump, so
/// transition sampling is deterministic regardless of how fast the host
/// executes pumps — wall-clock scheduling jitter never leaks into captures.
pub const VIRTUAL_FRAME: Duration = Duration::from_millis(16);

pub trait A11yDriver {
    fn pump(
        &mut self,
        content: &AnyViewBuilder<AnyView>,
        env: &Environment,
        capture_snapshot: bool,
    ) -> DriverPumpResult;
    /// Advances the virtual animation clock by exactly `step` and pumps one
    /// frame without snapshot readback.
    fn pump_step(
        &mut self,
        step: Duration,
        content: &AnyViewBuilder<AnyView>,
        env: &Environment,
    ) -> DriverPumpResult;
    /// Whether the mounted runtime is quiescent: no queued input, no spawned
    /// work awaiting a drain, and no renderer-scheduled semantic work.
    fn is_settled(&self) -> bool;
    /// The current virtual frame instant, if any pump has run yet. Perf runs
    /// seed their own frame clock from this so interleaved clocks stay
    /// monotone.
    fn clock(&self) -> Option<Instant> {
        None
    }
    /// Returns whether the runtime handled the accessibility action.
    fn perform_action(&mut self, request: AccessibilityActionRequest, env: &Environment) -> bool;
    fn hover_at(&mut self, x: f32, y: f32, env: &Environment);
    fn pointer_down(&mut self, x: f32, y: f32, env: &Environment);
    fn pointer_move(&mut self, x: f32, y: f32, env: &Environment);
    fn pointer_up(&mut self, x: f32, y: f32, env: &Environment);
    fn scroll_at(
        &mut self,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        is_line_delta: bool,
        env: &Environment,
    );
    fn text_input(&mut self, text: String, env: &Environment);
    fn key_press(&mut self, key: KeyCode, modifiers: Modifiers, env: &Environment);
    fn magnify_at(&mut self, x: f32, y: f32, factor: f32, env: &Environment);
    /// Returns whether anything held UI focus to clear.
    fn clear_ui_focus(&mut self, env: &Environment) -> bool;
    fn request_redraw(&mut self, content: &AnyViewBuilder<AnyView>, env: &Environment);
    fn pump_frame(&mut self, content: &AnyViewBuilder<AnyView>, env: &Environment) -> FrameTiming;
    fn pump_frame_at(
        &mut self,
        content: &AnyViewBuilder<AnyView>,
        env: &Environment,
        _at: Instant,
    ) -> FrameTiming {
        self.pump_frame(content, env)
    }
}

#[derive(Debug)]
pub struct DriverPumpResult {
    pub(crate) rebuilt: bool,
    pub(crate) tree_update: Option<AccessibilityTreeUpdate>,
    pub(crate) snapshot: Option<Snapshot>,
    pub(crate) ui_focus: Option<NodeId>,
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

pub struct HydrolysisA11yDriver {
    width: u32,
    height: u32,
    mode: DriverMode,
    runtime: Option<HeadlessRuntime>,
    /// Virtual frame clock: starts at the first pump's wall time and advances
    /// by a fixed step per pump, decoupling animation sampling from host
    /// scheduling. Perf pumps overwrite it so interleaved clocks stay
    /// monotone.
    clock: Option<Instant>,
    resources: ResourceSampler,
}

impl HydrolysisA11yDriver {
    pub(crate) const fn new(width: u32, height: u32, mode: DriverMode) -> Self {
        Self {
            width,
            height,
            mode,
            runtime: None,
            clock: None,
            resources: ResourceSampler::new(),
        }
    }

    fn runtime(
        &mut self,
        content: &AnyViewBuilder<AnyView>,
        env: &Environment,
    ) -> &mut HeadlessRuntime {
        self.runtime.get_or_insert_with(|| {
            HeadlessRuntime::new_for_tests(env.clone(), content.clone(), self.width, self.height)
        })
    }

    /// Advances the virtual clock by `step` and returns the new frame instant.
    fn tick(&mut self, step: Duration) -> Instant {
        let next = self
            .clock
            .map_or_else(Instant::now, |current| current + step);
        self.clock = Some(next);
        next
    }

    fn convert(result: hydrolysis::HeadlessPumpResult) -> DriverPumpResult {
        DriverPumpResult {
            rebuilt: result.rebuilt,
            tree_update: result.tree_update,
            snapshot: result.snapshot.map(|snapshot| Snapshot {
                width: snapshot.width,
                height: snapshot.height,
                rgba8: snapshot.rgba8,
            }),
            ui_focus: result.ui_focus.map(NodeId::from),
        }
    }
}

impl A11yDriver for HydrolysisA11yDriver {
    fn pump(
        &mut self,
        content: &AnyViewBuilder<AnyView>,
        env: &Environment,
        capture_snapshot: bool,
    ) -> DriverPumpResult {
        let at = self.tick(VIRTUAL_FRAME);
        let result = if capture_snapshot {
            self.runtime(content, env).pump_at(true, at)
        } else {
            match self.mode {
                DriverMode::Semantic => self.runtime(content, env).pump_semantic_at(at),
                DriverMode::Offscreen => self.runtime(content, env).pump_at(false, at),
            }
        };
        Self::convert(result)
    }

    fn pump_step(
        &mut self,
        step: Duration,
        content: &AnyViewBuilder<AnyView>,
        env: &Environment,
    ) -> DriverPumpResult {
        let at = self.tick(step);
        let result = match self.mode {
            DriverMode::Semantic => self.runtime(content, env).pump_semantic_at(at),
            DriverMode::Offscreen => self.runtime(content, env).pump_at(false, at),
        };
        Self::convert(result)
    }

    fn is_settled(&self) -> bool {
        self.runtime.as_ref().is_none_or(HeadlessRuntime::is_settled)
    }

    fn clock(&self) -> Option<Instant> {
        self.clock
    }

    fn perform_action(&mut self, request: AccessibilityActionRequest, env: &Environment) -> bool {
        let _ = env;
        self.runtime
            .as_mut()
            .expect("waterui-testing driver action requested before runtime initialization")
            .perform_accessibility_action(request)
    }

    fn hover_at(&mut self, x: f32, y: f32, _env: &Environment) {
        let runtime = self
            .runtime
            .as_mut()
            .expect("waterui-testing hover requested before runtime initialization");
        runtime.push_input_event(InputEvent::PointerMove {
            id: TEST_POINTER_ID,
            kind: PointerKind::Mouse,
            x,
            y,
        });
    }

    fn pointer_down(&mut self, x: f32, y: f32, _env: &Environment) {
        let runtime = self
            .runtime
            .as_mut()
            .expect("waterui-testing pointer down requested before runtime initialization");
        runtime.push_input_event(InputEvent::PointerDown {
            id: TEST_POINTER_ID,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        });
    }

    fn pointer_move(&mut self, x: f32, y: f32, _env: &Environment) {
        let runtime = self
            .runtime
            .as_mut()
            .expect("waterui-testing pointer move requested before runtime initialization");
        runtime.push_input_event(InputEvent::PointerMove {
            id: TEST_POINTER_ID,
            kind: PointerKind::Mouse,
            x,
            y,
        });
    }

    fn pointer_up(&mut self, x: f32, y: f32, _env: &Environment) {
        let runtime = self
            .runtime
            .as_mut()
            .expect("waterui-testing pointer up requested before runtime initialization");
        runtime.push_input_event(InputEvent::PointerUp {
            id: TEST_POINTER_ID,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        });
    }

    fn scroll_at(
        &mut self,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        is_line_delta: bool,
        _env: &Environment,
    ) {
        let runtime = self
            .runtime
            .as_mut()
            .expect("waterui-testing scroll requested before runtime initialization");
        runtime.push_input_event(InputEvent::Scroll {
            x,
            y,
            dx,
            dy,
            is_line_delta,
        });
    }

    fn text_input(&mut self, text: String, _env: &Environment) {
        let runtime = self
            .runtime
            .as_mut()
            .expect("waterui-testing text_input requested before runtime initialization");
        runtime.push_input_event(InputEvent::TextInput { text });
    }

    fn key_press(&mut self, key: KeyCode, modifiers: Modifiers, _env: &Environment) {
        let runtime = self
            .runtime
            .as_mut()
            .expect("waterui-testing key_press requested before runtime initialization");
        runtime.push_input_event(InputEvent::Key {
            key,
            native: None,
            state: KeyState::Pressed,
            modifiers,
        });
    }

    fn magnify_at(&mut self, x: f32, y: f32, factor: f32, _env: &Environment) {
        let runtime = self
            .runtime
            .as_mut()
            .expect("waterui-testing magnify requested before runtime initialization");
        runtime.push_input_event(InputEvent::Magnification {
            x,
            y,
            delta: 0.0,
            phase: TouchPhase::Started,
        });
        runtime.push_input_event(InputEvent::Magnification {
            x,
            y,
            delta: factor - 1.0,
            phase: TouchPhase::Moved,
        });
        runtime.push_input_event(InputEvent::Magnification {
            x,
            y,
            delta: 0.0,
            phase: TouchPhase::Ended,
        });
    }

    fn clear_ui_focus(&mut self, _env: &Environment) -> bool {
        self.runtime
            .as_mut()
            .expect("waterui-testing clear_ui_focus requested before runtime initialization")
            .clear_ui_focus()
    }

    fn request_redraw(&mut self, content: &AnyViewBuilder<AnyView>, env: &Environment) {
        self.runtime(content, env).request_redraw();
    }

    fn pump_frame(&mut self, content: &AnyViewBuilder<AnyView>, env: &Environment) -> FrameTiming {
        let at = self.tick(VIRTUAL_FRAME);
        self.pump_frame_at(content, env, at)
    }

    fn pump_frame_at(
        &mut self,
        content: &AnyViewBuilder<AnyView>,
        env: &Environment,
        at: Instant,
    ) -> FrameTiming {
        // Adopt the caller's clock so interleaved semantic pumps stay monotone.
        self.clock = Some(at);
        let started_at = std::time::Instant::now();
        let outcome = self.runtime(content, env).pump_at(false, at);
        FrameTiming {
            total: outcome.profile.total.max(started_at.elapsed()),
            rebuilt: outcome.rebuilt,
            profile: outcome.profile,
            resources: self.resources.sample(),
        }
    }
}

struct ResourceSampler {
    system: Option<System>,
    pid: Option<sysinfo::Pid>,
}

impl ResourceSampler {
    const fn new() -> Self {
        Self {
            system: None,
            pid: None,
        }
    }

    fn sample(&mut self) -> ResourceSample {
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
