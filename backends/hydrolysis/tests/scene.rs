//! Renderer presentation tests for `SceneView`: intrinsic-size layout
//! proposals and the captured-frame path that runs `build_scene`.
//!
//! Received from water-rs/waterui under water-rs/waterui#1130 (class 2 —
//! renderer presentation); every case names its origin file and asserts what
//! it asserted there, mounted under `Material3::defaults()` on the rendered
//! runtime.

use std::cell::Cell;
use std::rc::Rc;

use waterui::View;
use waterui::ViewExt as _;
use waterui::component::hstack;
use waterui::graphics::cherenkov::{Draw, Recorder};
use waterui::graphics::{RecordingResources, SceneContent, SceneView};
use waterui::layout::frame::Frame;
use waterui::layout::scroll::ScrollView;
use waterui::text::text;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::layout::Size;
use waterui_core::{AnyView, Environment};
use waterui_testing::{NodeBounds, OffscreenApp, Role, ui};

struct TestSceneContent(Rc<Cell<bool>>);

impl SceneContent for TestSceneContent {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        _resources: &mut RecordingResources<'_>,
        width: f32,
        height: f32,
    ) -> bool {
        self.0.set(true);
        recorder.fill(
            kurbo::Rect::from_origin_size(
                kurbo::Point::new(8.0, 8.0),
                kurbo::Size::new(f64::from(width.min(40.0)), f64::from(height.min(24.0))),
            ),
            cherenkov::Paint::Solid(cherenkov::WorkingColor::new([1.0, 0.0, 0.0, 1.0])),
        );
        false
    }

    fn rebuild_for_engine(&mut self) {}
}

/// Scene content that *is* a size: 100 x 200 logical points, twice as tall as
/// it is wide. Stands in for an SVG's `viewBox`, an image's pixel dimensions,
/// or a formula's typeset box — everything the intrinsic-size hook exists for.
struct NaturallySizedContent;

impl NaturallySizedContent {
    const NATURAL: Size = Size {
        width: 100.0,
        height: 200.0,
    };
}

impl SceneContent for NaturallySizedContent {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        _resources: &mut RecordingResources<'_>,
        width: f32,
        height: f32,
    ) -> bool {
        recorder.fill(
            kurbo::Rect::from_origin_size(
                kurbo::Point::ZERO,
                kurbo::Size::new(f64::from(width), f64::from(height)),
            ),
            cherenkov::Paint::Solid(cherenkov::WorkingColor::new([0.0, 0.4, 1.0, 1.0])),
        );
        false
    }

    fn intrinsic_size(&self) -> Option<Size> {
        Some(Self::NATURAL)
    }

    fn rebuild_for_engine(&mut self) {}
}

fn naturally_sized_scene() -> impl View {
    SceneView::new(NaturallySizedContent)
        .a11y_role(waterui::accessibility::AccessibilityRole::Image)
        .a11y_label("Intrinsic scene")
}

fn sizeless_scene() -> impl View {
    SceneView::new(TestSceneContent(Rc::new(Cell::new(false))))
        .a11y_role(waterui::accessibility::AccessibilityRole::Image)
        .a11y_label("Sizeless scene")
}

fn scene_bounds(app: &mut OffscreenApp, label: &str) -> NodeBounds {
    app.query().role(Role::IMAGE).label(label).single().bounds()
}

/// (a) The scroll axis proposes nothing, so the scene is laid out at the height
/// it naturally is instead of collapsing to zero — the defect in #253. The
/// viewport is deliberately shorter than the content so the scroll view's
/// `max(content, viewport)` cannot hide the answer.
///
/// Origin: waterui `testing/src/tests.rs`.
#[test]
fn scene_with_a_natural_size_keeps_it_on_an_unconstrained_scroll_axis() {
    let mut app = ui()
        .viewport(100, 120)
        .theme(hydrolysis_m3::Material3::defaults())
        .mount_offscreen(|| ScrollView::vertical(naturally_sized_scene()));

    let bounds = scene_bounds(&mut app, "Intrinsic scene");
    assert!(
        (bounds.height() - NaturallySizedContent::NATURAL.height).abs() < 0.5,
        "the unconstrained scroll axis must resolve to the natural height, got {}",
        bounds.height()
    );
}

/// (b) Given a box, the scene still fills it: an intrinsic size is what layout
/// falls back to, never a cap on what a container may ask for.
///
/// Origin: waterui `testing/src/tests.rs`.
#[test]
fn scene_with_a_natural_size_still_fills_a_frame() {
    let mut app = ui()
        .viewport(400, 400)
        .theme(hydrolysis_m3::Material3::defaults())
        .mount_offscreen(|| {
            Frame::new(naturally_sized_scene())
                .width(160.0)
                .height(90.0)
        });

    let bounds = scene_bounds(&mut app, "Intrinsic scene");
    assert!(
        (bounds.width() - 160.0).abs() < 0.5 && (bounds.height() - 90.0).abs() < 0.5,
        "a framed scene must fill its frame, got {}x{}",
        bounds.width(),
        bounds.height()
    );
}

/// (c) One axis named, the other open: the natural aspect ratio settles the open
/// one. A vertical scroll view names the width and leaves the height open, which
/// is exactly the `.resizable()`-image case in #253.
///
/// Origin: waterui `testing/src/tests.rs`.
#[test]
fn one_named_axis_drives_the_other_by_the_natural_aspect_ratio() {
    let mut app = ui()
        .viewport(200, 120)
        .theme(hydrolysis_m3::Material3::defaults())
        .mount_offscreen(|| ScrollView::vertical(naturally_sized_scene()));

    let bounds = scene_bounds(&mut app, "Intrinsic scene");
    // 200 is twice the natural width, so the height is twice the natural height.
    assert!(
        (bounds.width() - 200.0).abs() < 0.5 && (bounds.height() - 400.0).abs() < 0.5,
        "the natural 100x200 ratio must carry the named width to the open height, got {}x{}",
        bounds.width(),
        bounds.height()
    );
}

/// Content with no size of its own is untouched by any of this: it still fills
/// whatever the scroll view gives it.
///
/// Origin: waterui `testing/src/tests.rs`.
#[test]
fn scene_without_a_natural_size_still_fills_its_container() {
    let mut app = ui()
        .viewport(200, 120)
        .theme(hydrolysis_m3::Material3::defaults())
        .mount_offscreen(|| ScrollView::vertical(sizeless_scene()));

    let bounds = scene_bounds(&mut app, "Sizeless scene");
    assert!(
        (bounds.width() - 200.0).abs() < 0.5 && (bounds.height() - 120.0).abs() < 0.5,
        "a scene with no natural size must fill the viewport, got {}x{}",
        bounds.width(),
        bounds.height()
    );
}

/// A scene that is naturally a size is content-sized: it takes its own width in
/// a row and leaves the rest to its sibling, rather than eating the row the way
/// a background or a shader legitimately does.
///
/// Origin: waterui `testing/src/tests.rs`.
#[test]
fn a_naturally_sized_scene_does_not_eat_the_row() {
    let mut app = ui()
        .viewport(400, 200)
        .theme(hydrolysis_m3::Material3::defaults())
        .mount_offscreen(|| {
            hstack((
                naturally_sized_scene(),
                text("beside it").a11y_label("beside it"),
            ))
        });

    let bounds = scene_bounds(&mut app, "Intrinsic scene");
    assert!(
        (bounds.width() - NaturallySizedContent::NATURAL.width).abs() < 0.5,
        "a content-sized scene takes its own width and leaves the rest of the row \
         to its sibling, got {}",
        bounds.width()
    );
}

/// A `SceneView` mounted on the rendered runtime runs `build_scene` and the
/// frame it produces reads back as a buffer of the window's size.
///
/// Origin: waterui `testing/src/tests.rs`
/// (`smoke_scene_view_snapshot_runs_build_scene_and_returns_buffer`), exercised
/// through `HeadlessRuntime`'s public pump — the same `capture_window_tree` →
/// `render_scene_to_texture` → readback chain, instead of the crate-private
/// `readback_texture_rgba8` helper the origin called directly.
#[test]
fn smoke_scene_view_snapshot_runs_build_scene_and_returns_buffer() {
    let build_called = Rc::new(Cell::new(false));
    let content = AnyViewBuilder::new({
        let build_called = Rc::clone(&build_called);
        move || AnyView::new(SceneView::new(TestSceneContent(Rc::clone(&build_called))))
    });
    let mut runtime = hydrolysis::HeadlessRuntime::new_for_tests(
        Environment::new(),
        content,
        96,
        72,
        hydrolysis_m3::Material3::defaults(),
    );

    let mut settled = false;
    for _ in 0..64 {
        if !runtime.pump_at(false, std::time::Instant::now()).rebuilt {
            settled = true;
            break;
        }
    }
    assert!(settled, "frame never settled");
    assert!(build_called.get(), "expected scene view build_scene to run");

    let snapshot = runtime
        .pump_at(true, std::time::Instant::now())
        .snapshot
        .expect("capture must produce a snapshot");
    assert_eq!(snapshot.width, 96);
    assert_eq!(snapshot.height, 72);
    assert_eq!(snapshot.rgba8.len(), 96 * 72 * 4);
}
