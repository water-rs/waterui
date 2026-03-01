use waterui::app::App;
use waterui::window::Window;
use waterui_core::Environment;

#[cfg(not(feature = "winit"))]
use crate::platform::OffscreenWindow;
use crate::platform::PlatformWindow;
use crate::renderer::HydrolysisRenderer;

struct RuntimeWindow<P: PlatformWindow> {
    window: Window,
    platform: P,
    renderer: HydrolysisRenderer,
    needs_rebuild: bool,
}

impl<P: PlatformWindow> RuntimeWindow<P> {
    fn new(window: Window, platform: P, renderer: HydrolysisRenderer) -> Self {
        Self {
            window,
            platform,
            renderer,
            needs_rebuild: true,
        }
    }
}

fn create_bounds(width: u32, height: u32) -> vello::kurbo::Rect {
    vello::kurbo::Rect::new(0.0, 0.0, width as f64, height as f64)
}

fn render_window<P: PlatformWindow>(runtime: &mut RuntimeWindow<P>, env: &Environment) {
    runtime.platform.apply_properties(&runtime.window);
    {
        let surface = runtime.platform.surface();
        let (width, height) = surface.size();
        let format = surface.format();
        runtime
            .renderer
            .set_frame_resources(surface.device(), surface.queue());

        if runtime.renderer.advance_animations() {
            runtime.needs_rebuild = true;
        }
        let should_rebuild = runtime.needs_rebuild || runtime.renderer.take_rebuild_request();
        if should_rebuild {
            runtime.renderer.reset_scene();
            runtime.renderer.begin_rebuild_frame();
            let content = runtime.window.build_content();
            runtime
                .renderer
                .dispatch(content, env, create_bounds(width, height));
            runtime.renderer.finish_rebuild_frame();
            runtime.needs_rebuild = false;
        }

        let frame = surface
            .acquire()
            .expect("hydrolysis runner: failed to acquire frame");
        runtime.renderer.render_scene_to_surface(
            surface.device(),
            surface.queue(),
            frame.view(),
            format,
            width,
            height,
        );
        runtime.renderer.clear_frame_resources();
        surface.present(frame);
    }

    runtime
        .platform
        .sync_text_input_state(runtime.renderer.focused_text_input_state());
}

#[cfg(not(feature = "winit"))]
pub fn run(app: App) {
    let (windows, env) = app.into_parts();
    let env = env.extending(waterui_graphics::SceneViewMergeToParent);
    for window in windows {
        let frame = window.frame.get();
        let width = frame.width().max(1.0) as u32;
        let height = frame.height().max(1.0) as u32;
        let mut platform = OffscreenWindow::new(width, height, wgpu::TextureFormat::Rgba8Unorm);
        platform.apply_properties(&window);
        let renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };
        let mut runtime = RuntimeWindow::new(window, platform, renderer);
        render_window(&mut runtime, &env);
    }
}

#[cfg(feature = "winit")]
pub fn run(app: App) {
    winit_runner::run(app);
}

#[cfg(feature = "winit")]
mod winit_runner {
    use std::collections::HashMap;
    use std::mem;
    use std::sync::Arc;

    use nami::Signal;
    use waterui::app::App;
    use waterui::window::{Window, WindowState};
    use waterui_core::Environment;
    use winit::application::ApplicationHandler;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
    use winit::window::{Window as NativeWindow, WindowId};

    use crate::platform::{InputEvent, KeyState, PlatformWindow, WinitWindow};
    use crate::renderer::HydrolysisRenderer;
    use crate::runner::{RuntimeWindow, render_window};

    pub fn run(app: App) {
        let (windows, env) = app.into_parts();
        let env = env.extending(waterui_graphics::SceneViewMergeToParent);
        let mut runner = WinitRunner {
            env,
            pending_windows: windows,
            windows: HashMap::new(),
        };

        let event_loop = EventLoop::new().expect("hydrolysis runner: failed to create event loop");
        event_loop.set_control_flow(ControlFlow::Wait);
        event_loop
            .run_app(&mut runner)
            .expect("hydrolysis runner: event loop failed");
    }

    struct WinitRunner {
        env: Environment,
        pending_windows: Vec<Window>,
        windows: HashMap<WindowId, RuntimeWindow<WinitWindow>>,
    }

    impl WinitRunner {
        fn create_runtime_window(
            &mut self,
            event_loop: &ActiveEventLoop,
            window: Window,
        ) -> RuntimeWindow<WinitWindow> {
            let frame = window.frame.get();
            let attributes = NativeWindow::default_attributes()
                .with_title(window.title.get().as_str())
                .with_resizable(window.resizable)
                .with_inner_size(winit::dpi::LogicalSize::new(
                    frame.width() as f64,
                    frame.height() as f64,
                ));

            let native_window = Arc::new(
                event_loop
                    .create_window(attributes)
                    .expect("hydrolysis runner: failed to create winit window"),
            );
            let mut platform = pollster::block_on(WinitWindow::new(native_window));
            platform.apply_properties(&window);
            let renderer = {
                let surface = platform.surface();
                HydrolysisRenderer::new(surface.device())
            };
            RuntimeWindow::new(window, platform, renderer)
        }

        fn mount_pending_windows(&mut self, event_loop: &ActiveEventLoop) {
            let pending = mem::take(&mut self.pending_windows);
            for window in pending {
                let runtime = self.create_runtime_window(event_loop, window);
                let id = runtime.platform.id();
                self.windows.insert(id, runtime);
            }
        }

        fn handle_input_events(
            runtime: &mut RuntimeWindow<WinitWindow>,
            env: &Environment,
        ) -> bool {
            let mut should_close = runtime.window.state.get() == WindowState::Closed;
            for event in runtime.platform.drain_events() {
                match event {
                    InputEvent::CloseRequested => {
                        runtime.window.state.set(WindowState::Closed);
                        should_close = true;
                    }
                    InputEvent::Resize { .. } => {
                        runtime.needs_rebuild = true;
                    }
                    InputEvent::PointerDown { x, y, button } => {
                        if runtime.renderer.handle_pointer_down(x, y, button, env) {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::PointerUp { x, y, button } => {
                        if runtime.renderer.handle_pointer_up(x, y, button, env) {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::PointerMove { x, y } => {
                        if runtime.renderer.handle_pointer_move(x, y, env) {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::Scroll { x, y, dx, dy } => {
                        if runtime.renderer.handle_scroll(x, y, dx, dy) {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::Key {
                        key,
                        state: KeyState::Pressed,
                        modifiers,
                    } => {
                        if runtime.renderer.handle_key(&key, modifiers) {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::ImePreedit { text } => {
                        if runtime.renderer.handle_ime_preedit(text.as_str()) {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::ImeCommit { text } => {
                        if runtime.renderer.handle_ime_commit(text.as_str()) {
                            runtime.needs_rebuild = true;
                        }
                    }
                    InputEvent::ImeDisabled => {
                        if runtime.renderer.handle_ime_disabled() {
                            runtime.needs_rebuild = true;
                        }
                    }
                    _ => {}
                }
            }
            runtime
                .platform
                .sync_text_input_state(runtime.renderer.focused_text_input_state());
            should_close
        }

        fn remove_closed_windows(&mut self, event_loop: &ActiveEventLoop) {
            let mut close_ids = Vec::new();
            for (id, runtime) in &mut self.windows {
                runtime.platform.apply_properties(&runtime.window);
                if runtime.window.state.get() == WindowState::Closed {
                    close_ids.push(*id);
                }
            }

            for id in close_ids {
                self.windows.remove(&id);
            }

            if self.windows.is_empty() && self.pending_windows.is_empty() {
                event_loop.exit();
            }
        }
    }

    impl ApplicationHandler for WinitRunner {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            self.mount_pending_windows(event_loop);
            for runtime in self.windows.values() {
                runtime.platform.request_redraw();
            }
        }

        fn window_event(
            &mut self,
            event_loop: &ActiveEventLoop,
            window_id: WindowId,
            event: WindowEvent,
        ) {
            let Some(runtime) = self.windows.get_mut(&window_id) else {
                return;
            };
            runtime.platform.handle_window_event(&event);
            let should_close = Self::handle_input_events(runtime, &self.env);

            if should_close {
                self.windows.remove(&window_id);
                if self.windows.is_empty() && self.pending_windows.is_empty() {
                    event_loop.exit();
                }
                return;
            }

            if let WindowEvent::RedrawRequested = event {
                render_window(runtime, &self.env);
            }
        }

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            self.mount_pending_windows(event_loop);
            for runtime in self.windows.values_mut() {
                runtime
                    .platform
                    .sync_text_input_state(runtime.renderer.focused_text_input_state());
                if runtime.renderer.advance_animations() {
                    runtime.needs_rebuild = true;
                }
                if runtime.renderer.take_rebuild_request() {
                    runtime.needs_rebuild = true;
                }
                if runtime.needs_rebuild {
                    runtime.platform.request_redraw();
                }
            }
            self.remove_closed_windows(event_loop);
        }
    }
}
