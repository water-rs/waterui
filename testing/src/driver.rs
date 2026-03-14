use accesskit::{
    ActionRequest as AccessibilityActionRequest, TreeUpdate as AccessibilityTreeUpdate,
};
use hydrolysis::{HydrolysisRenderer, OffscreenWindow, PlatformWindow, PointerButton};
use waterui::component::table::TableConfig;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::view::Hook;
use waterui_core::{AnyView, Environment, Native};

use crate::semantics::NodeId;
use crate::snapshot::{Snapshot, readback_texture_rgba8};

pub(crate) trait A11yDriver {
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
    fn ui_focus(&self) -> Option<NodeId>;
}

#[derive(Debug)]
pub(crate) struct DriverPumpResult {
    pub(crate) rebuilt: bool,
    pub(crate) tree_update: Option<AccessibilityTreeUpdate>,
    pub(crate) snapshot: Option<Snapshot>,
    pub(crate) ui_focus: Option<NodeId>,
}

#[derive(Debug)]
pub(crate) struct HydrolysisA11yDriver {
    platform: OffscreenWindow,
    renderer: HydrolysisRenderer,
    needs_rebuild: bool,
}

impl HydrolysisA11yDriver {
    pub(crate) fn new(width: u32, height: u32) -> Self {
        let mut platform =
            OffscreenWindow::new(width.max(1), height.max(1), wgpu::TextureFormat::Rgba8Unorm);
        let renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };

        Self {
            platform,
            renderer,
            needs_rebuild: true,
        }
    }

    fn schedule_redraw_or_rebuild(&mut self, changed: bool) -> bool {
        if !changed {
            return false;
        }
        if self.renderer.take_rebuild_request() {
            self.needs_rebuild = true;
            return true;
        }
        self.renderer.request_redraw();
        true
    }
}

impl A11yDriver for HydrolysisA11yDriver {
    fn pump(
        &mut self,
        content: &AnyViewBuilder<AnyView>,
        env: &Environment,
        capture_snapshot: bool,
    ) -> DriverPumpResult {
        let surface = self.platform.surface();
        let (width, height) = surface.size();
        let bounds = vello::kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height));

        self.renderer
            .set_frame_resources(surface.device(), surface.queue());

        let animation_dirty = self.renderer.advance_animations();
        if animation_dirty {
            self.renderer.request_redraw();
        }

        let should_rebuild = self.needs_rebuild || self.renderer.take_rebuild_request();
        if should_rebuild {
            self.renderer.begin_rebuild_frame();
            let content = self
                .renderer
                .with_local_state_env(env, |_local_env| content.build());
            self.renderer.reset_scene();
            self.renderer.dispatch(content, env, bounds);
            self.renderer.finish_rebuild_frame();
            self.needs_rebuild = false;
        }

        let frame = surface
            .acquire()
            .expect("waterui-testing driver failed to acquire offscreen frame");
        self.renderer.render_scene_to_texture(
            surface.device(),
            surface.queue(),
            frame.view(),
            surface.format(),
            width,
            height,
            vello::peniko::Color::TRANSPARENT,
        );
        let snapshot = capture_snapshot.then(|| Snapshot {
            width,
            height,
            rgba8: readback_texture_rgba8(
                surface.device(),
                surface.queue(),
                frame.texture(),
                width,
                height,
            ),
        });
        self.renderer.clear_frame_resources();
        surface.present(frame);

        if self.renderer.take_rebuild_request() {
            self.needs_rebuild = true;
        }

        DriverPumpResult {
            rebuilt: should_rebuild || animation_dirty,
            tree_update: self.renderer.take_accessibility_tree_update(),
            snapshot,
            ui_focus: self.renderer.focused_ui_node().map(NodeId::from),
        }
    }

    fn perform_action(&mut self, request: AccessibilityActionRequest, env: &Environment) -> bool {
        let changed = self.renderer.handle_accessibility_action(request, env);
        if changed {
            self.needs_rebuild = true;
        }
        changed
    }

    fn hover_at(&mut self, x: f32, y: f32, env: &Environment) -> bool {
        let changed = self.renderer.handle_pointer_move(x, y, env);
        self.schedule_redraw_or_rebuild(changed)
    }

    fn pointer_down(&mut self, x: f32, y: f32, env: &Environment) -> bool {
        let changed = self
            .renderer
            .handle_pointer_down(x, y, PointerButton::Primary, env);
        self.schedule_redraw_or_rebuild(changed)
    }

    fn pointer_move(&mut self, x: f32, y: f32, env: &Environment) -> bool {
        let changed = self.renderer.handle_pointer_move(x, y, env);
        self.schedule_redraw_or_rebuild(changed)
    }

    fn pointer_up(&mut self, x: f32, y: f32, env: &Environment) -> bool {
        let changed = self
            .renderer
            .handle_pointer_up(x, y, PointerButton::Primary, env);
        self.schedule_redraw_or_rebuild(changed)
    }

    fn magnify_at(&mut self, x: f32, y: f32, factor: f32, env: &Environment) -> bool {
        let changed = self.renderer.apply_magnification_gesture(x, y, factor, env);
        self.schedule_redraw_or_rebuild(changed)
    }

    fn clear_ui_focus(&mut self, _env: &Environment) -> bool {
        let changed = self.renderer.clear_ui_focus();
        self.schedule_redraw_or_rebuild(changed)
    }

    fn ui_focus(&self) -> Option<NodeId> {
        self.renderer.focused_ui_node().map(NodeId::from)
    }
}

pub(crate) fn install_native_component_hooks(env: &mut Environment) {
    env.insert(Hook::new(|_env: &Environment, config: TableConfig| {
        Native::new(config)
    }));
}
