//! Render-on-demand for embedded GPU surfaces.
//!
//! Hydrolysis redraws its whole window scene on every frame it runs, and that
//! stays true here — none of this is damage tracking. What these tests pin is
//! that an embedded surface's offscreen texture is an *input* to that
//! composite, retained across frames exactly like the render tree it hangs in.
//! A view that asked for nothing, at an unchanged size and scale, with
//! unchanged pointer and gesture state, is composited from the texture it
//! already filled instead of being asked to fill it again.
//!
//! Every test drives the real runner path: a second GPU surface that asks for a
//! redraw from inside `render` keeps the window pumping frames, which is what
//! makes "the probe did not render" mean something — the frames really
//! happened, and the probe sat them out.

use core::cell::RefCell;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::time::Duration;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use waterui::{Binding, ViewExt as _};
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::gpu::{Context as GpuContext, Frame as GpuFrame};
use waterui_graphics::input::SurfaceInputEvent;
use waterui_graphics::{GpuContent, GpuContentView, RedrawHandle};
use waterui_layout::frame::Frame;
use waterui_layout::stack::vstack;

use super::{MinimalTestTheme, pumped_test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerKind};

const WINDOW_WIDTH: u32 = 320;
const WINDOW_HEIGHT: u32 = 320;
const DRIVER_HEIGHT: f32 = 100.0;
const PROBE_WIDTH: f32 = 120.0;
const PROBE_HEIGHT: f32 = 90.0;

/// Where the probe lands in window coordinates: the column anchors at the
/// window's top, so the probe sits under the full-width 100-high driver plus
/// the stack's 10-point default spacing, centred across the 320-wide window.
const PROBE_ORIGIN_X: f64 = 100.0;
const PROBE_ORIGIN_Y: f64 = 110.0;

const POINTER_ID: u64 = 3;

/// A view that counts the frames it is actually asked to render, and that can
/// be switched between idle and continuously animating from the test body.
/// The `redraw` slot captures the [`Context`] redraw handle the engine hands
/// the view at setup, so the test can drive an out-of-band redraw request —
/// the path a decoder or compositor callback takes.
#[derive(Clone, Default)]
struct RenderCounter {
    renders: Arc<AtomicU32>,
    animating: Arc<AtomicBool>,
    redraw: Arc<Mutex<Option<RedrawHandle>>>,
}

impl RenderCounter {
    /// A counter for a view that asks for another frame from every frame — the
    /// window's animating element.
    fn animating() -> Self {
        let counter = Self::default();
        counter.set_animating(true);
        counter
    }

    fn count(&self) -> u32 {
        self.renders.load(Ordering::Relaxed)
    }

    fn set_animating(&self, animating: bool) {
        self.animating.store(animating, Ordering::Relaxed);
    }

    /// Requests another frame through the `Context` redraw handle — the same
    /// call an asynchronously producing content (a video decoder, a
    /// compositor callback) makes from off the render path.
    fn request_redraw(&self) {
        self.redraw
            .lock()
            .expect("redraw slot")
            .as_ref()
            .expect("setup stored the redraw handle")
            .request_redraw();
    }
}

struct CountingView(RenderCounter);

impl GpuContent for CountingView {
    fn setup(&mut self, gpu: &GpuContext<'_>) {
        *self.0.redraw.lock().expect("redraw slot") = Some(gpu.redraw.clone());
    }

    fn render(&mut self, frame: &mut GpuFrame<'_>) {
        self.0.renders.fetch_add(1, Ordering::Relaxed);
        if self.0.animating.load(Ordering::Relaxed) {
            frame.request_redraw();
        }
    }
}

/// A window holding an always-animating surface above a probe surface whose
/// width the test owns.
fn runtime_with(
    driver: &RenderCounter,
    probe: &RenderCounter,
    probe_width: &Binding<f32>,
) -> HeadlessRuntime {
    runtime_with_probe_view(
        driver,
        GpuContentView::new(CountingView(probe.clone())),
        probe_width,
    )
}

/// The same window, with the probe surface's view supplied by the test so it
/// can carry `on_input` / `on_ime_caret` handlers.
fn runtime_with_probe_view(
    driver: &RenderCounter,
    probe_view: GpuContentView,
    probe_width: &Binding<f32>,
) -> HeadlessRuntime {
    let views = RefCell::new(Some((driver.clone(), probe_view, probe_width.clone())));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let (driver, probe_view, probe_width) = views
            .borrow_mut()
            .take()
            .expect("the probe window is built once");
        AnyView::new(vstack((
            GpuContentView::new(CountingView(driver))
                .size(crate::num_cast::u32_as_f32(WINDOW_WIDTH), DRIVER_HEIGHT),
            Frame::new(probe_view)
                .width(probe_width)
                .height(PROBE_HEIGHT),
        )))
    });
    HeadlessRuntime::new_for_tests(
        pumped_test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    )
}

/// Pumps window frames on a 16ms cadence from a fixed origin, so the frame
/// clock a test drives is deterministic rather than wall-clock.
struct Frames {
    start: Instant,
    next: u64,
}

impl Frames {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            next: 0,
        }
    }

    fn pump(&mut self, runtime: &mut HeadlessRuntime, count: u64) {
        for _ in 0..count {
            let at = self.start + Duration::from_millis(self.next * 16);
            self.next += 1;
            let _ = runtime.pump_at(false, at);
        }
    }
}

/// Pumps until the surfaces have finished their async setup and the probe has
/// filled its texture once. Returns the probe's render count, which every test
/// measures from.
fn settled(
    runtime: &mut HeadlessRuntime,
    frames: &mut Frames,
    driver: &RenderCounter,
    probe: &RenderCounter,
) -> u32 {
    frames.pump(runtime, 6);
    assert!(
        driver.count() > 0,
        "the animating surface is what keeps the window drawing frames at all"
    );
    assert_eq!(
        probe.count(),
        1,
        "the probe renders once, to fill the texture every later frame composites"
    );
    probe.count()
}

fn move_pointer(runtime: &mut HeadlessRuntime, x: f64, y: f64) {
    runtime.push_input_event(InputEvent::PointerMove {
        id: POINTER_ID,
        kind: PointerKind::Mouse,
        x: crate::num_cast::f64_as_f32(x),
        y: crate::num_cast::f64_as_f32(y),
    });
}

#[test]
#[expect(
    clippy::similar_names,
    reason = "the names follow the fixture domain vocabulary; renaming would obscure rather than clarify"
)]
fn an_idle_surface_reuses_its_texture_while_the_window_keeps_drawing() {
    let driver = RenderCounter::animating();
    let probe = RenderCounter::default();
    let width = Binding::f32(PROBE_WIDTH);
    let mut runtime = runtime_with(&driver, &probe, &width);
    let mut frames = Frames::new();
    let rendered = settled(&mut runtime, &mut frames, &driver, &probe);

    let driven = driver.count();
    frames.pump(&mut runtime, 8);

    assert_eq!(
        driver.count(),
        driven + 8,
        "the animating surface renders on every one of the eight frames it asked for"
    );
    assert_eq!(
        probe.count(),
        rendered,
        "an idle surface composites its retained texture instead of re-rendering \
         whenever something else in the window animates"
    );
}

#[test]
fn a_pointer_moving_over_an_idle_surface_re_renders_it() {
    let driver = RenderCounter::animating();
    let probe = RenderCounter::default();
    let width = Binding::f32(PROBE_WIDTH);
    let handler_probe = probe.clone();
    let mut runtime = runtime_with_probe_view(
        &driver,
        GpuContentView::new(CountingView(probe.clone())).on_input(move |event| {
            if let SurfaceInputEvent::PointerMove { .. } = event {
                handler_probe.request_redraw();
            }
        }),
        &width,
    );
    let mut frames = Frames::new();
    let rendered = settled(&mut runtime, &mut frames, &driver, &probe);

    // Pointer input reaches the surface through `on_input`; the surface
    // re-renders because the handler asks for it — the move alone renders
    // nothing.
    move_pointer(&mut runtime, PROBE_ORIGIN_X + 30.0, PROBE_ORIGIN_Y + 20.0);
    frames.pump(&mut runtime, 1);
    assert_eq!(
        probe.count(),
        rendered + 1,
        "a pointer move the handler acts on re-renders the surface once"
    );

    frames.pump(&mut runtime, 3);
    assert_eq!(
        probe.count(),
        rendered + 1,
        "a pointer that then holds still changes nothing"
    );

    // The pointer leaving is itself an event the handler sees over this
    // surface's last-known position.
    move_pointer(&mut runtime, PROBE_ORIGIN_X + 40.0, PROBE_ORIGIN_Y + 10.0);
    frames.pump(&mut runtime, 1);
    assert_eq!(
        probe.count(),
        rendered + 2,
        "the pointer moving elsewhere over the surface reaches its handler"
    );

    // Moves outside the surface's hitbox never reach its handler.
    move_pointer(&mut runtime, 40.0, 30.0);
    frames.pump(&mut runtime, 3);
    assert_eq!(
        probe.count(),
        rendered + 2,
        "a pointer moving around outside the surface never reaches it"
    );
}

#[test]
fn resizing_an_idle_surface_re_renders_it_at_the_new_size() {
    let driver = RenderCounter::animating();
    let probe = RenderCounter::default();
    let width = Binding::f32(PROBE_WIDTH);
    let mut runtime = runtime_with(&driver, &probe, &width);
    let mut frames = Frames::new();
    let rendered = settled(&mut runtime, &mut frames, &driver, &probe);

    frames.pump(&mut runtime, 3);
    assert_eq!(probe.count(), rendered);

    width.set(PROBE_WIDTH * 2.0);
    frames.pump(&mut runtime, 2);
    assert_eq!(
        probe.count(),
        rendered + 1,
        "a resized surface renders into the texture the resize recreated"
    );

    frames.pump(&mut runtime, 3);
    assert_eq!(
        probe.count(),
        rendered + 1,
        "and goes idle again at the new size"
    );
}

#[test]
fn a_surface_asking_for_redraws_renders_every_frame_and_then_settles() {
    let driver = RenderCounter::animating();
    let probe = RenderCounter::default();
    let width = Binding::f32(PROBE_WIDTH);
    let mut runtime = runtime_with(&driver, &probe, &width);
    let mut frames = Frames::new();
    let rendered = settled(&mut runtime, &mut frames, &driver, &probe);

    // The flag only takes effect once the view runs; a redraw request
    // through the setup-time handle buys that one frame — the same kick a
    // decoder pushing a new frame off-thread uses.
    probe.set_animating(true);
    probe.request_redraw();
    frames.pump(&mut runtime, 1);
    assert_eq!(probe.count(), rendered + 1);

    frames.pump(&mut runtime, 4);
    assert_eq!(
        probe.count(),
        rendered + 5,
        "a view that asks for another frame from `render` gets one, every frame"
    );

    probe.set_animating(false);
    frames.pump(&mut runtime, 4);
    assert_eq!(
        probe.count(),
        rendered + 6,
        "the last request outstanding is served, and then it goes quiet again"
    );
}
