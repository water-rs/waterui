//! Rendering a full-window GPU surface straight into the window's own target.
//!
//! Three things are pinned here, and each of them was broken:
//!
//! * the path is taken on a HiDPI window, not only at scale 1 — the window root
//!   transform is `Affine::scale(scale_factor)`, so a test for the identity
//!   transform was false on every Retina display and this path was dead code
//!   there;
//! * a view sees one texture format for its whole lifetime, so moving between
//!   this path and the composited one does not rebuild its GPU resources;
//! * only a view that declares itself opaque is handed the window's target,
//!   which arrives uncleared and still holding the previous frame.
//!
//! Every test drives the real runner path and reads the frame report's own
//! counters, so "it rendered directly" means the render pass took that branch,
//! not that the layer looked eligible.

use core::cell::Cell;
use core::cell::RefCell;
use core::time::Duration;
use std::rc::Rc;
use std::time::Instant;
use waterui_testing::TestArtifacts;

use waterui::Binding;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::{GpuContext, GpuFrame, GpuSurface, GpuView};
use waterui_layout::frame::Frame;

use super::pumped_test_environment;
use crate::HeadlessRuntime;

const WINDOW_WIDTH: u32 = 160;
const WINDOW_HEIGHT: u32 = 120;

/// Written into every pixel of the surface, by both paths, as a literal texel
/// value: an `Rgba8Unorm` clear takes the colour as given, so the two paths are
/// comparable byte for byte.
const FILL: wgpu::Color = wgpu::Color {
    r: 0.125,
    g: 0.5,
    b: 0.75,
    a: 1.0,
};

/// Where this module's visual evidence is written: `waterui-testing`'s
/// canonical `<root>/hydrolysis/direct_to_target/<stage>.png` layout, with the
/// root from `WATERUI_TEST_ARTIFACTS_DIR` when CI sets it and the platform temp
/// directory otherwise.
fn image_dir() -> std::path::PathBuf {
    TestArtifacts::new("hydrolysis").case_dir("direct_to_target")
}

/// What a probe recorded about its own lifetime: how often it was set up, how
/// often it drew, and which format it was handed each time.
#[derive(Clone, Default)]
struct ProbeLog {
    setups: Rc<Cell<u32>>,
    renders: Rc<Cell<u32>>,
    formats: Rc<RefCell<Vec<wgpu::TextureFormat>>>,
}

impl ProbeLog {
    fn setups(&self) -> u32 {
        self.setups.get()
    }

    fn renders(&self) -> u32 {
        self.renders.get()
    }

    /// Every distinct format the view was asked to render into, in order.
    fn formats(&self) -> Vec<wgpu::TextureFormat> {
        let mut formats = self.formats.borrow().clone();
        formats.dedup();
        formats
    }
}

/// A view that fills whatever it is handed with [`FILL`], and that answers
/// [`GpuView::is_opaque`] as the test tells it to.
struct FillProbe {
    log: ProbeLog,
    opaque: bool,
}

impl GpuView for FillProbe {
    async fn setup(&mut self, _ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
        self.log.setups.set(
            self.log
                .setups
                .get()
                .checked_add(1)
                .expect("setup overflow"),
        );
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        self.log.renders.set(
            self.log
                .renders
                .get()
                .checked_add(1)
                .expect("render overflow"),
        );
        self.log.formats.borrow_mut().push(frame.format);
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("hydrolysis_direct_to_target_probe_encoder"),
            });
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hydrolysis_direct_to_target_probe_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &frame.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(FILL),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }));
        frame.queue.submit([encoder.finish()]);
    }

    fn is_opaque(&self) -> bool {
        self.opaque
    }
}

/// A window whose entire content is one GPU surface, sized by the test.
///
/// The size is a `Binding` rather than a rebuild so the surface's node — and
/// therefore the `GpuView` inside it and everything `setup` gave it — survives
/// every change the tests make.
fn runtime_with(
    log: &ProbeLog,
    opaque: bool,
    width: &Binding<f32>,
    height: &Binding<f32>,
    scale_factor: f64,
) -> HeadlessRuntime {
    let parts = RefCell::new(Some((log.clone(), width.clone(), height.clone())));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let (log, width, height) = parts
            .borrow_mut()
            .take()
            .expect("the probe window is built once");
        AnyView::new(
            Frame::new(GpuSurface::new(FillProbe { log, opaque }))
                .width(width)
                .height(height),
        )
    });
    HeadlessRuntime::new_for_tests(
        pumped_test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
    )
    .with_scale_factor(scale_factor)
}

/// Pumps window frames on a fixed 16ms cadence, so the frame clock is the
/// test's rather than the wall's.
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

    fn at(&mut self) -> Instant {
        let at = self.start + Duration::from_millis(self.next * 16);
        self.next += 1;
        at
    }

    fn pump(&mut self, runtime: &mut HeadlessRuntime, count: u64) {
        for _ in 0..count {
            let at = self.at();
            let _ = runtime.pump_at(false, at);
        }
    }

    /// One frame that certainly runs the window's render pass, and reports what
    /// that pass did.
    ///
    /// A capturing pump is how a test asks for that unconditionally: a window
    /// whose content asks for nothing is entitled to sit a frame out, and a
    /// skipped frame reports the default counters — zero of everything, which
    /// reads exactly like "it was composited" and would make these assertions
    /// depend on whether the previous pump happened to leave a request behind.
    fn render(&mut self, runtime: &mut HeadlessRuntime) -> RenderedFrame {
        let at = self.at();
        let result = runtime.pump_at(true, at);
        RenderedFrame {
            counters: result.profile.counters,
            snapshot: result
                .snapshot
                .expect("a capturing pump must produce a snapshot"),
        }
    }
}

/// One frame's render pass: what it did, and what it left on screen.
struct RenderedFrame {
    counters: crate::runner::FrameCounters,
    snapshot: crate::runner::HeadlessSnapshot,
}

/// Pumps until the surface's async setup has completed and it has drawn once.
fn settled(runtime: &mut HeadlessRuntime, frames: &mut Frames, log: &ProbeLog) {
    for _ in 0..12u32 {
        if log.renders() > 0 {
            break;
        }
        frames.pump(runtime, 1);
    }
    assert_eq!(
        log.setups(),
        1,
        "the surface's view is set up exactly once for the surface's lifetime"
    );
    assert!(
        log.renders() > 0,
        "the surface must have drawn before a test measures which path drew it"
    );
}

fn full_window_bindings() -> (Binding<f32>, Binding<f32>) {
    (
        Binding::f32(WINDOW_WIDTH as f32),
        Binding::f32(WINDOW_HEIGHT as f32),
    )
}

fn write_png(name: &str, snapshot: &crate::runner::HeadlessSnapshot) -> std::path::PathBuf {
    let directory = image_dir();
    std::fs::create_dir_all(&directory).expect("the image directory must be creatable");
    let path = directory.join(name);
    let image = image::RgbaImage::from_raw(snapshot.width, snapshot.height, snapshot.rgba8.clone())
        .expect("snapshot dimensions must match the rgba buffer");
    image.save(&path).expect("snapshot png must be writable");
    path
}

#[test]
fn an_opaque_full_window_surface_renders_directly_at_every_scale() {
    for scale in [1.0_f64, 2.0] {
        let log = ProbeLog::default();
        let (width, height) = full_window_bindings();
        let mut runtime = runtime_with(&log, true, &width, &height, scale);
        let mut frames = Frames::new();
        settled(&mut runtime, &mut frames, &log);

        let counters = frames.render(&mut runtime).counters;
        assert_eq!(
            counters.direct_gpu_surfaces, 1,
            "an opaque surface covering the whole window renders straight into the \
             window target at scale {scale}, where the window root transform is \
             Affine::scale({scale})"
        );
        assert_eq!(
            counters.gpu_surface_layers, 0,
            "nothing is left for the compositor to draw at scale {scale}"
        );
        assert_eq!(
            counters.scene_layers, 0,
            "and there is no composite pass at all at scale {scale}"
        );
    }
}

#[test]
fn a_non_opaque_full_window_surface_is_composited_at_every_scale() {
    for scale in [1.0_f64, 2.0] {
        let log = ProbeLog::default();
        let (width, height) = full_window_bindings();
        let mut runtime = runtime_with(&log, false, &width, &height, scale);
        let mut frames = Frames::new();
        settled(&mut runtime, &mut frames, &log);

        let counters = frames.render(&mut runtime).counters;
        assert_eq!(
            counters.direct_gpu_surfaces, 0,
            "a view that has not declared itself opaque never gets the window's own \
             uncleared texture, however exactly it covers the window (scale {scale})"
        );
        assert_eq!(
            counters.gpu_surface_layers, 1,
            "it is composited over the window's cleared base colour instead (scale {scale})"
        );
    }
}

#[test]
fn moving_between_the_two_paths_never_re_runs_setup() {
    let log = ProbeLog::default();
    let (width, height) = full_window_bindings();
    let mut runtime = runtime_with(&log, true, &width, &height, 2.0);
    let mut frames = Frames::new();
    settled(&mut runtime, &mut frames, &log);

    assert_eq!(
        frames.render(&mut runtime).counters.direct_gpu_surfaces,
        1,
        "the surface starts out covering the window"
    );

    // Inset the surface: its transformed bounds no longer match the viewport,
    // so the window has to composite it. Nothing structural changed — the same
    // node, the same runtime, the same view.
    width.set(WINDOW_WIDTH as f32 - 20.0);
    height.set(WINDOW_HEIGHT as f32 - 20.0);
    frames.pump(&mut runtime, 2);
    let counters = frames.render(&mut runtime).counters;
    assert_eq!(
        counters.direct_gpu_surfaces, 0,
        "an inset surface does not cover the viewport and is composited"
    );
    assert_eq!(counters.gpu_surface_layers, 1);
    assert_eq!(
        log.setups(),
        1,
        "moving onto the composited path must not rebuild the view's GPU resources"
    );

    width.set(WINDOW_WIDTH as f32);
    height.set(WINDOW_HEIGHT as f32);
    frames.pump(&mut runtime, 2);
    assert_eq!(
        frames.render(&mut runtime).counters.direct_gpu_surfaces,
        1,
        "and it goes back to rendering directly once it covers the window again"
    );
    assert_eq!(
        log.setups(),
        1,
        "setup runs exactly once across both switches"
    );
    assert_eq!(
        log.formats(),
        vec![wgpu::TextureFormat::Rgba8Unorm],
        "and the view saw one format the whole way through — the target's own \
         linear format, on both paths"
    );
}

/// The two paths must produce the same window. This is what the opacity
/// contract buys: the direct path skips the clear to the window's base colour,
/// which is only sound because the view fills every pixel — and a missing clear
/// would show up here as a difference against the composited render of the very
/// same scene.
#[test]
fn the_direct_and_composited_paths_agree_pixel_for_pixel() {
    let scale = 2.0;

    let direct_log = ProbeLog::default();
    let (direct_width, direct_height) = full_window_bindings();
    let mut direct_runtime = runtime_with(&direct_log, true, &direct_width, &direct_height, scale);
    let mut direct_frames = Frames::new();
    settled(&mut direct_runtime, &mut direct_frames, &direct_log);
    let direct = direct_frames.render(&mut direct_runtime);
    assert_eq!(direct.counters.direct_gpu_surfaces, 1);
    let direct = direct.snapshot;

    let composed_log = ProbeLog::default();
    let (composed_width, composed_height) = full_window_bindings();
    let mut composed_runtime = runtime_with(
        &composed_log,
        false,
        &composed_width,
        &composed_height,
        scale,
    );
    let mut composed_frames = Frames::new();
    settled(&mut composed_runtime, &mut composed_frames, &composed_log);
    let composed = composed_frames.render(&mut composed_runtime);
    assert_eq!(composed.counters.direct_gpu_surfaces, 0);
    let composed = composed.snapshot;

    let direct_path = write_png("direct_scale2.png", &direct);
    let composed_path = write_png("composed_scale2.png", &composed);
    eprintln!(
        "wrote {} and {}",
        direct_path.display(),
        composed_path.display()
    );

    assert_eq!(
        (direct.width, direct.height),
        (WINDOW_WIDTH * 2, WINDOW_HEIGHT * 2),
        "a 2x window captures at twice its logical size"
    );
    assert_eq!(
        (composed.width, composed.height),
        (direct.width, direct.height)
    );
    assert_eq!(
        direct.rgba8, composed.rgba8,
        "the direct path draws the same window as the composite it replaces"
    );

    // Agreeing is not enough on its own — both paths agreeing on the wrong
    // thing would pass that. What the surface drew has to actually be there,
    // over the whole window, with none of the window's base colour left
    // anywhere: the direct path skips the clear that would have painted it.
    let (pixels, _) = direct.rgba8.as_chunks::<4>();
    let expected = *pixels.first().expect("the capture must have pixels");
    assert!(
        near_fill(expected),
        "the captured colour {expected:?} must be the fill the probe cleared to"
    );
    assert!(
        pixels.iter().all(|pixel| *pixel == expected),
        "every pixel of the window belongs to the surface, at both ends of the comparison"
    );
}

/// Whether a captured `Rgba8Unorm` texel is [`FILL`], allowing the one step of
/// slack that rounding a float clear colour onto 8 bits leaves.
fn near_fill(pixel: [u8; 4]) -> bool {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "each component is a clear colour in 0..=1 scaled onto 0..=255"
    )]
    let expected = [FILL.r, FILL.g, FILL.b, FILL.a].map(|component| (component * 255.0) as u8);
    pixel
        .iter()
        .zip(expected)
        .all(|(actual, expected)| actual.abs_diff(expected) <= 1)
}
