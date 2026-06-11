//! Desktop panel simulator: the embedded rendering flow in a native window.
//!
//! Runs the complete dew pipeline — dispatch, layout, banded rasterization,
//! dirty-region flush — on the host, presenting the simulated panel's
//! framebuffer in a window via `softbuffer`. No cross-compilation involved:
//! this validates everything an embedded target runs except the final
//! `DisplayFlush` sink, exactly like LVGL's SDL simulator or Slint's
//! desktop preview.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use waterui_core::{AnyView, Environment};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::display::BufferDisplay;
use crate::runtime::DewRuntime;

/// Frame-pump cadence while the window is idle.
const TICK: Duration = Duration::from_millis(16);

/// Opens a `width` × `height` simulated panel window rendering
/// `build_root()` until the window is closed.
///
/// `on_tick` runs on the main thread once per frame tick — the place to
/// drive time-based state (bindings must stay on the main thread).
///
/// # Panics
///
/// Panics when the window or presentation surface cannot be created —
/// a simulator without a window has nothing to simulate.
pub fn run(
    width: u32,
    height: u32,
    title: impl Into<String>,
    env: Environment,
    build_root: impl Fn() -> AnyView + 'static,
    on_tick: impl FnMut() + 'static,
) {
    // Reactive text and timers spawn local tasks; give the main thread its
    // executors (no-ops when the host app already initialized them).
    let _ = executor_core::try_init_global_executor(native_executor::NativeExecutor::new());
    let _ = executor_core::try_init_local_executor(native_executor::NativeExecutor::new());

    let runtime = DewRuntime::new(BufferDisplay::new(width, height), env, 16, build_root);
    let event_loop = EventLoop::new().expect("simulator event loop");
    let mut app = SimulatorApp {
        runtime,
        width,
        height,
        title: title.into(),
        on_tick: Box::new(on_tick),
        window: None,
    };
    event_loop
        .run_app(&mut app)
        .expect("simulator event loop run");
}

struct SimulatorApp {
    runtime: DewRuntime<BufferDisplay>,
    width: u32,
    height: u32,
    title: String,
    on_tick: Box<dyn FnMut()>,
    window: Option<PanelWindow>,
}

struct PanelWindow {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
}

impl SimulatorApp {
    fn present(&mut self) {
        let Some(panel) = self.window.as_mut() else {
            return;
        };
        let size = panel.window.inner_size();
        let (Some(surface_width), Some(surface_height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return;
        };
        panel
            .surface
            .resize(surface_width, surface_height)
            .expect("simulator surface resize");
        let mut frame = panel.surface.buffer_mut().expect("simulator frame buffer");
        let pixels = self.runtime.display().pixels();
        for py in 0..size.height {
            let src_y = (py * self.height / size.height).min(self.height - 1) as usize;
            for px in 0..size.width {
                let src_x = (px * self.width / size.width).min(self.width - 1) as usize;
                let offset = (src_y * self.width as usize + src_x) * 4;
                let [r, g, b] = [pixels[offset], pixels[offset + 1], pixels[offset + 2]];
                frame[(py * size.width + px) as usize] =
                    (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
            }
        }
        frame.present().expect("simulator frame present");
    }
}

impl ApplicationHandler for SimulatorApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attributes = Window::default_attributes()
            .with_title(self.title.clone())
            .with_inner_size(LogicalSize::new(self.width, self.height))
            .with_resizable(false);
        let window = Rc::new(
            event_loop
                .create_window(attributes)
                .expect("simulator window"),
        );
        let context = softbuffer::Context::new(window.clone()).expect("simulator context");
        let surface =
            softbuffer::Surface::new(&context, window.clone()).expect("simulator surface");
        self.window = Some(PanelWindow { window, surface });
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.present(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        (self.on_tick)();
        if self.runtime.pump().is_some()
            && let Some(panel) = self.window.as_ref()
        {
            panel.window.request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + TICK));
    }
}
