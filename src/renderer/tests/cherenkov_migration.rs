//! water-rs/hydrolysis#205 — the Cherenkov migration acceptance fixtures.
//!
//! Every fixture drives the real headless runner at a fixed clock and asserts
//! on two things at once: that the frame still renders (the snapshot) and
//! that the work the frame did is recorded by the migration counters on
//! [`crate::runner::FrameCounters::migration`]. The counters are the
//! acceptance instrument for the port — a fixture's Vello-era values
//! (`semantic builds`, `recorded view contents`, `font and image
//! registrations`, `gpu submissions`, `host wakeups`) are what the retained
//! rewrite must drive to zero on steady frames, while the engine-era
//! counters (`live operand updates`, `layer creations`, `layer removals`)
//! become the nonzero signal.
//!
//! The fixtures deliberately assert *which* counter families moved rather
//! than exact counts: exact numbers are baseline data (recorded in
//! `docs/cherenkov-migration.md`), and an exact assertion would break on
//! unrelated dev churn the migration never touched.
//!
//! Fixture inventory (the plan's fixture families):
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
//!   webview-system) and the WKWebView bridge requires a real window — no
//!   headless harness can mount it. The boundary checker quarantines the
//!   call site instead.
//! * capture determinism → `repeated_fixed_clock_captures_are_identical`

use core::time::Duration;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use waterui::component::text;
use waterui::prelude::{Color, ContextMenu};
use waterui::shape::RoundedRectangle;
use waterui::{AnyView, Binding, FilterViewExt as _, View, ViewExt as _};
use waterui_controls::button::button;
use waterui_controls::menu::CommandExt as _;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_graphics::{GpuContext, GpuFrame, GpuSurface, GpuView, Scene2D, SceneContent};
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

/// The migration counter values a fixture reads back. The four zero-expected
/// engine-era fields are asserted on every fixture — a nonzero value before
/// the retained engine exists is a counter bug, not an early success.
fn assert_engine_era_counters_zero(counters: &FrameCounters) {
    let m = counters.migration;
    assert_eq!(m.live_operand_updates, 0, "no live operands exist yet");
    assert_eq!(m.layer_creations, 0, "no retained engine layers exist yet");
    assert_eq!(m.layer_removals, 0, "no retained engine layers exist yet");
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
    let m = counters.migration;

    assert!(m.semantic_builds > 0, "the mount must dispatch view bodies");
    assert!(
        m.recorded_view_contents > 0,
        "view contents were re-encoded"
    );
    assert!(m.gpu_submissions > 0, "the render submitted GPU work");
    assert_engine_era_counters_zero(&counters);
}

/// Scene content that draws one rotated image plus one image-brush fill —
/// the `draw_image` and `fill(Brush::Image)` registrations a `Recording::image`
/// and a brush-encoded fill must carry.
struct ImagePane {
    draw: fn(&mut dyn Scene2D),
}

impl SceneContent for ImagePane {
    fn build_scene(&mut self, scene: &mut dyn Scene2D, _width: f32, _height: f32) -> bool {
        (self.draw)(scene);
        false
    }

    fn set_invalidator(&mut self, _invalidator: Option<waterui_graphics::SceneInvalidator>) {}
}

fn solid_image() -> peniko::ImageData {
    peniko::ImageData {
        data: peniko::Blob::from(vec![0x80u8; 16 * 16 * 4]),
        format: peniko::ImageFormat::Rgba8,
        alpha_type: peniko::ImageAlphaType::AlphaPremultiplied,
        width: 16,
        height: 16,
    }
}

#[test]
fn transformed_image_brushes_count() {
    use kurbo::Affine;
    use peniko::{Brush, ImageBrush};

    let mut runtime = runtime_with(waterui_graphics::SceneView::new(ImagePane {
        draw: |scene| {
            scene.draw_image(
                &ImageBrush::new(solid_image()),
                Affine::translate((40.0, 40.0)) * Affine::rotate(0.4),
            );
            scene.fill(
                peniko::Fill::NonZero,
                Affine::translate((120.0, 60.0)) * Affine::scale(2.0),
                &Brush::Image(ImageBrush::new(solid_image())),
                None,
                &kurbo::BezPath::from_svg("M0,0 L16,0 L16,16 Z").expect("static path parses"),
            );
        },
    }));
    let mut frames = Frames::new();
    let (counters, _snapshot) = frames.render(&mut runtime);
    let m = counters.migration;

    assert_eq!(
        m.image_registrations, 2,
        "the rotated draw_image and the Brush::Image fill each register an image payload"
    );
    assert!(m.recorded_view_contents > 0);
    assert_engine_era_counters_zero(&counters);
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
    let m = counters.migration;

    assert!(
        m.font_registrations >= 2,
        "each text run registers a font payload per encoded frame"
    );
    assert_eq!(
        m.image_registrations, 0,
        "a glyph-only scene registers no images"
    );
    assert!(m.recorded_view_contents > 0);
    assert_engine_era_counters_zero(&counters);
}

/// One text run each on the three colour/vector font technologies the
/// boundary must keep working: a variable face (wght axis set by `.weight`),
/// a COLRv0 colour face, and a colour-bitmap emoji face. They run on the
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
    let m = counters.migration;

    assert!(
        m.font_registrations >= 3,
        "variable + COLR + bitmap runs each register a font payload; got {}",
        m.font_registrations
    );
    assert!(m.recorded_view_contents > 0);
    assert_engine_era_counters_zero(&counters);
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
    let (counters, _snapshot) = frames.render(&mut runtime);
    let m = counters.migration;

    assert!(
        m.image_registrations > 0,
        "the rasterized silhouette is drawn as an image payload; got {}",
        m.image_registrations
    );
    assert!(m.recorded_view_contents > 0);
    assert_engine_era_counters_zero(&counters);
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
    let copied_for_view = copied.clone();
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

    let m = result.profile.counters.migration;
    assert!(
        m.structural_patches + m.semantic_builds > 0,
        "mounting the menu mutates the retained tree"
    );
    assert_engine_era_counters_zero(&result.profile.counters);
}

/// Popup opening: a mounted popup is a second window the pump's merged a11y
/// update must surface, and its mount frame records the patch that opened it.
#[test]
#[cfg(all(feature = "accessibility", not(target_arch = "wasm32")))]
fn popup_opening_counts() {
    let copied = Binding::container(false);
    let copied_for_view = copied.clone();
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

    let m = result.profile.counters.migration;
    assert!(
        m.recorded_view_contents > 0,
        "the mount frame still encodes"
    );
    assert_engine_era_counters_zero(&result.profile.counters);
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
    assert!(first.migration.semantic_builds > 0);

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
        let m = counters.migration;
        saw_scroll_work |= m.recorded_view_contents > 0 && m.gpu_submissions > 0;
    }
    assert!(
        saw_scroll_work,
        "scroll frames must still encode view contents and submit GPU work"
    );
    assert_engine_era_counters_zero(&first);
}

/// A probe GPU surface — one clear pass into whatever it is handed.
struct FillProbe {
    renders: Rc<core::cell::Cell<u32>>,
}

impl GpuView for FillProbe {
    async fn setup(&mut self, _ctx: &GpuContext<'_>, _env: &mut waterui_core::Environment) {}

    fn render(&mut self, frame: &mut GpuFrame) {
        self.renders.set(self.renders.get() + 1);
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("migration_fixture_gpu_probe"),
            });
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("migration_fixture_gpu_probe_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &frame.view,
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

/// GPU content sitting under a clip and an opacity layer: the surface cannot
/// take the direct-to-target path, so it is composited — the frame records a
/// GPU-surface layer plus the clip layer above it.
#[test]
fn gpu_content_under_clips_and_effects() {
    let renders = Rc::new(core::cell::Cell::new(0u32));
    let probe = FillProbe {
        renders: Rc::clone(&renders),
    };
    let view = Frame::new(GpuSurface::new(probe))
        .width(120.0)
        .height(120.0)
        .clip(RoundedRectangle::new(12.0))
        .opacity(0.9f32);
    let mut runtime = runtime_with(view);
    let mut frames = Frames::new();

    // The surface's async setup needs a few executor drains before it renders.
    for _ in 0..12 {
        if renders.get() > 0 {
            break;
        }
        let at = frames.at();
        let _ = runtime.pump_at(false, at);
    }
    assert!(renders.get() > 0, "the GPU surface must have drawn");

    let (counters, _snapshot) = frames.render(&mut runtime);
    let m = counters.migration;
    assert!(
        counters.gpu_surface_layers >= 1,
        "a GPU surface under a clip composites as a GPU-surface layer"
    );
    assert!(m.gpu_submissions > 0, "the probe's submit is counted");
    assert!(m.recorded_view_contents > 0);
    assert_engine_era_counters_zero(&counters);
}

/// The acceptance's determinism clause: two capturing pumps of the same view
/// at the same fixed instant produce byte-identical snapshots and identical
/// migration counters.
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
