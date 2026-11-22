//! Rendered-runtime contracts for `SceneView`: intrinsic-size layout
//! proposals and the captured-frame path that runs `build_scene`. The layout
//! semantics are the engine's — how `SceneContent::intrinsic_size` feeds a
//! proposal — so these stay with the backend, mounted under `TestStyle`.
//!
//! Received from water-rs/waterui under water-rs/waterui#1130 (class 2 —
//! renderer presentation); every case names its origin file and asserts what
//! it asserted there.

mod support;

use std::cell::Cell;
use std::rc::Rc;

use hydrolysis::draw::{Draw, Recorder};
use kurbo::Rect;
use support::{mount_rendered, node_bounds_with_role};
use waterui::View;
use waterui::ViewExt as _;
use waterui::component::hstack;
use waterui::graphics::backend::{NextFrame, RecordingResources, SceneContent, SceneView};
use waterui::layout::frame::Frame;
use waterui::layout::scroll::ScrollView;
use waterui::text::text;
use waterui_core::Environment;
use waterui_core::layout::Size;

struct TestSceneContent(Rc<Cell<bool>>);

impl SceneContent for TestSceneContent {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        _resources: &mut RecordingResources<'_>,
        width: f32,
        height: f32,
    ) -> NextFrame {
        self.0.set(true);
        recorder.fill(
            Rect::from_origin_size(
                kurbo::Point::new(8.0, 8.0),
                kurbo::Size::new(f64::from(width.min(40.0)), f64::from(height.min(24.0))),
            ),
            hydrolysis::draw::Paint::Solid(hydrolysis::draw::WorkingColor::new([
                1.0, 0.0, 0.0, 1.0,
            ])),
        );
        NextFrame::Idle
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
    ) -> NextFrame {
        recorder.fill(
            Rect::from_origin_size(
                kurbo::Point::ZERO,
                kurbo::Size::new(f64::from(width), f64::from(height)),
            ),
            hydrolysis::draw::Paint::Solid(hydrolysis::draw::WorkingColor::new([
                0.0, 0.4, 1.0, 1.0,
            ])),
        );
        NextFrame::Idle
    }

    fn rebuild_for_engine(&mut self) {}

    fn intrinsic_size(&self) -> Option<Size> {
        Some(Self::NATURAL)
    }
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

/// (a) The scroll axis proposes nothing, so the scene is laid out at the height
/// it naturally is instead of collapsing to zero — the defect in #253. The
/// viewport is deliberately shorter than the content so the scroll view's
/// `max(content, viewport)` cannot hide the answer.
///
/// Origin: waterui `testing/src/tests.rs`.
#[test]
fn scene_with_a_natural_size_keeps_it_on_an_unconstrained_scroll_axis() {
    let mut app = mount_rendered(
        Environment::new(),
        || ScrollView::vertical(naturally_sized_scene()),
        100,
        120,
    );
    app.settle();

    let bounds = node_bounds_with_role(app.tree(), accesskit::Role::Image, "Intrinsic scene");
    assert!(
        (bounds.height() - f64::from(NaturallySizedContent::NATURAL.height)).abs() < 0.5,
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
    let mut app = mount_rendered(
        Environment::new(),
        || {
            Frame::new(naturally_sized_scene())
                .width(160.0)
                .height(90.0)
        },
        400,
        400,
    );
    app.settle();

    let bounds = node_bounds_with_role(app.tree(), accesskit::Role::Image, "Intrinsic scene");
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
    let mut app = mount_rendered(
        Environment::new(),
        || ScrollView::vertical(naturally_sized_scene()),
        200,
        120,
    );
    app.settle();

    let bounds = node_bounds_with_role(app.tree(), accesskit::Role::Image, "Intrinsic scene");
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
    let mut app = mount_rendered(
        Environment::new(),
        || ScrollView::vertical(sizeless_scene()),
        200,
        120,
    );
    app.settle();

    let bounds = node_bounds_with_role(app.tree(), accesskit::Role::Image, "Sizeless scene");
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
    let mut app = mount_rendered(
        Environment::new(),
        || {
            hstack((
                naturally_sized_scene(),
                text("beside it").a11y_label("beside it"),
            ))
        },
        400,
        200,
    );
    app.settle();

    let bounds = node_bounds_with_role(app.tree(), accesskit::Role::Image, "Intrinsic scene");
    assert!(
        (bounds.width() - f64::from(NaturallySizedContent::NATURAL.width)).abs() < 0.5,
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
/// `render_scene_to_texture` → readback chain, instead of the public
/// `readback_texture_rgba8` helper the origin called directly.
#[test]
fn smoke_scene_view_snapshot_runs_build_scene_and_returns_buffer() {
    use hydrolysis::testing::TestStyle;
    use waterui_core::AnyView;
    use waterui_core::handler::AnyViewBuilder;

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
        TestStyle::new(),
    );

    let mut settled = false;
    for _ in 0..64 {
        if !runtime.pump_at(false, std::time::Instant::now()).rebuilt() {
            settled = true;
            break;
        }
    }
    assert!(settled, "frame never settled");
    assert!(build_called.get(), "expected scene view build_scene to run");

    let snapshot = runtime
        .pump_at(true, std::time::Instant::now())
        .into_snapshot()
        .expect("capture must produce a snapshot");
    assert_eq!(snapshot.width(), 96);
    assert_eq!(snapshot.height(), 72);
    assert_eq!(snapshot.rgba8().len(), 96 * 72 * 4);
}

/// One 60 Hz frame of the pumped clock.
const FRAME: std::time::Duration = std::time::Duration::from_micros(16_667);

/// A rendered runtime showing a 40 x 40 canvas whose square slides along the
/// clock `clock` hands it, the seconds it has read reported to `seen`.
fn clocked_canvas(
    seen: &Rc<Cell<std::time::Duration>>,
    clock: impl Fn(&waterui::graphics::canvas::Cx) -> waterui::reactive::Computed<std::time::Duration>
    + 'static,
) -> hydrolysis::HeadlessRuntime {
    use hydrolysis::testing::TestStyle;
    use waterui::SignalExt as _;
    use waterui::graphics::canvas::Canvas;
    use waterui::graphics::color::WorkingColor;
    use waterui_core::AnyView;
    use waterui_core::handler::AnyViewBuilder;
    use waterui_core::layout::{Affine2, Rect};

    let clock = Rc::new(clock);
    let seen = Rc::clone(seen);
    let content = AnyViewBuilder::new(move || {
        let clock = Rc::clone(&clock);
        let seen = Rc::clone(&seen);
        AnyView::new(
            Canvas::new(move |cx| {
                let time = clock(cx);
                let seen = Rc::clone(&seen);
                cx.transform(
                    time.map(move |t| {
                        seen.set(t);
                        Affine2::translate(t.as_secs_f32(), 0.0)
                    }),
                    |cx| {
                        cx.fill(
                            Rect::from_size(Size::new(8.0, 8.0)),
                            WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
                        );
                    },
                );
            })
            .size(40.0, 40.0),
        )
    });
    hydrolysis::HeadlessRuntime::new_for_tests(
        Environment::new(),
        content,
        96,
        72,
        TestStyle::new(),
    )
}

/// A canvas following its unbounded clock asks for every frame for as long
/// as it is shown: the runtime presents them, and counts them as perpetual
/// work — never settled, yet with nothing left that ends on its own.
#[test]
fn a_canvas_on_its_unbounded_clock_is_perpetual_work() {
    let seen = Rc::new(Cell::new(std::time::Duration::ZERO));
    let mut runtime = clocked_canvas(&seen, waterui::graphics::canvas::Cx::time);
    let mut at = std::time::Instant::now();
    for _ in 0..30 {
        at += FRAME;
        let _ = runtime.pump_at(false, at);
    }
    assert!(
        seen.get() >= FRAME * 25,
        "the canvas clock advances with the presented frames: {:?}",
        seen.get()
    );
    assert!(
        !runtime.is_settled(),
        "the clocked canvas is scheduled work for as long as it is shown"
    );
    assert!(
        runtime.is_settled_except_perpetual(),
        "nothing but work with no end remains"
    );
    assert!(
        !runtime.has_pending_semantic_update(),
        "the frames it asks for continue it; they are no unapplied change"
    );
}

/// A canvas whose only clock holds at an end asks for frames until it gets
/// there: that is finite work, and the runtime comes fully to rest after it.
#[test]
fn a_canvas_on_a_bounded_clock_is_finite_work_until_its_end() {
    let end = std::time::Duration::from_millis(200);
    let seen = Rc::new(Cell::new(std::time::Duration::ZERO));
    let mut runtime = clocked_canvas(&seen, move |cx| cx.time_until(end));
    let mut at = std::time::Instant::now();
    for _ in 0..4 {
        at += FRAME;
        let _ = runtime.pump_at(false, at);
    }
    assert!(
        !runtime.is_settled_except_perpetual(),
        "short of its end the bounded clock is work that will finish"
    );
    let mut frames = 0;
    while !runtime.is_settled() {
        assert!(frames < 60, "the bounded clock never reached its end");
        at += FRAME;
        let _ = runtime.pump_at(false, at);
        frames += 1;
    }
    assert_eq!(seen.get(), end, "the clock holds at its end");
}
