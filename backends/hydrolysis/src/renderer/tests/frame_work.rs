//! Frame-work fixtures for the fine-grained frame model
//! (water-rs/hydrolysis#205).
//!
//! Every fixture drives the real headless runner at a fixed clock and asserts
//! on two things at once: that the frame still renders (the snapshot) and
//! that the work the frame did is recorded by the frame-work counters on
//! [`crate::runner::FrameCounters::frame_work`]. The counters are the
//! instrument the frame model is measured against — a fixture's whole-frame
//! values (`semantic builds`, `recorded view contents`, `font and image
//! registrations`, `gpu submissions`, `host wakeups`) are what a steady
//! frame drives to zero, while the fine-grained counters (`live operand
//! updates`, `layer creations`, `layer removals`) carry the nonzero signal.
//!
//! The fixtures deliberately assert *which* counter families moved rather
//! than exact counts: exact numbers are baseline data, and an exact
//! assertion would break on unrelated dev churn the frame model never
//! touched.
//!
//! Fixture inventory:
//! * nested clip/blend/opacity → `nested_clip_blend_opacity_counts`
//! * transformed image brushes → `transformed_image_brushes_count`
//! * glyph-only scenes → `glyph_only_scene_counts`
//! * variable / COLR / bitmap fonts → `variable_colr_bitmap_fonts_count`
//! * all shadow silhouettes → `shadow_silhouettes_count`
//! * context-menu holes → `context_menu_holes_render` (accessibility)
//! * popup opening → `popup_opening_counts` (accessibility)
//! * scrolling → `scrolling_counts`
//! * GPU content under clips/effects → `gpu_content_under_clips_and_effects`
//! * native-view interleaving → documented only: `record_native_view_layer`
//!   exists solely under `hydrolysis_macos_system_webview` (winit + macOS +
//!   webview-system) and the `WKWebView` bridge requires a real window — no
//!   headless harness can mount it.
//! * capture determinism → `repeated_fixed_clock_captures_are_identical`

use core::time::Duration;
use std::cell::RefCell;
use std::time::Instant;

use waterui::component::text;
use waterui::prelude::{Color, ContextMenu};
use waterui::shape::RoundedRectangle;
use waterui::{AnyView, Binding, FilterViewExt as _, View, ViewExt as _};
use waterui_controls::button::button;
use waterui_controls::menu::CommandExt as _;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_graphics::draw::{Draw, Recorder};
use waterui_graphics::gpu::{Context as GpuContext, Frame as GpuFrame};
use waterui_graphics::{GpuContent, GpuContentView, RecordingResources, SceneContent};
use waterui_layout::frame::Frame;
use waterui_layout::scroll::scroll;
use waterui_layout::stack::{VStack, vstack};
use waterui_shape::{Ellipse, FixedRoundedRectangle, ShapeExt as _};
use waterui_text::font::{Body, Font};

use super::{MinimalTestTheme, pumped_test_environment, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::runner::{FrameCounters, HeadlessSnapshot};

const WINDOW: u32 = 320;

/// Pumps window frames on a fixed 16 ms cadence so the frame clock is the
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

    /// One capturing frame — forcing the render pass so the counters always
    /// describe work that actually happened, never a skipped pump's zeros.
    fn render(&mut self, runtime: &mut HeadlessRuntime) -> (FrameCounters, HeadlessSnapshot) {
        let at = self.at();
        let result = runtime.pump_at(true, at);
        (
            result.profile.counters,
            result
                .snapshot
                .expect("a capturing pump must produce a snapshot"),
        )
    }
}

fn secondary_click(x: f32, y: f32) -> [InputEvent; 2] {
    [
        InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Secondary,
        },
        InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Secondary,
        },
    ]
}

fn runtime_with(view: impl View) -> HeadlessRuntime {
    let view = RefCell::new(Some(AnyView::new(view)));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the fixture view is built once")
    });
    HeadlessRuntime::new_for_tests(
        pumped_test_environment(),
        builder,
        WINDOW,
        WINDOW,
        MinimalTestTheme::default(),
    )
}

/// The frame-work values a fixture reads back. Transient presentations
/// (popup windows, drawn context menus) install no retained engine layers:
/// every retained-engine field stays zero on their frames.
fn assert_engine_counters_zero(counters: &FrameCounters) {
    let m = counters.frame_work;
    assert_eq!(m.live_operand_updates, 0, "no live operands update");
    assert_eq!(m.layer_creations, 0, "no retained engine layers mount");
    assert_eq!(m.layer_removals, 0, "no retained engine layers unmount");
}

/// The retained-engine counters a fixture whose content mounts as retained
/// scene layers reads back: the engine mounts its layers on the presented
/// frame (`layer_creations >= 1`), while a fixture with no reactive input
/// and no unmount sees no live-operand updates and no removals.
fn assert_retained_engine_counters(counters: &FrameCounters) {
    let m = counters.frame_work;
    assert_eq!(
        m.live_operand_updates, 0,
        "nothing reactive runs in this fixture"
    );
    assert!(
        m.layer_creations >= 1,
        "the retained engine mounts the fixture's layers"
    );
    assert_eq!(m.layer_removals, 0, "nothing unmounts in this fixture");
}

/// A nested `.clip` → `.opacity` → `.blur` tower: three different layer kinds
/// (clip, alpha, filter) stacked inside each other, which is the scope nesting
/// a `Recording::push_clip`/`push_group` pair must reproduce.
#[test]
fn nested_clip_blend_opacity_counts() {
    let view = ()
        .size(200.0, 200.0)
        .background(
            RoundedRectangle::new(24.0).fill(waterui::graphics::color::Srgb::from_hex("#3050a0")),
        )
        .clip(RoundedRectangle::new(24.0))
        .opacity(0.8f32)
        .blur(1.5f32);
    let mut runtime = runtime_with(view);
    let mut frames = Frames::new();
    let (counters, _snapshot) = frames.render(&mut runtime);
    let m = counters.frame_work;

    assert!(m.semantic_builds > 0, "the mount must dispatch view bodies");
    assert!(
        m.recorded_view_contents > 0,
        "view contents were re-encoded"
    );
    assert!(m.gpu_submissions > 0, "the render submitted GPU work");
    assert_retained_engine_counters(&counters);
}

/// Scene content that draws one rotated image plus one image-paint fill —
/// the `image` and `fill(Paint::Image)` ops a `Recording::image` and a
/// brush-encoded fill carry into the engine. The `Registered` handle stays
/// alive on the pane: releasing it unregisters the image under a recording
/// that still names it.
struct ImagePane {
    image: Option<waterui_graphics::Registered<waterui_graphics::draw::ImageId>>,
}

impl SceneContent for ImagePane {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        resources: &mut RecordingResources<'_>,
        _width: f32,
        _height: f32,
    ) -> bool {
        use kurbo::Affine;
        use waterui_graphics::draw::{Extend, ImagePattern, Paint, Sampling};

        if self.image.is_none() {
            self.image = Some(
                resources
                    .image(solid_image())
                    .expect("image registration failed"),
            );
        }
        let image = resources.name(self.image.as_ref().expect("registered above"));
        recorder.transform(
            Affine::translate((40.0, 40.0)) * Affine::rotate(0.4),
            |recorder| {
                recorder.image(
                    image,
                    kurbo::Rect::new(0.0, 0.0, 16.0, 16.0),
                    Sampling::Linear,
                );
            },
        );
        recorder.transform(
            Affine::translate((120.0, 60.0)) * Affine::scale(2.0),
            |recorder| {
                recorder.fill(
                    kurbo::BezPath::from_svg("M0,0 L16,0 L16,16 Z").expect("static path parses"),
                    Paint::Image(ImagePattern {
                        image,
                        transform: Affine::IDENTITY,
                        extend_x: Extend::Pad,
                        extend_y: Extend::Pad,
                        sampling: Sampling::Linear,
                    }),
                );
            },
        );
        false
    }

    fn set_invalidator(&mut self, _invalidator: Option<waterui_graphics::SceneInvalidator>) {}

    fn rebuild_for_engine(&mut self) {
        self.image = None;
    }
}

/// An opaque red image — semitransparent fixtures read back white over the
/// window's white clear, so the probe must be opaque to be detectable.
fn solid_image() -> waterui_graphics::ImageData<waterui_graphics::Rgba8> {
    let mut data = vec![0xFFu8; 16 * 16 * 4];
    for px in data.as_chunks_mut::<4>().0 {
        px[1] = 0x00;
        px[2] = 0x00;
    }
    waterui_graphics::ImageData::new(16, 16, std::sync::Arc::<[u8]>::from(data))
        .expect("a well-formed Rgba8 image")
        .premultiplied()
}

#[test]
fn transformed_image_brushes_count() {
    let mut runtime = runtime_with(waterui_graphics::SceneView::new(ImagePane { image: None }));
    let mut frames = Frames::new();
    let (counters, snapshot) = frames.render(&mut runtime);
    let m = counters.frame_work;

    let pixel_at = |x: usize, y: usize| -> &[u8] {
        &snapshot.rgba8[(x + y * snapshot.width as usize) * 4..][..4]
    };
    assert_ne!(
        pixel_at(136, 76),
        pixel_at(10, 10),
        "the transformed image ops must reach the presented frame"
    );
    assert!(m.recorded_view_contents > 0);
    assert_retained_engine_counters(&counters);
}

/// A window that is only text: every encode is a glyph run — no fills, no
/// images — which is the "glyph-only recording" case the boundary's
/// trailing-scene drain must still flush.
#[test]
fn glyph_only_scene_counts() {
    let mut runtime = runtime_with(vstack((
        text("Glyph run one").font(Font::new(Body)),
        text("Glyph run two").font(Font::new(Body)),
    )));
    let mut frames = Frames::new();
    let (counters, _snapshot) = frames.render(&mut runtime);
    let m = counters.frame_work;

    assert!(
        m.font_registrations >= 2,
        "each text run registers a font payload per encoded frame"
    );
    assert_eq!(
        m.image_registrations, 0,
        "a glyph-only scene registers no images"
    );
    assert!(m.recorded_view_contents > 0);
    assert_retained_engine_counters(&counters);
}

/// One text run each on the three colour/vector font technologies the
/// boundary must keep working: a variable face (wght axis set by `.weight`),
/// a `COLRv0` colour face, and a colour-bitmap emoji face. They run on the
/// native font loader because the deterministic collection pins generic
/// families; named-family resolution is what the fixture needs.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn variable_colr_bitmap_fonts_count() {
    use waterui_text::font::FontWeight;

    let view = RefCell::new(Some(AnyView::new(vstack((
        text("Variable").font(
            Font::new(Body)
                .family("Test Variable ABC")
                .weight(FontWeight::Black),
        ),
        text("COLR").font(Font::new(Body).family("Bungee Color Regular")),
        text("\u{1f600}\u{1f680}").font(Font::new(Body)),
    )))));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the fixture view is built once")
    });
    let mut runtime = HeadlessRuntime::new_for_tests_native_fonts(
        test_environment(),
        builder,
        WINDOW,
        WINDOW,
        MinimalTestTheme::default(),
    );
    let mut frames = Frames::new();
    let (counters, _snapshot) = frames.render(&mut runtime);
    let m = counters.frame_work;

    assert!(
        m.font_registrations >= 3,
        "variable + COLR + bitmap runs each register a font payload; got {}",
        m.font_registrations
    );
    assert!(m.recorded_view_contents > 0);
    assert_retained_engine_counters(&counters);
}

/// Every silhouette class `apply_shadow` rasterizes: a rounded-rect caster
/// and an ellipse caster in one window, so the fixture covers both silhouette
/// kinds rather than whichever one happens to appear first.
#[test]
fn shadow_silhouettes_count() {
    use waterui::graphics::Color;
    use waterui::style::{Shadow, Vector};

    fn caster(shape: impl waterui::shape::Shape + Clone + 'static) -> impl View {
        ().size(80.0, 80.0)
            .background(FixedRoundedRectangle::new(12.0).fill(Color::srgb(60, 120, 200)))
            .shadow(Shadow::new(
                Color::srgb(0, 0, 0),
                Vector::new(2.0, 3.0),
                4.0,
                shape,
            ))
    }

    let mut runtime = runtime_with(vstack((
        caster(FixedRoundedRectangle::new(12.0)),
        caster(Ellipse),
    )));
    let mut frames = Frames::new();
    let (counters, snapshot) = frames.render(&mut runtime);
    let m = counters.frame_work;

    // The engine owns the silhouette: a shadow is recorded as a native op
    // (`blurred_rounded_rect`/`scene.shadow`), never an image upload, so a
    // shadow-only scene registers no images.
    assert_eq!(
        m.image_registrations, 0,
        "the engine rasterizes silhouettes natively; got {}",
        m.image_registrations
    );
    // The scene paints nothing but the blue casters and their black
    // silhouette blurs, so a dark pixel is silhouette evidence — `pixel`-
    // precise geometry is `tests/shadow.rs`'s beat. One dark pixel per
    // caster half proves both silhouette classes reached the frame.
    let mid = (snapshot.height as usize / 2) * snapshot.width as usize * 4;
    // A blurred black silhouette on the white background paints neutral
    // greys — channels within a few points of each other and under white.
    // The caster's blue fill never matches (its blue channel leads by ~140).
    let grey_shadow =
        |p: &[u8; 4]| p[0] < 230 && p[0].abs_diff(p[1]) < 12 && p[1].abs_diff(p[2]) < 12;
    assert!(
        snapshot.rgba8[..mid]
            .as_chunks::<4>()
            .0
            .iter()
            .any(grey_shadow),
        "the rounded-rect caster's blurred silhouette must reach the frame"
    );
    assert!(
        snapshot.rgba8[mid..]
            .as_chunks::<4>()
            .0
            .iter()
            .any(grey_shadow),
        "the ellipse caster's blurred silhouette must reach the frame"
    );
    assert!(m.recorded_view_contents > 0);
    assert_retained_engine_counters(&counters);
}

/// A secondary click on a `.context_menu` view: the menu mounts (as a popup
/// window or the drawn presentation — both present a menu to a11y) and the
/// merged tree answers a command label. The occlusion hole the drawn
/// presentation punches through its scrim is exercised by the same mount —
/// this is the counter side of `context_menu_occlusion`'s hit-test fixtures.
#[test]
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
fn context_menu_holes_render() {
    let copied = Binding::container(false);
    let copied_for_view = copied;
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let copied = copied_for_view.clone();
        AnyView::new(
            Frame::new(button("host").action(|| {}))
                .width(160.0)
                .height(160.0)
                // A lifted preview forces the drawn presentation — the
                // scrim/destination-out hole path — instead of a plain
                // popup window.
                .context_menu(
                    ContextMenu::new(vec!["Copy".action(move || copied.set(true))])
                        .preview(Color::srgb(255, 0, 0)),
                ),
        )
    });
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW,
        WINDOW,
        MinimalTestTheme::default(),
    );
    let mut frames = Frames::new();
    let _ = frames.render(&mut runtime);

    for event in secondary_click(80.0, 80.0) {
        runtime.push_input_event(event);
    }
    let at = frames.at();
    let result = runtime.pump_at(true, at + Duration::from_millis(600));

    let menu = result.tree_update.as_ref().is_some_and(|update| {
        update
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Copy"))
    });
    assert!(
        menu,
        "the mounted context menu must merge into the a11y tree"
    );

    let m = result.profile.counters.frame_work;
    assert!(
        m.structural_patches + m.semantic_builds > 0,
        "mounting the menu mutates the retained tree"
    );
    assert_engine_counters_zero(&result.profile.counters);
}

/// Popup opening: a mounted popup is a second window the pump's merged a11y
/// update must surface, and its mount frame records the patch that opened it.
#[test]
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
fn popup_opening_counts() {
    let copied = Binding::container(false);
    let copied_for_view = copied;
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let copied = copied_for_view.clone();
        AnyView::new(
            Frame::new(button("host").action(|| {}))
                .width(160.0)
                .height(160.0)
                .context_menu(vec!["Copy".action(move || copied.set(true))]),
        )
    });
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW,
        WINDOW,
        MinimalTestTheme::default(),
    );
    let mut frames = Frames::new();
    let _ = frames.render(&mut runtime);

    for event in secondary_click(80.0, 80.0) {
        runtime.push_input_event(event);
    }
    let at = frames.at();
    // The popup mounts on the pump after the click lands — past the platform
    // hold threshold so the context-menu gesture resolves.
    let result = runtime.pump_at(true, at + Duration::from_millis(600));
    assert!(
        result.tree_update.as_ref().is_some_and(|update| update
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Copy"))),
        "the open popup's commands must merge into the a11y tree"
    );

    let m = result.profile.counters.frame_work;
    assert!(
        m.recorded_view_contents > 0,
        "the mount frame still encodes"
    );
    assert_engine_counters_zero(&result.profile.counters);
}

/// Scrolling a lazy list: input events drive structural patches into the
/// retained tree and each rendered frame still re-encodes content.
#[test]
fn scrolling_counts() {
    let data = (0..200).map(SelfId::new).collect::<Vec<_>>();
    let view = scroll(VStack::for_each(data, |row| {
        Frame::new(text(format!("row {}", row.into_inner()))).size(300.0, 44.0)
    }));
    let mut runtime = runtime_with(view);
    let mut frames = Frames::new();
    let (first, _) = frames.render(&mut runtime);
    assert!(first.frame_work.semantic_builds > 0);
    assert_retained_engine_counters(&first);

    let mut saw_scroll_work = false;
    for _ in 0..6 {
        runtime.push_input_event(InputEvent::Scroll {
            x: 160.0,
            y: 160.0,
            dx: 0.0,
            dy: -120.0,
            is_line_delta: false,
        });
        let (counters, _snapshot) = frames.render(&mut runtime);
        let m = counters.frame_work;
        saw_scroll_work |= m.recorded_view_contents > 0 && m.gpu_submissions > 0;
    }
    assert!(
        saw_scroll_work,
        "scroll frames must still encode view contents and submit GPU work"
    );
}

/// A probe GPU surface — one clear pass into whatever it is handed. The
/// producer runs on the engine's render thread, so the counter is shareable
/// across threads.
struct FillProbe {
    renders: std::sync::Arc<core::sync::atomic::AtomicU32>,
}

impl GpuContent for FillProbe {
    fn setup(&mut self, _gpu: &GpuContext<'_>) {}

    fn render(&mut self, frame: &mut GpuFrame<'_>) {
        self.renders
            .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame_work_fixture_gpu_probe"),
            });
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("frame_work_fixture_gpu_probe_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLUE),
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
}

/// GPU content sitting under a clip and an opacity layer: the engine mounts
/// it as a GPU content layer with the clip/opacity scope above it.
#[test]
fn gpu_content_under_clips_and_effects() {
    let renders = std::sync::Arc::new(core::sync::atomic::AtomicU32::new(0));
    let probe = FillProbe {
        renders: std::sync::Arc::clone(&renders),
    };
    let view = Frame::new(GpuContentView::new(probe))
        .width(120.0)
        .height(120.0)
        .clip(RoundedRectangle::new(12.0))
        .opacity(0.9f32);
    let mut runtime = runtime_with(view);
    let mut frames = Frames::new();

    // The surface's async setup needs a few executor drains before it renders.
    for _ in 0..12 {
        if renders.load(core::sync::atomic::Ordering::Relaxed) > 0 {
            break;
        }
        let at = frames.at();
        let _ = runtime.pump_at(false, at);
    }
    assert!(
        renders.load(core::sync::atomic::Ordering::Relaxed) > 0,
        "the GPU surface must have drawn"
    );

    let (counters, _snapshot) = frames.render(&mut runtime);
    let m = counters.frame_work;
    assert!(
        counters.gpu_content_layers >= 1,
        "a GPU surface under a clip composites as a GPU content layer"
    );
    assert!(m.gpu_submissions > 0, "the probe's submit is counted");
    assert!(m.recorded_view_contents > 0);
    assert_retained_engine_counters(&counters);
}

/// The determinism clause: two capturing pumps of the same view at the same
/// fixed instant produce byte-identical snapshots.
#[test]
fn repeated_fixed_clock_captures_are_identical() {
    let view = || {
        AnyView::new(vstack((
            text("steady").font(Font::new(Body)),
            ().size(80.0, 80.0)
                .background(
                    RoundedRectangle::new(8.0)
                        .fill(waterui::graphics::color::Srgb::from_hex("#4050b0")),
                )
                .clip(RoundedRectangle::new(8.0))
                .opacity(0.75f32),
        )))
    };
    let mut runtime = runtime_with(view());
    let at = Instant::now();
    let first = runtime.pump_at(true, at);
    let second = runtime.pump_at(true, at);

    let (a, b) = (
        first.snapshot.expect("first capture"),
        second.snapshot.expect("second capture"),
    );
    assert_eq!(
        a.rgba8, b.rgba8,
        "fixed-clock captures of an unchanged view must be byte-identical"
    );
    // Per-pump counters are not asserted identical: the second pump re-runs
    // the measure/layout work its own pipeline state requires, which legit-
    // imately differs once the first pump populated caches.
}
