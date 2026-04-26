use accesskit::{
    ActionRequest as AccessibilityActionRequest, TreeUpdate as AccessibilityTreeUpdate,
};
use hydrolysis::{HeadlessRuntime, InputEvent, PointerButton, TouchPhase};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{AnyView, Environment};

use crate::semantics::NodeId;
use crate::snapshot::Snapshot;

pub trait A11yDriver {
    fn pump(
        &mut self,
        content: &AnyViewBuilder<AnyView>,
        env: &Environment,
        capture_snapshot: bool,
    ) -> DriverPumpResult;
    fn perform_action(&mut self, request: AccessibilityActionRequest, env: &Environment) -> bool;
    fn hover_at(&mut self, x: f32, y: f32, env: &Environment) -> bool;
    fn pointer_down(&mut self, x: f32, y: f32, env: &Environment) -> bool;
    fn pointer_move(&mut self, x: f32, y: f32, env: &Environment) -> bool;
    fn pointer_up(&mut self, x: f32, y: f32, env: &Environment) -> bool;
    fn magnify_at(&mut self, x: f32, y: f32, factor: f32, env: &Environment) -> bool;
    fn clear_ui_focus(&mut self, env: &Environment) -> bool;
}

#[derive(Debug)]
pub struct DriverPumpResult {
    pub(crate) rebuilt: bool,
    pub(crate) tree_update: Option<AccessibilityTreeUpdate>,
    pub(crate) snapshot: Option<Snapshot>,
    pub(crate) ui_focus: Option<NodeId>,
}

pub struct HydrolysisA11yDriver {
    width: u32,
    height: u32,
    runtime: Option<HeadlessRuntime>,
}

impl HydrolysisA11yDriver {
    pub(crate) const fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            runtime: None,
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
}

impl A11yDriver for HydrolysisA11yDriver {
    fn pump(
        &mut self,
        content: &AnyViewBuilder<AnyView>,
        env: &Environment,
        capture_snapshot: bool,
    ) -> DriverPumpResult {
        let result = self.runtime(content, env).pump(capture_snapshot);

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

    fn perform_action(&mut self, request: AccessibilityActionRequest, env: &Environment) -> bool {
        let _ = env;
        self.runtime
            .as_mut()
            .expect("waterui-testing driver action requested before runtime initialization")
            .perform_accessibility_action(request)
    }

    fn hover_at(&mut self, x: f32, y: f32, _env: &Environment) -> bool {
        let runtime = self
            .runtime
            .as_mut()
            .expect("waterui-testing hover requested before runtime initialization");
        runtime.push_input_event(InputEvent::PointerMove { x, y });
        true
    }

    fn pointer_down(&mut self, x: f32, y: f32, _env: &Environment) -> bool {
        let runtime = self
            .runtime
            .as_mut()
            .expect("waterui-testing pointer down requested before runtime initialization");
        runtime.push_input_event(InputEvent::PointerDown {
            x,
            y,
            button: PointerButton::Primary,
        });
        true
    }

    fn pointer_move(&mut self, x: f32, y: f32, _env: &Environment) -> bool {
        let runtime = self
            .runtime
            .as_mut()
            .expect("waterui-testing pointer move requested before runtime initialization");
        runtime.push_input_event(InputEvent::PointerMove { x, y });
        true
    }

    fn pointer_up(&mut self, x: f32, y: f32, _env: &Environment) -> bool {
        let runtime = self
            .runtime
            .as_mut()
            .expect("waterui-testing pointer up requested before runtime initialization");
        runtime.push_input_event(InputEvent::PointerUp {
            x,
            y,
            button: PointerButton::Primary,
        });
        true
    }

    fn magnify_at(&mut self, x: f32, y: f32, factor: f32, _env: &Environment) -> bool {
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
        true
    }

    fn clear_ui_focus(&mut self, _env: &Environment) -> bool {
        self.runtime
            .as_mut()
            .expect("waterui-testing clear_ui_focus requested before runtime initialization")
            .clear_ui_focus()
    }
}
