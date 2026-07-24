//! Embedded-device simulator: the embedded rendering flow in a native window.
//!
//! Runs the complete dew pipeline — dispatch, layout, banded rasterization,
//! dirty-region flush — on the host, presenting the simulated panel's
//! framebuffer in a window via `softbuffer`. No cross-compilation involved:
//! this validates everything an embedded target runs except the final
//! `DisplayFlush` sink, exactly like LVGL's SDL simulator or Slint's
//! desktop preview.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Instant;

use waterui_backend_core::input::TouchPhase;
use waterui_core::{AnyView, Environment};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, TouchPhase as WinitTouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::board::{HostBoard, PointerSample};
use crate::frame_cadence::FrameCadence;
use crate::runtime::DewRuntime;

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

    let runtime = DewRuntime::new(HostBoard::new(width, height), env, 16, build_root);
    let event_loop = EventLoop::new().expect("simulator event loop");
    let mut app = SimulatorApp {
        runtime,
        width,
        height,
        title: title.into(),
        on_tick: Box::new(on_tick),
        window: None,
        cadence: FrameCadence::sixty_hz(Instant::now()),
        cursor: None,
        mouse_pressed: false,
        active_touch: None,
    };
    event_loop
        .run_app(&mut app)
        .expect("simulator event loop run");
}

struct SimulatorApp {
    runtime: DewRuntime<HostBoard>,
    width: u32,
    height: u32,
    title: String,
    on_tick: Box<dyn FnMut()>,
    window: Option<PanelWindow>,
    cadence: FrameCadence,
    cursor: Option<(f64, f64)>,
    mouse_pressed: bool,
    active_touch: Option<u64>,
}

struct PanelWindow {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
}

impl SimulatorApp {
    fn panel_position(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let size = self.window.as_ref()?.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return None;
        }
        let panel_x =
            (x * f64::from(self.width) / f64::from(size.width)).clamp(0.0, f64::from(self.width));
        let panel_y = (y * f64::from(self.height) / f64::from(size.height))
            .clamp(0.0, f64::from(self.height));
        Some((panel_x, panel_y))
    }

    fn enqueue_pointer(&mut self, phase: TouchPhase, position: (f64, f64)) {
        self.runtime.board_mut().push_pointer(PointerSample {
            x: position.0,
            y: position.1,
            phase,
        });
    }

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
        let pixels = self.runtime.board().framebuffer().pixels();
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
            WindowEvent::CursorMoved { position, .. } => {
                let Some(position) = self.panel_position(position.x, position.y) else {
                    return;
                };
                self.cursor = Some(position);
                if self.mouse_pressed && self.active_touch.is_none() {
                    self.enqueue_pointer(TouchPhase::Moved, position);
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } if self.active_touch.is_none() => {
                let Some(position) = self.cursor else {
                    return;
                };
                match state {
                    ElementState::Pressed if !self.mouse_pressed => {
                        self.mouse_pressed = true;
                        self.enqueue_pointer(TouchPhase::Started, position);
                    }
                    ElementState::Released if self.mouse_pressed => {
                        self.mouse_pressed = false;
                        self.enqueue_pointer(TouchPhase::Ended, position);
                    }
                    _ => {}
                }
            }
            WindowEvent::Touch(touch) => {
                let accept = match touch.phase {
                    WinitTouchPhase::Started => self.active_touch.is_none(),
                    WinitTouchPhase::Moved
                    | WinitTouchPhase::Ended
                    | WinitTouchPhase::Cancelled => self.active_touch == Some(touch.id),
                };
                if !accept {
                    return;
                }
                let Some(position) = self.panel_position(touch.location.x, touch.location.y) else {
                    return;
                };
                let phase = match touch.phase {
                    WinitTouchPhase::Started => {
                        if self.mouse_pressed {
                            self.mouse_pressed = false;
                            self.enqueue_pointer(TouchPhase::Cancelled, position);
                        }
                        self.active_touch = Some(touch.id);
                        TouchPhase::Started
                    }
                    WinitTouchPhase::Moved => TouchPhase::Moved,
                    WinitTouchPhase::Ended => {
                        self.active_touch = None;
                        TouchPhase::Ended
                    }
                    WinitTouchPhase::Cancelled => {
                        self.active_touch = None;
                        TouchPhase::Cancelled
                    }
                };
                self.enqueue_pointer(phase, position);
            }
            WindowEvent::Focused(false) if self.mouse_pressed => {
                self.mouse_pressed = false;
                if let Some(position) = self.cursor {
                    self.enqueue_pointer(TouchPhase::Cancelled, position);
                }
            }
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
        let deadline = self.cadence.deadline_after_work(Instant::now());
        event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
    }
}
