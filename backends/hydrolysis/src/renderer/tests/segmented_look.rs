//! A segmented picker's look under a test theme (water-rs/waterui#4539):
//! a thumb part following the selection under the look's motion without
//! recording again, a drag between segments and its settle, the right-to-left
//! mirror, the group opacity of a disabled picker, a backdrop material over
//! the labels, and the accessibility tree the same placement emits.

use core::time::Duration;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use cherenkov_record::{Draw as _, ShapeData, WorkingColor};
use kurbo::Rect;
use nami::{Binding, Signal as _};
use waterui::ViewExt as _;
use waterui::animation::Animation;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::interaction::InteractionState;
use waterui_core::layout::LayoutDirection;
use waterui_core::{AnyView, Environment};
use waterui_form::picker::{PickerStyle, picker};
use waterui_text::text;

use super::{MinimalTestTheme, pumped_test_environment};
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::renderer::live_segmented::LiveSegmented;
use crate::theme::{
    Custom, Driven, Inputs, Motion, SegmentedDrag, SegmentedLook, SegmentedSpec, SegmentedValue,
};
use crate::{HeadlessRuntime, HeadlessSnapshot};

/// The window, in points; `SCALE` device pixels per point.
const WIDTH: u32 = 300;
const HEIGHT: u32 = 60;
const SCALE: f64 = 2.0;
const POINTER: u64 = 1;
/// The fixture's segment count: the window's width splits into segments
/// 100 points wide.
const COUNT: usize = 3;
const SEGMENT: f64 = 100.0;

fn red() -> WorkingColor {
    WorkingColor::new([1.0, 0.0, 0.0, 1.0])
}

/// The thumb at `inputs`' position: one segment wide, the row's height.
fn thumb_rect(inputs: &Inputs<'_, SegmentedValue>) -> Rect {
    let width = inputs.frame.width() / crate::num_cast::usize_as_f64(COUNT);
    let x0 = inputs.value.position.mul_add(width, inputs.frame.x0);
    Rect::new(x0, inputs.frame.y0, x0 + width, inputs.frame.y1)
}

/// The plain test look with a red thumb below the labels, its changes
/// running with `motion`.
fn thumb_look(motion: Motion) -> SegmentedLook {
    let plain = crate::testing::plain_segmented_look();
    SegmentedLook::new(*plain.layout(), plain.label().clone(), motion).part(Custom::new(
        (Driven::<ShapeData, SegmentedValue>::new(|inputs| {
            ShapeData::of(&thumb_rect(inputs))
        }),),
        |recorder, _frame, (thumb,)| recorder.fill(thumb, red()),
    ))
}

fn linear(millis: u64) -> Motion {
    Motion::uniform(Animation::linear(Duration::from_millis(millis)))
}

/// A segmented picker of three segments in a window, pumped on explicit
/// instants.
struct Rig {
    runtime: HeadlessRuntime,
    start: Instant,
    /// The instant of the last frame, in milliseconds from `start`.
    now: u64,
    /// The instant the current motion began.
    origin: u64,
    selection: Binding<i32>,
}

impl Rig {
    fn new(
        look: impl Fn(&SegmentedSpec, &Environment) -> SegmentedLook + 'static,
        direction: LayoutDirection,
        disabled: bool,
    ) -> Self {
        Self::themed(
            MinimalTestTheme::default().with_segmented(look),
            direction,
            disabled,
        )
    }

    fn themed(theme: MinimalTestTheme, direction: LayoutDirection, disabled: bool) -> Self {
        let mut rig = Self::mount(theme, direction, disabled);
        rig.settle();
        rig
    }

    /// The rig before its first frame.
    fn mount(theme: MinimalTestTheme, direction: LayoutDirection, disabled: bool) -> Self {
        let selection = Binding::i32(0);
        let view = RefCell::new(Some(AnyView::new(
            picker(
                "Mode",
                vec![text("One").tag(0), text("Two").tag(1), text("Three").tag(2)],
                &selection,
            )
            .style(PickerStyle::Segmented)
            .hide_label()
            .disabled(disabled),
        )));
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            view.borrow_mut()
                .take()
                .expect("the fixture view is built once")
        });
        let mut env = pumped_test_environment();
        env.insert(direction);
        let runtime = HeadlessRuntime::new_for_tests(env, builder, WIDTH, HEIGHT, theme)
            .with_scale_factor(SCALE);
        Self {
            runtime,
            start: Instant::now(),
            now: 0,
            origin: 0,
            selection,
        }
    }

    fn pump(&mut self, at: u64) -> crate::HeadlessPumpResult {
        assert!(at >= self.now, "frames pump forward in time");
        self.now = at;
        self.runtime
            .pump_at(true, self.start + Duration::from_millis(at))
    }

    /// Frames past every motion the looks declare.
    fn settle(&mut self) {
        for _ in 0..40 {
            self.pump(self.now + 16);
        }
        self.origin = self.now;
    }

    /// Starts a motion: a frame now, from which `sample` measures.
    fn begin(&mut self) {
        self.pump(self.now + 16);
        self.origin = self.now;
    }

    /// The frame `after` milliseconds past the motion's start.
    fn sample(&mut self, after: u64) -> HeadlessSnapshot {
        self.pump(self.origin + after)
            .into_snapshot()
            .expect("a captured frame carries pixels")
    }

    /// The one mounted live segmented picker.
    fn picker(&self) -> Rc<LiveSegmented> {
        let mut pickers = self
            .runtime
            .renderer()
            .hit_test
            .interaction
            .live_segmenteds();
        pickers.dedup_by(|a, b| Rc::ptr_eq(a, b));
        let [picker] = <[_; 1]>::try_from(pickers).expect("one picker binds one live picker");
        picker
    }

    /// Each segment's touch target, left to right in the window.
    fn targets(&self) -> Vec<Rect> {
        let mut targets: Vec<Rect> = self
            .runtime
            .renderer()
            .hit_test
            .pointer_targets
            .iter()
            .filter(|target| target.track.is_some())
            .map(|target| target.bounds)
            .collect();
        targets.sort_by(|a, b| a.x0.total_cmp(&b.x0));
        assert_eq!(targets.len(), COUNT, "every segment registers a target");
        targets
    }

    fn pointer(
        &mut self,
        event: impl FnOnce(u64, PointerKind, f32, f32) -> InputEvent,
        x: f64,
        y: f64,
    ) {
        let (x, y) = (narrow(x), narrow(y));
        self.runtime
            .push_input_event(event(POINTER, PointerKind::Mouse, x, y));
    }

    fn down(&mut self, x: f64, y: f64) {
        self.pointer(
            |id, kind, x, y| InputEvent::PointerDown {
                id,
                kind,
                x,
                y,
                button: PointerButton::Primary,
            },
            x,
            y,
        );
    }

    fn move_to(&mut self, x: f64, y: f64) {
        self.pointer(
            |id, kind, x, y| InputEvent::PointerMove { id, kind, x, y },
            x,
            y,
        );
    }

    fn up(&mut self, x: f64, y: f64) {
        self.pointer(
            |id, kind, x, y| InputEvent::PointerUp {
                id,
                kind,
                x,
                y,
                button: PointerButton::Primary,
            },
            x,
            y,
        );
    }
}

fn narrow(value: f64) -> f32 {
    crate::num_cast::f64_as_f32(value)
}

/// The device pixel a coordinate of `points` falls in.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a window coordinate in points is non-negative and far inside usize"
)]
const fn device(points: f64) -> usize {
    (points * SCALE) as usize
}

/// The pixel at `(x, y)` points.
fn pixel(snapshot: &HeadlessSnapshot, x: f64, y: f64) -> [u8; 4] {
    let offset = (device(y) * snapshot.width() as usize + device(x)) * 4;
    snapshot.rgba8()[offset..offset + 4]
        .try_into()
        .expect("a pixel is four bytes")
}

const fn is_red([r, g, b, _]: [u8; 4]) -> bool {
    r > 200 && g < 60 && b < 60
}

/// The thumb's leading edge, in points from the window's leading edge: the
/// first red pixel along a row through the top of the segment row, above
/// the labels, from the leading end.
fn thumb_edge(snapshot: &HeadlessSnapshot, row: Rect, direction: LayoutDirection) -> f64 {
    let y = row.y0 + 2.0;
    let points = |step: usize| crate::num_cast::usize_as_f64(step) / SCALE;
    let at = |step: usize| match direction {
        LayoutDirection::LeftToRight => points(step),
        LayoutDirection::RightToLeft => f64::from(WIDTH) - points(step) - 1.0 / SCALE,
    };
    let step = (0..device(f64::from(WIDTH)))
        .find(|step| is_red(pixel(snapshot, at(*step), y)))
        .expect("the thumb is drawn");
    points(step)
}

/// `value` within `tolerance` of `want`.
#[track_caller]
fn near(what: &str, value: f64, want: f64, tolerance: f64) {
    assert!(
        (value - want).abs() <= tolerance,
        "{what}: {value:.2}, expected {want:.2} (± {tolerance})"
    );
}

fn save(case: &str, stage: &str, snapshot: &HeadlessSnapshot) {
    let path = crate::testing::artifact_snapshot_path("hydrolysis", case, stage);
    std::fs::create_dir_all(path.parent().expect("the case directory"))
        .expect("the capture directory must be creatable");
    image::RgbaImage::from_raw(
        snapshot.width(),
        snapshot.height(),
        snapshot.rgba8().to_vec(),
    )
    .expect("snapshot dimensions must match the rgba buffer")
    .save(&path)
    .expect("the frame must be writable");
}

/// A selection change moves the thumb's operand along the look's value
/// motion — half way through a linear motion it is half way — and records
/// the parts no further time.
#[test]
fn a_selection_change_moves_the_thumb_along_the_motion_without_recording() {
    let mut rig = Rig::new(
        |_, _| thumb_look(linear(320)),
        LayoutDirection::LeftToRight,
        false,
    );
    let row = rig.targets()[0];
    let picker = rig.picker();
    let recordings = picker.recordings();
    let before = rig.sample(0);
    near(
        "at rest",
        thumb_edge(&before, row, LayoutDirection::LeftToRight),
        0.0,
        1.0,
    );

    rig.selection.set(2);
    rig.begin();
    let mid = rig.sample(160);
    near(
        "half way through the motion",
        thumb_edge(&mid, row, LayoutDirection::LeftToRight),
        SEGMENT,
        10.0,
    );
    rig.settle();
    let after = rig.sample(0);
    near(
        "settled",
        thumb_edge(&after, row, LayoutDirection::LeftToRight),
        2.0 * SEGMENT,
        1.0,
    );
    save("segmented_thumb_selection", "before", &before);
    save("segmented_thumb_selection", "mid", &mid);
    save("segmented_thumb_selection", "after", &after);

    assert!(
        Rc::ptr_eq(&picker, &rig.picker()),
        "the selection change kept the realization"
    );
    assert_eq!(
        picker.recordings(),
        recordings,
        "the selection change recorded the parts no further time"
    );
    assert_eq!(picker.value().selected, 2);
}

/// A drag on the selected segment carries the thumb with the touch, with no
/// animation, and a release settles it on the nearest segment, selecting it.
#[test]
fn a_drag_carries_the_thumb_and_settles_on_the_nearest_segment() {
    let mut rig = Rig::new(
        |spec, _| thumb_look(linear(15)).drag(SegmentedDrag::new(spec.direction())),
        LayoutDirection::LeftToRight,
        false,
    );
    let row = rig.targets()[0];
    let (x, y) = (row.center().x, row.center().y);
    rig.down(x, y);
    rig.move_to(x + 20.0, y);
    rig.move_to(x + 60.0, y);
    rig.pump(rig.now + 16);
    let dragged = rig.pump(rig.now + 16).into_snapshot().expect("pixels");
    near(
        "the thumb follows the touch",
        thumb_edge(&dragged, row, LayoutDirection::LeftToRight),
        60.0,
        2.0,
    );
    assert!(
        rig.picker()
            .thumb_state()
            .contains(InteractionState::DRAGGED),
        "the thumb reads as dragged"
    );

    rig.move_to(x + 130.0, y);
    rig.pump(rig.now + 16);
    let past = rig.pump(rig.now + 16).into_snapshot().expect("pixels");
    near(
        "the thumb follows the touch past a segment",
        thumb_edge(&past, row, LayoutDirection::LeftToRight),
        130.0,
        2.0,
    );

    rig.up(x + 130.0, y);
    rig.settle();
    assert_eq!(
        rig.selection.snapshot(),
        1,
        "the release at 1.3 segments selects the nearest, the second"
    );
    let settled = rig.sample(0);
    near(
        "settled on the second segment",
        thumb_edge(&settled, row, LayoutDirection::LeftToRight),
        SEGMENT,
        1.0,
    );
    assert!(
        !rig.picker()
            .thumb_state()
            .contains(InteractionState::DRAGGED),
        "the settled thumb no longer reads as dragged"
    );
    save("segmented_drag", "dragged", &dragged);
    save("segmented_drag", "past", &past);
    save("segmented_drag", "settled", &settled);
}

/// A right-to-left picker mirrors its parts about the row: the leading
/// segment is the rightmost, and the thumb resting on it sits there.
#[test]
fn a_right_to_left_picker_mirrors_its_parts() {
    let mut rig = Rig::new(
        |_, _| thumb_look(linear(15)),
        LayoutDirection::RightToLeft,
        false,
    );
    let row = rig.targets()[0];
    let rest = rig.sample(0);
    near(
        "the thumb rests on the rightmost segment",
        thumb_edge(&rest, row, LayoutDirection::RightToLeft),
        0.0,
        1.0,
    );
    rig.selection.set(1);
    rig.settle();
    let moved = rig.sample(0);
    near(
        "the second segment lies left of the first",
        thumb_edge(&moved, row, LayoutDirection::RightToLeft),
        SEGMENT,
        1.0,
    );
    save("segmented_rtl", "rest", &rest);
    save("segmented_rtl", "second", &moved);
}

/// A disabled picker composites once at the look's group opacity: a blue
/// segment part over the red thumb hides it entirely, and the group fades
/// the blue alone, so no red shows through.
#[test]
fn a_disabled_picker_composites_at_its_group_opacity() {
    let look = |_: &SegmentedSpec, _: &Environment| {
        thumb_look(linear(15))
            .segment_part(Custom::new(
                (Driven::<ShapeData, crate::theme::Segment>::new(|inputs| {
                    ShapeData::of(&inputs.frame)
                }),),
                |recorder, _frame, (segment,)| {
                    recorder.fill(segment, WorkingColor::new([0.0, 0.0, 1.0, 1.0]));
                },
            ))
            .group_opacity(Driven::new(|inputs: &Inputs<'_, ()>| {
                if inputs.state.contains(InteractionState::DISABLED) {
                    0.5
                } else {
                    1.0
                }
            }))
    };
    // A disabled picker registers no touch target; the enabled one in the
    // same layout gives the row.
    let row = Rig::new(look, LayoutDirection::LeftToRight, false).targets()[0];
    let mut rig = Rig::new(look, LayoutDirection::LeftToRight, true);
    let snapshot = rig.sample(0);
    save("segmented_disabled", "rest", &snapshot);
    let [r, g, b, _] = pixel(&snapshot, row.x0 + 4.0, row.y0 + 2.0);
    assert!(
        r.abs_diff(g) <= 3 && b > r.saturating_add(60),
        "the faded blue covers the thumb, which does not show through: ({r}, {g}, {b})"
    );
}

/// A backdrop material over the labels is recorded after them, so it
/// captures them: a lens over the whole row that saturates what it captures
/// to white erases the labels' ink.
#[test]
#[cfg(hydrolysis_gpu_painter)]
fn a_backdrop_material_over_the_labels_captures_them() {
    use super::toggle_glass::{GLASS, GLASS_SHADER, plan};
    use cherenkov_record::MaterialEffect;

    let themed = |lens: bool| {
        MinimalTestTheme {
            chrome: plan(),
            ..MinimalTestTheme::default()
        }
        .with_segmented(move |_, _| {
            let look = crate::testing::plain_segmented_look();
            if !lens {
                return look;
            }
            look.over_labels(Custom::new(
                (
                    Driven::<ShapeData, SegmentedValue>::new(|inputs| ShapeData::of(&inputs.frame)),
                    Driven::<MaterialEffect, SegmentedValue>::constant(MaterialEffect::new(vec![
                        1.0, 1.0, 1.0, 0.0,
                    ])),
                ),
                |recorder, _frame, (lens, effect)| {
                    recorder.backdrop_material(lens, GLASS_SHADER, GLASS, effect);
                },
            ))
        })
    };
    let mut plain = Rig::themed(themed(false), LayoutDirection::LeftToRight, false);
    let row = plain.targets()[0];
    let without = plain.sample(0);
    let mut lensed = Rig::themed(themed(true), LayoutDirection::LeftToRight, false);
    let with = lensed.sample(0);
    save("segmented_over_labels", "without", &without);
    save("segmented_over_labels", "with", &with);

    // The darkest pixel of the first label's ink without the lens.
    let points = |step: usize| crate::num_cast::usize_as_f64(step) / SCALE;
    let (ink, _) = (0..device(row.width()))
        .flat_map(|x| (0..device(row.height())).map(move |y| (x, y)))
        .map(|(x, y)| {
            let at = (row.x0 + points(x), row.y0 + points(y));
            (at, pixel(&without, at.0, at.1)[0])
        })
        .min_by_key(|(_, red)| *red)
        .expect("the row has pixels");
    let [r, ..] = pixel(&without, ink.0, ink.1);
    assert!(r < 128, "the label draws dark ink without the lens: {r}");
    let [r, g, b, _] = pixel(&with, ink.0, ink.1);
    assert!(
        r > 240 && g > 240 && b > 240,
        "the lens captured the label beneath it and saturated it: ({r}, {g}, {b})"
    );
}

/// The accessibility tree places each segment where its parts draw — the
/// leading one rightmost in a right-to-left picker — and a selection change
/// reaches the published tree without the picker rendering again.
#[cfg(feature = "accessibility")]
#[test]
fn accessibility_follows_the_mirrored_placement_and_the_live_selection() {
    use accesskit::Role;

    let mut rig = Rig::mount(
        MinimalTestTheme::default().with_segmented(|_, _| thumb_look(linear(15))),
        LayoutDirection::RightToLeft,
        false,
    );
    let first = rig.pump(16);
    let update = first
        .tree_update()
        .expect("the first frame publishes the tree");
    let option = |label: &str| {
        update
            .nodes
            .iter()
            .find(|(_, node)| node.role() == Role::RadioButton && node.label() == Some(label))
            .map_or_else(
                || panic!("the option {label} is published"),
                |(_, node)| node.clone(),
            )
    };
    for (label, x0) in [("One", 200.0), ("Two", 100.0), ("Three", 0.0)] {
        let bounds = option(label).bounds().expect("an option carries bounds");
        near(&format!("{label}'s leading edge"), bounds.x0, x0, 0.5);
        near(&format!("{label}'s width"), bounds.width(), SEGMENT, 0.5);
    }
    assert_eq!(option("One").is_selected(), Some(true));
    drop(first);
    rig.settle();

    let recordings = rig.picker().recordings();
    rig.selection.set(1);
    let result = rig.pump(rig.now + 16);
    let update = result
        .tree_update()
        .expect("the selection change publishes an update");
    let selected = |label: &str| {
        update
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .and_then(|(_, node)| node.is_selected())
    };
    assert_eq!(
        selected("Two"),
        Some(true),
        "the new selection is published"
    );
    assert_eq!(selected("One"), Some(false), "the old one is cleared");
    drop(result);
    assert_eq!(rig.picker().recordings(), recordings);
}
