use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::Cell;

use nami::{Signal, SignalExt as _, binding};
use waterui_core::Environment;
use waterui_core::layout::{Affine2, Point, Rect, Size};

use super::names::Names;
use super::paint::affine;
use super::scene::CanvasScene;
use super::{
    Canvas, CanvasSize, Circle, Drawing, ImagePattern, ImageSource, ImageStatus, Layer,
    LinearGradient, Paint, PixelBuffer, PixelFormat, Sampling, StrokeStyle,
};
use crate::color::WorkingColor;
use crate::draw::{self, Content, ContentChange, LayoutSize, Operand};
use crate::scene::resources::tests::{Mount, null_resources, removed_images};
use crate::scene_view::{SceneContent, SceneFrame, SceneInvalidator, SceneUpdate};

fn mount(canvas: Canvas) -> CanvasScene {
    CanvasScene::mount(canvas, &Environment::new())
}

/// Counts the host passes the canvas asks for.
fn counting_invalidator(scene: &mut CanvasScene) -> Rc<Cell<u32>> {
    let count = Rc::new(Cell::new(0));
    let invalidator: SceneInvalidator = {
        let count = Rc::clone(&count);
        Rc::new(move || count.set(count.get() + 1))
    };
    scene.set_invalidator(Some(invalidator));
    count
}

/// One recording of `scene` against a `Null` engine's table.
fn record(scene: &mut CanvasScene) -> Content {
    let (resources, _events) = null_resources();
    let mut names = resources.recording();
    let content = Content::record(&LayoutSize::new(), |recorder| {
        scene.build_scene(recorder, &mut names, 100.0, 50.0);
    });
    drop(names.finish());
    content
}

/// One frame of a host that keeps `scene`'s recording installed.
fn update(scene: &mut CanvasScene) -> SceneUpdate {
    update_at(scene, crate::draw::Instant::now())
}

/// One frame presented at `time` by a host that keeps `scene`'s recording
/// installed.
fn update_at(scene: &mut CanvasScene, time: crate::draw::Instant) -> SceneUpdate {
    let (resources, _events) = null_resources();
    scene.update_scene(
        &resources,
        SceneFrame {
            width: 100.0,
            height: 50.0,
            time,
        },
    )
}

/// The structural edit the installed `content` received since its last
/// change, then its length.
fn edited(content: &mut Content) -> usize {
    let Some(ContentChange::Edit(edit)) = content.take_change() else {
        panic!("a structural change edits the installed recording in place");
    };
    let [_, ..] = edit.splices[..] else {
        panic!("an edit splices the changed region's commands");
    };
    content.len()
}

fn red() -> WorkingColor {
    WorkingColor::new([1.0, 0.0, 0.0, 1.0])
}

fn pixel() -> ImageSource {
    ImageSource::new(PixelBuffer::new(
        1,
        1,
        PixelFormat::Rgba8,
        Vec::from([255, 0, 0, 255]),
    ))
    .expect("a valid one-pixel image")
}

#[test]
fn a_signal_operand_updates_in_place_without_recording_again() {
    let radius = binding(4.0_f32);
    let runs = Rc::new(Cell::new(0));
    let mut scene = mount(Canvas::new({
        let radius = radius.clone();
        let runs = Rc::clone(&runs);
        move |cx| {
            runs.set(runs.get() + 1);
            cx.fill(Rect::from_size(Size::new(10.0, 10.0)), red());
            cx.fill(radius.map(|r| Circle::new(Point::new(5.0, 5.0), r)), red());
        }
    }));
    let invalidations = counting_invalidator(&mut scene);
    let mut content = record(&mut scene);
    let Some(ContentChange::Replace(_)) = content.take_change() else {
        panic!("the first commit sends the whole list");
    };

    radius.set(8.0);
    let Some(ContentChange::Update(updates)) = content.take_change() else {
        panic!("a changed operand sends a slot update");
    };
    assert_eq!(updates.len(), 1, "only the circle's shape changes");
    assert_eq!(updates[0].command, 1);
    assert!(matches!(updates[0].value, Operand::Shape(_)));
    assert_eq!(runs.get(), 1, "the closure runs once");
    assert_eq!(invalidations.get(), 0, "an operand change records nothing");
}

#[test]
fn an_app_animation_animates_the_operand() {
    let x = binding(0.0_f32);
    let mut scene = mount(Canvas::new({
        let x = x.clone();
        move |cx| {
            let animated = x.with(waterui_core::animation::Animation::linear(
                core::time::Duration::from_millis(200),
            ));
            cx.transform(animated.map(|x| Affine2::translate(x, 0.0)), |cx| {
                cx.fill(Rect::from_size(Size::new(1.0, 1.0)), red());
            });
        }
    }));
    let mut content = record(&mut scene);
    let _ = content.take_change();
    x.set(100.0);
    assert_eq!(
        content.take_change(),
        None,
        "an animated change waits for the frame that samples it"
    );
    let start = crate::draw::Instant::now();
    assert!(content.sample(start).is_animating());
    assert!(
        content
            .sample(start + core::time::Duration::from_millis(100))
            .is_animating(),
        "half-way through, the operand is still moving"
    );
    let Some(ContentChange::Update(updates)) = content.take_change() else {
        panic!("a sampled frame sends the operand's interpolated value");
    };
    let Operand::Transform(mid) = updates[0].value else {
        panic!("the transform scope's operand");
    };
    let x = mid.translation().x;
    assert!(x > 10.0 && x < 90.0, "mid-flight translation {x}");
}

#[test]
fn a_theme_independent_paint_signal_updates_its_slot() {
    let color = binding(red());
    let mut scene = mount(Canvas::new({
        let color = color.clone();
        move |cx| cx.fill(Rect::from_size(Size::new(10.0, 10.0)), color)
    }));
    let mut content = record(&mut scene);
    let _ = content.take_change();
    color.set(WorkingColor::WHITE);
    let Some(ContentChange::Update(updates)) = content.take_change() else {
        panic!("a changed paint sends a slot update");
    };
    assert!(matches!(updates[0].value, Operand::Paint(_)));
}

#[test]
fn a_scope_records_again_alone() {
    let count = binding(2_usize);
    let root_runs = Rc::new(Cell::new(0));
    let scope_runs = Rc::new(Cell::new(0));
    let mut scene = mount(Canvas::new({
        let count = count.clone();
        let (root_runs, scope_runs) = (Rc::clone(&root_runs), Rc::clone(&scope_runs));
        move |cx| {
            root_runs.set(root_runs.get() + 1);
            cx.fill(Rect::from_size(Size::new(10.0, 10.0)), red());
            let scope_runs = Rc::clone(&scope_runs);
            cx.scope(count, move |cx, count: &usize| {
                scope_runs.set(scope_runs.get() + 1);
                for i in 0..*count {
                    #[expect(clippy::cast_precision_loss, reason = "a handful of test circles")]
                    let x = i as f32 * 10.0;
                    cx.fill(Circle::new(Point::new(x, 0.0), 2.0), red());
                }
            });
        }
    }));
    let invalidations = counting_invalidator(&mut scene);
    let mut content = record(&mut scene);
    assert_eq!(content.len(), 3);
    let _ = content.take_change();

    count.set(4);
    assert_eq!(scope_runs.get(), 2, "the scope records again");
    assert_eq!(root_runs.get(), 1, "the closure does not");
    assert_eq!(invalidations.get(), 1, "the host is asked for one pass");
    assert_eq!(
        update(&mut scene),
        SceneUpdate::Retained { again: false },
        "the installed recording stays"
    );
    assert_eq!(edited(&mut content), 5);
}

#[derive(Clone, Debug, PartialEq)]
struct Dot {
    id: u32,
    x: f32,
}

#[test]
fn each_records_inserted_keys_only_and_updates_changed_items_in_place() {
    let dots = binding(Vec::from([Dot { id: 1, x: 0.0 }, Dot { id: 2, x: 10.0 }]));
    let item_runs = Rc::new(Cell::new(0));
    let mut scene = mount(Canvas::new({
        let dots = dots.clone();
        let item_runs = Rc::clone(&item_runs);
        move |cx| {
            cx.each(
                dots,
                |dot: &Dot| dot.id,
                move |cx, dot| {
                    item_runs.set(item_runs.get() + 1);
                    cx.fill(
                        dot.map(|dot| Circle::new(Point::new(dot.x, 0.0), 2.0)),
                        red(),
                    );
                },
            );
        }
    }));
    let invalidations = counting_invalidator(&mut scene);
    let mut content = record(&mut scene);
    assert_eq!(content.len(), 2);
    assert_eq!(item_runs.get(), 2);
    let _ = content.take_change();

    // A changed item under the same key: an in-place slot update.
    dots.set(Vec::from([Dot { id: 1, x: 5.0 }, Dot { id: 2, x: 10.0 }]));
    assert_eq!(item_runs.get(), 2, "a changed item records nothing");
    assert_eq!(invalidations.get(), 0);
    let Some(ContentChange::Update(updates)) = content.take_change() else {
        panic!("the moved dot updates its slot");
    };
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].command, 0);

    // An inserted key records once; the others are reused.
    dots.set(Vec::from([
        Dot { id: 1, x: 5.0 },
        Dot { id: 3, x: 20.0 },
        Dot { id: 2, x: 10.0 },
    ]));
    assert_eq!(item_runs.get(), 3, "only the new key records");
    assert_eq!(invalidations.get(), 1);
    assert!(matches!(update(&mut scene), SceneUpdate::Retained { .. }));
    assert_eq!(edited(&mut content), 3);

    // A move reorders without recording; a removal drops.
    dots.set(Vec::from([Dot { id: 2, x: 10.0 }, Dot { id: 1, x: 5.0 }]));
    assert_eq!(item_runs.get(), 3, "a move and a removal record nothing");
    assert_eq!(invalidations.get(), 2);
    assert!(matches!(update(&mut scene), SceneUpdate::Retained { .. }));
    assert_eq!(edited(&mut content), 2);

    // The moved item's subscription follows it to its new index.
    dots.set(Vec::from([Dot { id: 2, x: 10.0 }, Dot { id: 1, x: 7.0 }]));
    let Some(ContentChange::Update(updates)) = content.take_change() else {
        panic!("the moved dot still updates its slot in place");
    };
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].command, 1, "dot 1 now draws second");
}

#[test]
fn kept_values_drop_with_their_region() {
    struct Probe(Rc<Cell<bool>>);
    impl Drop for Probe {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    let shown = binding(true);
    let dropped = Rc::new(Cell::new(false));
    let scene = mount(Canvas::new({
        let shown = shown.clone();
        let dropped = Rc::clone(&dropped);
        move |cx| {
            cx.scope(shown, move |cx, shown: &bool| {
                if *shown {
                    cx.keep(Probe(Rc::clone(&dropped)));
                }
            });
        }
    }));
    assert!(!dropped.get());
    shown.set(false);
    assert!(dropped.get(), "the region that kept it recorded again");
    drop(scene);
}

#[test]
fn a_drawing_is_send_and_replays_on_a_new_engine() {
    fn assert_send_sync<T: Send + Sync + Clone>() {}
    assert_send_sync::<Drawing>();
    let image = pixel();
    let drawing = std::thread::spawn({
        let image = image.clone();
        move || {
            Drawing::record(Size::new(4.0, 4.0), |cx| {
                cx.fill(Rect::from_size(Size::new(4.0, 4.0)), red());
                cx.image(
                    &image,
                    Rect::from_size(Size::new(4.0, 4.0)),
                    Sampling::Nearest,
                );
                cx.layer(Layer::opacity(0.5), |cx| {
                    cx.stroke(
                        Circle::new(Point::new(2.0, 2.0), 1.0),
                        &StrokeStyle::new(1.0),
                        WorkingColor::WHITE,
                    );
                });
            })
        }
    })
    .join()
    .expect("recording off the main thread");
    let mut scene = mount(Canvas::from(drawing));
    assert_eq!(scene.intrinsic_size(), Some(Size::new(4.0, 4.0)));
    assert_eq!(image.status().snapshot(), ImageStatus::Pending);

    let first = Mount::new();
    let report = first.frame(&mut scene);
    assert_eq!(report.drawn.len(), 1, "the drawing draws its image");
    assert_eq!(image.status().snapshot(), ImageStatus::Ready);

    // A replacement engine: the canvas registers again on its own.
    let second = Mount::new();
    scene.rebuild_for_engine();
    let report = second.frame(&mut scene);
    assert_eq!(report.drawn.len(), 1, "the new engine draws the image too");
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, cherenkov::testing::Event::AddImage(_))),
        "the image is registered on the new engine: {:?}",
        report.events
    );
}

#[test]
fn an_image_is_held_while_drawn_and_released_after() {
    let shown = binding(true);
    let image = pixel();
    let mut scene = mount(Canvas::new({
        let shown = shown.clone();
        move |cx| {
            cx.scope(shown, move |cx, shown: &bool| {
                if *shown {
                    cx.fill(
                        Rect::from_size(Size::new(8.0, 8.0)),
                        ImagePattern::new(image.clone()),
                    );
                }
            });
        }
    }));
    let host = Mount::new();
    let first = host.frame(&mut scene);
    assert!(!first.retained, "the first frame installs the canvas");
    assert_eq!(removed_images(&first.events), Vec::new());
    // Presented again: the installed recording stays, nothing released.
    let again = host.frame(&mut scene);
    assert!(again.retained);
    assert_eq!(removed_images(&again.events), Vec::new());

    shown.set(false);
    let after = host.frame(&mut scene);
    assert!(
        after.retained,
        "the scope's region is recorded again in place"
    );
    assert_eq!(
        removed_images(&after.events).len(),
        1,
        "the image is released once no installed recording names it: {:?}",
        after.events
    );
}

#[test]
fn time_advances_only_while_observed() {
    let mut idle = mount(Canvas::new(|cx| {
        cx.fill(Rect::from_size(Size::new(1.0, 1.0)), red());
    }));
    let (resources, _events) = null_resources();
    let mut names = resources.recording();
    let mut animating = true;
    let _ = Content::record(&LayoutSize::new(), |recorder| {
        animating = idle.build_scene(recorder, &mut names, 1.0, 1.0);
    });
    assert!(
        !animating,
        "a canvas that does not read time asks for no frame"
    );

    let mut clocked = mount(Canvas::new(|cx| {
        let time = cx.time();
        cx.transform(
            time.map(|t| Affine2::translate(t.as_secs_f32(), 0.0)),
            |cx| cx.fill(Rect::from_size(Size::new(1.0, 1.0)), red()),
        );
    }));
    let _ = Content::record(&LayoutSize::new(), |recorder| {
        animating = clocked.build_scene(recorder, &mut names, 1.0, 1.0);
    });
    assert!(animating, "a canvas bound to time asks for the next frame");
    drop(names.finish());
}

#[test]
fn time_is_the_presented_frames_clock() {
    let mut scene = mount(Canvas::new(|cx| {
        let time = cx.time();
        cx.transform(
            time.map(|t| Affine2::translate(t.as_secs_f32() * 1000.0, 0.0)),
            |cx| cx.fill(Rect::from_size(Size::new(1.0, 1.0)), red()),
        );
    }));
    let start = crate::draw::Instant::now();
    // The first frame mounts the clock at its own instant, then records.
    assert_eq!(update_at(&mut scene, start), SceneUpdate::Record);
    let mut content = record(&mut scene);
    let _ = content.take_change();

    assert_eq!(
        update_at(&mut scene, start + core::time::Duration::from_millis(48)),
        SceneUpdate::Retained { again: true },
        "a canvas bound to time keeps asking for frames"
    );
    let Some(ContentChange::Update(updates)) = content.take_change() else {
        panic!("a presented frame updates the time operand in place");
    };
    let Operand::Transform(moved) = updates[0].value else {
        panic!("the transform scope's operand");
    };
    let x = moved.translation().x;
    assert!(
        (x - 48.0).abs() < 1e-3,
        "48 ms after mounting, time reads 48 ms: {x}"
    );
}

#[test]
fn time_until_holds_at_its_end_and_stops_asking_for_frames() {
    let end = core::time::Duration::from_millis(100);
    let mut scene = mount(Canvas::new(move |cx| {
        let time = cx.time_until(end);
        cx.transform(
            time.map(|t| Affine2::translate(t.as_secs_f32() * 1000.0, 0.0)),
            |cx| cx.fill(Rect::from_size(Size::new(1.0, 1.0)), red()),
        );
    }));
    let start = crate::draw::Instant::now();
    assert_eq!(update_at(&mut scene, start), SceneUpdate::Record);
    let mut content = record(&mut scene);
    let _ = content.take_change();

    assert_eq!(
        update_at(&mut scene, start + core::time::Duration::from_millis(48)),
        SceneUpdate::Retained { again: true },
        "before its end the clock keeps asking for frames"
    );
    let _ = content.take_change();
    assert_eq!(
        update_at(&mut scene, start + core::time::Duration::from_millis(500)),
        SceneUpdate::Retained { again: false },
        "at its end the clock stops asking for frames"
    );
    let Some(ContentChange::Update(updates)) = content.take_change() else {
        panic!("the last frame updates the time operand in place");
    };
    let Operand::Transform(held) = updates[0].value else {
        panic!("the transform scope's operand");
    };
    assert!(
        (held.translation().x - 100.0).abs() < 1e-3,
        "the clock holds at its end, not at the frame's time: {}",
        held.translation().x
    );
}

#[test]
fn size_follows_the_box() {
    let seen = Rc::new(Cell::new(Size::zero()));
    let mut scene = mount(Canvas::new({
        let seen = Rc::clone(&seen);
        move |cx| {
            let size = cx.size();
            cx.keep(size.watch(move |change| seen.set(*change.value())));
        }
    }));
    let _ = record(&mut scene);
    assert_eq!(seen.get(), Size::new(100.0, 50.0));
}

#[test]
fn canvas_size_before_mounting_reports_the_built_size() {
    let size = CanvasSize::new();
    let canvas = Canvas::new(|_| {}).sized_by(size.clone());
    let size = size.signal();
    assert_eq!(size.snapshot(), Size::zero(), "no layout has run yet");
    let mut scene = mount(canvas);
    let _ = record(&mut scene);
    assert_eq!(size.snapshot(), Size::new(100.0, 50.0));
    let _ = update(&mut scene);
    assert_eq!(size.snapshot(), Size::new(100.0, 50.0));
}

#[test]
fn canvas_size_follows_a_resize() {
    let size = CanvasSize::new();
    let canvas = Canvas::new(|_| {}).sized_by(size.clone());
    let size = size.signal();
    let seen = Rc::new(Cell::new(Size::zero()));
    let _guard = size.watch({
        let seen = Rc::clone(&seen);
        move |change| seen.set(*change.value())
    });
    let mut scene = mount(canvas);
    let _ = record(&mut scene);
    assert_eq!(seen.get(), Size::new(100.0, 50.0));

    let (resources, _events) = null_resources();
    let mut names = resources.recording();
    let _ = Content::record(&LayoutSize::new(), |recorder| {
        scene.build_scene(recorder, &mut names, 200.0, 80.0);
    });
    drop(names.finish());
    assert_eq!(size.snapshot(), Size::new(200.0, 80.0));
    assert_eq!(seen.get(), Size::new(200.0, 80.0));
}

#[test]
fn canvas_size_is_the_signal_the_closure_reads() {
    let inside = Rc::new(Cell::new(Size::zero()));
    let size = CanvasSize::new();
    let canvas = Canvas::new({
        let inside = Rc::clone(&inside);
        move |cx| {
            let size = cx.size();
            cx.keep(size.watch(move |change| inside.set(*change.value())));
        }
    })
    .sized_by(size.clone());
    let size = size.signal();
    let mut scene = mount(canvas);
    let _ = record(&mut scene);
    assert_eq!(inside.get(), size.snapshot());
    assert_eq!(inside.get(), Size::new(100.0, 50.0));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "second canvas")]
fn a_canvas_size_given_to_two_mounted_canvases_is_reported() {
    let size = CanvasSize::new();
    let _first = mount(Canvas::new(|_| {}).sized_by(size.clone()));
    let _second = mount(Canvas::new(|_| {}).sized_by(size));
}

#[test]
fn a_canvas_size_can_be_given_again_after_its_canvas_unmounts() {
    let size = CanvasSize::new();
    let first = mount(Canvas::new(|_| {}).sized_by(size.clone()));
    drop(first);
    let mut second = mount(Canvas::new(|_| {}).sized_by(size.clone()));
    let _ = record(&mut second);
    assert_eq!(size.signal().snapshot(), Size::new(100.0, 50.0));
}

fn skew() -> Affine2 {
    Affine2::new(2.0, 0.5, -0.5, 1.0, 3.0, 4.0)
}

#[test]
fn a_transformed_paint_lowers_to_a_transformed_draw_paint() {
    let gradient = LinearGradient::new(Point::new(0.0, 0.0), Point::new(10.0, 0.0));
    let paint = Paint::from(gradient.clone()).transformed(skew());
    let lowered = paint
        .lower(&Names::default())
        .expect("a gradient needs no resource");
    let draw::Paint::Transformed(lowered) = lowered else {
        panic!("a transformed paint lowers to a transformed draw paint");
    };
    assert_eq!(lowered.transform, affine(skew()));
    assert_eq!(
        *lowered.paint,
        Paint::from(gradient)
            .lower(&Names::default())
            .expect("a gradient needs no resource")
    );
}

#[test]
fn the_identity_transform_leaves_a_paint_alone() {
    let paint = Paint::from(red());
    assert_eq!(paint.clone().transformed(Affine2::IDENTITY), paint);
}

#[test]
fn transforming_a_transformed_paint_nests_it() {
    let inner = Affine2::scale(2.0, 3.0);
    let outer = Affine2::rotate(0.5);
    let paint = Paint::from(red()).transformed(inner).transformed(outer);
    let draw::Paint::Transformed(lowered) = paint.lower(&Names::default()).expect("a colour")
    else {
        panic!("a transformed paint lowers to a transformed draw paint");
    };
    assert_eq!(lowered.transform, affine(outer));
    let draw::Paint::Transformed(nested) = &*lowered.paint else {
        panic!("the earlier transform stays inside");
    };
    assert_eq!(nested.transform, affine(inner));
}

#[test]
fn a_transformed_image_pattern_is_still_registered() {
    let image = pixel();
    let mut scene = mount(Canvas::new({
        move |cx| {
            cx.fill(
                Rect::from_size(Size::new(8.0, 8.0)),
                Paint::from(ImagePattern::new(image)).transformed(skew()),
            );
        }
    }));
    let report = Mount::new().frame(&mut scene);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, cherenkov::testing::Event::AddImage(_))),
        "the image under the transform is registered: {:?}",
        report.events
    );
}
