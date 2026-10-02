//! A full-window `GpuContentView` presented through the engine layer tree.
//!
//! Before the Cherenkov cutover a full-window opaque GPU surface could render
//! straight into the window's own target, skipping the compositor; whether a
//! frame took that branch or the composited one was a Hydrolysis decision with
//! its own counters. In the engine-content model there is exactly one path: a
//! `GpuContentView`'s producer installs once on a keyed engine layer, draws
//! into an engine-owned attachment, and presentation is the engine's.
//!
//! What stays pinned here:
//!
//! * a full-window surface draws at HiDPI scale too — the window root
//!   transform is `Affine::scale(scale_factor)`, so content sized in logical
//!   points must still cover the window at scale 2;
//! * a view sees one attachment format for its whole lifetime, so resizing it
//!   through the compositor does not rebuild its GPU resources;
//! * `GpuContent::is_opaque` is advisory metadata for the engine: content
//!   that declares it and content that does not present the same pixels when
//!   both write a fully opaque fill.
//!
//! Every test drives the real runner path and reads the frame report's own
//! counters, so "the layer mounted" means the render pass installed it, not
//! that the layer looked eligible.

use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use waterui_testing::TestArtifacts;

use waterui::Binding;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_graphics::gpu::{Context as GpuContext, Frame as GpuFrame};
use waterui_graphics::{GpuContent, GpuContentView};
use waterui_layout::frame::Frame;

use super::{MinimalTestTheme, pumped_test_environment};
use crate::HeadlessRuntime;

const WINDOW_WIDTH: u32 = 160;
const WINDOW_HEIGHT: u32 = 120;

/// Written into every pixel of the surface, by both declaration choices, as a
/// literal texel value. The probe clears an engine-owned attachment in
/// premultiplied linear Display P3; black is gamut-neutral, so it survives
/// the engine's presentation pass (gamut-mapped to sRGB) unchanged — a
/// saturated primary would shift — and stays distinguishable from the
/// window's light theme background.
const FILL: wgpu::Color = wgpu::Color::BLACK;

/// Where this module's visual evidence is written: `waterui-testing`'s
/// canonical `<root>/hydrolysis/direct_to_target/<stage>.png` layout, with the
/// root from `WATERUI_TEST_ARTIFACTS_DIR` when CI sets it and the platform temp
/// directory otherwise.
fn image_dir() -> std::path::PathBuf {
    TestArtifacts::new("hydrolysis").case_dir("direct_to_target")
}

/// What a probe recorded about its own lifetime: how often it was set up, how
/// often it drew, and which format it was handed each time. The producer runs
/// on the engine's render thread, so the log is shareable across threads.
#[derive(Clone, Default)]
struct ProbeLog {
    setups: Arc<AtomicU32>,
    renders: Arc<AtomicU32>,
    formats: Arc<Mutex<Vec<wgpu::TextureFormat>>>,
}

impl ProbeLog {
    fn setups(&self) -> u32 {
        self.setups.load(Ordering::Relaxed)
    }

    fn renders(&self) -> u32 {
        self.renders.load(Ordering::Relaxed)
    }

    /// Every distinct format the view was asked to render into, in order.
    fn formats(&self) -> Vec<wgpu::TextureFormat> {
        let mut formats = self.formats.lock().expect("formats log").clone();
        formats.dedup();
        formats
    }
}

/// A view that fills whatever it is handed with [`FILL`], and that answers
/// [`GpuContent::is_opaque`] as the test tells it to.
struct FillProbe {
    log: ProbeLog,
    opaque: bool,
}

impl GpuContent for FillProbe {
    fn setup(&mut self, _gpu: &GpuContext<'_>) {
        self.log.setups.fetch_add(1, Ordering::Relaxed);
    }

    fn render(&mut self, frame: &mut GpuFrame<'_>) {
        self.log.renders.fetch_add(1, Ordering::Relaxed);
        self.log
            .formats
            .lock()
            .expect("formats log")
            .push(frame.format);
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("hydrolysis_direct_to_target_probe_encoder"),
            });
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hydrolysis_direct_to_target_probe_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame.view,
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
/// therefore the `GpuContentView` inside it and everything `setup` gave it —
/// survives every change the tests make.
fn runtime_with(
    log: &ProbeLog,
    opaque: bool,
    width: &Binding<f32>,
    height: &Binding<f32>,
    scale_factor: f64,
) -> HeadlessRuntime {
    let parts = Mutex::new(Some((log.clone(), width.clone(), height.clone())));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let (log, width, height) = parts
            .lock()
            .expect("probe parts")
            .take()
            .expect("the probe window is built once");
        AnyView::new(
            Frame::new(GpuContentView::new(FillProbe { log, opaque }))
                .width(width)
                .height(height),
        )
    });
    HeadlessRuntime::new_for_tests(
        pumped_test_environment(),
        builder,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
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
fn a_full_window_gpu_content_view_mounts_as_one_engine_layer_at_every_scale() {
    for scale in [1.0_f64, 2.0] {
        let log = ProbeLog::default();
        let (width, height) = full_window_bindings();
        let mut runtime = runtime_with(&log, true, &width, &height, scale);
        let mut frames = Frames::new();
        settled(&mut runtime, &mut frames, &log);

        let counters = frames.render(&mut runtime).counters;
        assert_eq!(
            counters.gpu_content_layers, 1,
            "a surface covering the whole window mounts as one engine content \
             layer at scale {scale}"
        );
        assert_eq!(
            counters.scene_segment_layers, 0,
            "and a window holding only that layer draws no scene segments at \
             scale {scale}"
        );
        assert_eq!(
            counters.scene_layers, 1,
            "the GPU content layer is the window's only composited layer at \
             scale {scale}"
        );
    }
}

#[test]
fn a_non_opaque_full_window_surface_mounts_the_same_way_at_every_scale() {
    for scale in [1.0_f64, 2.0] {
        let log = ProbeLog::default();
        let (width, height) = full_window_bindings();
        let mut runtime = runtime_with(&log, false, &width, &height, scale);
        let mut frames = Frames::new();
        settled(&mut runtime, &mut frames, &log);

        let counters = frames.render(&mut runtime).counters;
        assert_eq!(
            counters.gpu_content_layers, 1,
            "`is_opaque` being false mounts the same single content layer — the \
             declaration is advisory, not a different presentation path (scale {scale})"
        );
    }
}

#[test]
fn resizing_a_gpu_content_view_never_re_runs_setup() {
    let log = ProbeLog::default();
    let (width, height) = full_window_bindings();
    let mut runtime = runtime_with(&log, true, &width, &height, 2.0);
    let mut frames = Frames::new();
    settled(&mut runtime, &mut frames, &log);

    assert_eq!(
        frames.render(&mut runtime).counters.gpu_content_layers,
        1,
        "the surface starts out covering the window"
    );

    // Inset the surface: the layer's bounds and pixel size change. Nothing
    // structural changed — the same node, the same runtime, the same view —
    // so only the layer's size and transform edits apply.
    width.set(WINDOW_WIDTH as f32 - 20.0);
    height.set(WINDOW_HEIGHT as f32 - 20.0);
    frames.pump(&mut runtime, 2);
    let counters = frames.render(&mut runtime).counters;
    assert_eq!(counters.gpu_content_layers, 1);
    assert_eq!(
        log.setups(),
        1,
        "a resize must not rebuild the view's GPU resources"
    );

    width.set(WINDOW_WIDTH as f32);
    height.set(WINDOW_HEIGHT as f32);
    frames.pump(&mut runtime, 2);
    assert_eq!(
        frames.render(&mut runtime).counters.gpu_content_layers,
        1,
        "and the layer persists once it covers the window again"
    );
    assert_eq!(
        log.setups(),
        1,
        "setup runs exactly once across both resizes"
    );
    assert_eq!(
        log.formats(),
        vec![wgpu::TextureFormat::Rgba16Float],
        "and the view saw one format the whole way through — the engine-owned \
         content attachment's format"
    );
}

/// Whether a window shows a declared-opaque surface or one that isn't must not
/// change the picture when the fill is itself fully opaque: the declaration is
/// advisory, so identical content presents identical pixels.
#[test]
fn opaque_and_non_opaque_content_present_identically() {
    let scale = 2.0;

    let direct_log = ProbeLog::default();
    let (direct_width, direct_height) = full_window_bindings();
    let mut direct_runtime = runtime_with(&direct_log, true, &direct_width, &direct_height, scale);
    let mut direct_frames = Frames::new();
    settled(&mut direct_runtime, &mut direct_frames, &direct_log);
    let direct = direct_frames.render(&mut direct_runtime);
    assert_eq!(direct.counters.gpu_content_layers, 1);
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
    assert_eq!(composed.counters.gpu_content_layers, 1);
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
        "declaring `is_opaque` does not change what identical content presents"
    );

    // Agreeing is not enough on its own — both renders agreeing on the wrong
    // thing would pass that. What the surface drew has to actually be there,
    // over the whole window, with none of the window's base colour left
    // anywhere.
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

/// Whether a captured `Rgba8Unorm` texel is [`FILL`], allowing the slack the
/// engine's linear-P3 presentation leaves on the exact gamut corner.
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
        .all(|(actual, expected)| actual.abs_diff(expected) <= 8)
}
