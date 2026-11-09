use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::Cell;

use nami::{Signal, SignalExt as _, binding};
use waterui_core::Environment;
use waterui_core::layout::{Affine2, Point, Rect, Size};

use super::scene::CanvasScene;
use super::{
    Canvas, Circle, Drawing, ImagePattern, ImageSource, ImageStatus, Layer, PixelBuffer,
    PixelFormat, Sampling, StrokeStyle,
};
use crate::color::WorkingColor;
use crate::draw::{Content, ContentChange, LayoutSize, Operand};
use crate::scene::resources::tests::{Mount, null_resources, removed_images};
use crate::scene_view::{SceneContent, SceneInvalidator};

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
    assert_eq!(record(&mut scene).len(), 3);

    count.set(4);
    assert_eq!(scope_runs.get(), 2, "the scope records again");
    assert_eq!(root_runs.get(), 1, "the closure does not");
    assert_eq!(invalidations.get(), 1, "the host is asked for one pass");
    assert_eq!(record(&mut scene).len(), 5);
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
    assert_eq!(record(&mut scene).len(), 3);

    // A move reorders without recording; a removal drops.
    dots.set(Vec::from([Dot { id: 2, x: 10.0 }, Dot { id: 1, x: 5.0 }]));
    assert_eq!(item_runs.get(), 3, "a move and a removal record nothing");
    assert_eq!(invalidations.get(), 2);
    assert_eq!(record(&mut scene).len(), 2);
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
    assert_eq!(removed_images(&first.events), Vec::new());
    // Drawn again: the same registration, nothing released.
    let again = host.frame(&mut scene);
    assert_eq!(removed_images(&again.events), Vec::new());

    shown.set(false);
    let after = host.frame(&mut scene);
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
