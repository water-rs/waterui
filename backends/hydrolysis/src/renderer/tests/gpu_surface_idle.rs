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

use core::cell::{Cell, RefCell};
use core::time::Duration;
use std::rc::Rc;
use std::time::Instant;

use waterui::{Binding, ViewExt as _};
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::{GpuContext, GpuFrame, GpuSurface, GpuView};
use waterui_layout::frame::Frame;
use waterui_layout::stack::vstack;

use super::pumped_test_environment;
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
#[derive(Clone, Default)]
struct RenderCounter {
    renders: Rc<Cell<u32>>,
    animating: Rc<Cell<bool>>,
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
        self.renders.get()
    }

    fn set_animating(&self, animating: bool) {
        self.animating.set(animating);
    }
}

struct CountingView(RenderCounter);

impl GpuView for CountingView {
    async fn setup(&mut self, _ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {}

    fn render(&mut self, frame: &mut GpuFrame) {
        self.0.renders.set(self.0.renders.get() + 1);
        if self.0.animating.get() {
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
    let views = RefCell::new(Some((driver.clone(), probe.clone(), probe_width.clone())));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let (driver, probe, probe_width) = views
            .borrow_mut()
            .take()
            .expect("the probe window is built once");
        AnyView::new(vstack((
            GpuSurface::new(CountingView(driver)).size(WINDOW_WIDTH as f32, DRIVER_HEIGHT),
            Frame::new(GpuSurface::new(CountingView(probe)))
                .width(probe_width)
                .height(PROBE_HEIGHT),
        )))
    });
    HeadlessRuntime::new_for_tests(
        pumped_test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
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
        x: x as f32,
        y: y as f32,
    });
}

#[test]
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
    let mut runtime = runtime_with(&driver, &probe, &width);
    let mut frames = Frames::new();
    let rendered = settled(&mut runtime, &mut frames, &driver, &probe);

    // Pointer-reactive views sample `GpuFrame::pointer` per frame and never
    // request a redraw for it, so the pointer moving is the renderer's business
    // to notice.
    move_pointer(&mut runtime, PROBE_ORIGIN_X + 30.0, PROBE_ORIGIN_Y + 20.0);
    frames.pump(&mut runtime, 1);
    assert_eq!(
        probe.count(),
        rendered + 1,
        "a pointer arriving over the surface changes what its next frame would draw"
    );

    frames.pump(&mut runtime, 3);
    assert_eq!(
        probe.count(),
        rendered + 1,
        "a pointer that then holds still changes nothing"
    );

    // Leaving the surface projects to no pointer at all, which is a change.
    move_pointer(&mut runtime, 12.0, 12.0);
    frames.pump(&mut runtime, 1);
    assert_eq!(
        probe.count(),
        rendered + 2,
        "the pointer leaving is as much a change as it arriving"
    );

    // Moving further away outside the surface projects to no pointer again.
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

    // The flag only takes effect once the view runs; the pointer arriving over
    // it buys that one frame.
    probe.set_animating(true);
    move_pointer(&mut runtime, PROBE_ORIGIN_X + 10.0, PROBE_ORIGIN_Y + 10.0);
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
