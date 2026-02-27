use waterui::app::App;
use waterui::window::Window;
use waterui_core::{AnyView, Environment};

use crate::platform::PlatformWindow;
#[cfg(not(feature = "winit"))]
use crate::platform::OffscreenWindow;
use crate::renderer::HydrolysisRenderer;
#[cfg(not(feature = "winit"))]
use std::mem;

struct RuntimeWindow<P: PlatformWindow> {
    window: Window,
    content: Option<AnyView>,
    platform: P,
    renderer: HydrolysisRenderer,
    rendered_once: bool,
}

impl<P: PlatformWindow> RuntimeWindow<P> {
    fn new(window: Window, content: AnyView, platform: P, renderer: HydrolysisRenderer) -> Self {
        Self {
            window,
            content: Some(content),
            platform,
            renderer,
            rendered_once: false,
        }
    }
}

fn create_bounds(width: u32, height: u32) -> vello::kurbo::Rect {
    vello::kurbo::Rect::new(0.0, 0.0, width as f64, height as f64)
}

fn render_window<P: PlatformWindow>(runtime: &mut RuntimeWindow<P>, env: &Environment) {
    runtime.platform.apply_properties(&runtime.window);
    let surface = runtime.platform.surface();
    let (width, height) = surface.size();
    runtime
        .renderer
        .set_frame_resources(surface.device(), surface.queue());

    if let Some(content) = runtime.content.take() {
        runtime.renderer.reset_scene();
        runtime
            .renderer
            .dispatch(content, env, create_bounds(width, height));
    }

    let frame = surface
        .acquire()
        .expect("hydrolysis runner: failed to acquire frame");
    runtime.renderer.render_scene_to_texture(
        surface.device(),
        surface.queue(),
        frame.view(),
        width,
        height,
    );
    runtime.renderer.clear_frame_resources();
    surface.present(frame);
    runtime.rendered_once = true;
}

#[cfg(not(feature = "winit"))]
pub fn run(app: App) {
    let (windows, env) = app.into_parts();
    for mut window in windows {
        let frame = window.frame.get();
        let width = frame.width().max(1.0) as u32;
        let height = frame.height().max(1.0) as u32;
        let mut platform = OffscreenWindow::new(width, height, wgpu::TextureFormat::Rgba8Unorm);
        platform.apply_properties(&window);
        let renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };
        let content = mem::take(&mut window.content);
        let mut runtime = RuntimeWindow::new(window, content, platform, renderer);
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

    use crate::platform::{InputEvent, PlatformWindow, WinitWindow};
    use crate::renderer::HydrolysisRenderer;
    use crate::runner::{RuntimeWindow, render_window};

    pub fn run(app: App) {
        let (windows, env) = app.into_parts();
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
            mut window: Window,
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
            let content = mem::take(&mut window.content);
            RuntimeWindow::new(window, content, platform, renderer)
        }

        fn mount_pending_windows(&mut self, event_loop: &ActiveEventLoop) {
            let pending = mem::take(&mut self.pending_windows);
            for window in pending {
                let runtime = self.create_runtime_window(event_loop, window);
                let id = runtime.platform.id();
                self.windows.insert(id, runtime);
            }
        }

        fn handle_input_events(runtime: &mut RuntimeWindow<WinitWindow>) -> bool {
            let mut should_close = runtime.window.state.get() == WindowState::Closed;
            for event in runtime.platform.drain_events() {
                match event {
                    InputEvent::CloseRequested => {
                        runtime.window.state.set(WindowState::Closed);
                        should_close = true;
                    }
                    InputEvent::Resize { .. } => {
                        runtime.platform.request_redraw();
                    }
                    _ => {}
                }
            }
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
            let should_close = Self::handle_input_events(runtime);

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
            for runtime in self.windows.values() {
                if !runtime.rendered_once {
                    runtime.platform.request_redraw();
                }
            }
            self.remove_closed_windows(event_loop);
        }
    }
}
