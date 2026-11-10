//! Progress looks (water-rs/waterui#4043): a linear, circular or loading
//! indicator is a look whose parts the theme declares. A determinate
//! indicator's value moves the recorded parts' operands, and an
//! indeterminate indicator's motion is keyframes the engine samples on its
//! own clock, so neither records, lays out or builds anything per frame.

use core::f64::consts::TAU;
use core::time::Duration;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

use cherenkov_record::{Curve, Draw as _, Keyframes, Paint, Repeat, ShapeData, WorkingColor};
use kurbo::{Rect, Size};
use nami::Signal as _;
use waterui::component::progress::{Progress, progress};
use waterui::theme::ColorScheme;
use waterui::{AnyView, Binding, Color, Environment, View, ViewExt as _};
use waterui_core::handler::AnyViewBuilder;

use super::{MinimalTestTheme, pumped_test_environment, test_environment_with_scheme};
use crate::renderer::FrameWorkCounters;
use crate::theme::{Custom, Driven, Motion, ProgressControl};
use crate::{HeadlessRuntime, HeadlessSnapshot};
use waterui::animation::Animation;

const WINDOW: u32 = 320;
/// How long one cycle of the fixtures' motion lasts.
const CYCLE: Duration = Duration::from_secs(1);

fn red() -> Paint {
    Paint::from(WorkingColor::new([1.0, 0.0, 0.0, 1.0]))
}

fn blue() -> Paint {
    Paint::from(WorkingColor::new([0.0, 0.0, 1.0, 1.0]))
}

fn cyan() -> Paint {
    Paint::from(WorkingColor::new([0.0, 1.0, 1.0, 1.0]))
}

fn yellow() -> Paint {
    Paint::from(WorkingColor::new([1.0, 1.0, 0.0, 1.0]))
}

fn control(size: Size) -> ProgressControl {
    ProgressControl::new(
        size,
        Motion::uniform(Animation::linear(Duration::from_millis(15))),
    )
}

/// A bar that travels the length of its frame once a cycle.
pub(super) fn moving_bar() -> ProgressControl {
    control(Size::new(72.0, 8.0)).part(Custom::new(
        (
            Driven::<ShapeData, f64>::repeating(
                |frame| {
                    let bar = |x: f64| ShapeData::of(&Rect::new(x, frame.y0, x + 16.0, frame.y1));
                    Keyframes::new(CYCLE, bar(frame.x0)).then(
                        1.0,
                        Curve::linear(CYCLE),
                        bar(frame.x1 - 16.0),
                    )
                },
                Repeat::Forever,
            ),
            Driven::<Paint, f64>::constant(red()),
        ),
        |recorder, _frame, (bar, paint)| recorder.fill(bar, paint),
    ))
}

/// A solid bar whose colour keyframes are built from the colour scheme: red
/// in the light scheme, blue in the dark one.
pub(super) fn scheme_bar(env: &Environment) -> ProgressControl {
    control(Size::new(72.0, 8.0)).part(Custom::new(
        (Driven::<Paint, f64>::repeating_following(
            waterui::theme::current_color_scheme(env),
            |scheme: &ColorScheme, _frame| {
                let [first, second] = match scheme {
                    ColorScheme::Dark => [blue(), cyan()],
                    ColorScheme::Light => [red(), yellow()],
                };
                let linear = Curve::linear(CYCLE);
                Keyframes::new(CYCLE, first.clone())
                    .then(0.5, linear, second)
                    .then(1.0, linear, first)
            },
            Repeat::Forever,
        ),),
        |recorder, frame, (paint,)| recorder.fill(frame, paint),
    ))
}

/// A tick at the top of the frame that turns once a cycle with its layer,
/// or with two nested layers that turn half a cycle's turn each.
pub(super) fn spinner(nested: bool) -> ProgressControl {
    let turn = |angle: f64| Keyframes::new(CYCLE, 0.0).then(1.0, Curve::linear(CYCLE), angle);
    let tick = Custom::new(
        (Driven::<Paint, f64>::constant(red()),),
        |recorder, frame, (paint,)| {
            let center = frame.center().x;
            recorder.fill(
                Rect::new(center - 2.0, frame.y0, center + 2.0, frame.y0 + 10.0),
                paint,
            );
        },
    );
    control(Size::new(32.0, 32.0)).part(if nested {
        tick.rotating(turn(TAU / 2.0), Repeat::Forever)
            .rotating(turn(TAU / 2.0), Repeat::Forever)
    } else {
        tick.rotating(turn(TAU), Repeat::Forever)
    })
}

/// A square that swells and shrinks once a cycle.
pub(super) fn morphing() -> ProgressControl {
    control(Size::new(48.0, 48.0)).part(Custom::new(
        (
            Driven::<ShapeData, f64>::repeating(
                |frame| {
                    let square = |side: f64| {
                        ShapeData::of(&Rect::from_center_size(
                            frame.center(),
                            Size::new(side, side),
                        ))
                    };
                    Keyframes::new(CYCLE, square(8.0))
                        .then(0.5, Curve::linear(CYCLE), square(24.0))
                        .then(1.0, Curve::linear(CYCLE), square(8.0))
                },
                Repeat::Forever,
            ),
            Driven::<Paint, f64>::constant(red()),
        ),
        |recorder, _frame, (shape, paint)| recorder.fill(shape, paint),
    ))
}

fn runtime_with(view: impl View, theme: MinimalTestTheme) -> HeadlessRuntime {
    runtime_in(pumped_test_environment(), view, theme)
}

fn runtime_in(env: Environment, view: impl View, theme: MinimalTestTheme) -> HeadlessRuntime {
    let view = RefCell::new(Some(AnyView::new(view)));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the fixture view is built once")
    });
    HeadlessRuntime::new_for_tests(env, builder, WINDOW, WINDOW, theme)
}

/// Pumps rendering frames on a fixed 16 ms cadence.
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

    fn pump(&mut self, runtime: &mut HeadlessRuntime) -> FrameWorkCounters {
        let at = self.start + Duration::from_millis(self.next * 16);
        self.next += 1;
        runtime.pump_at(true, at).profile().counters().frame_work()
    }

    /// The next frame's pixels.
    fn snapshot(&mut self, runtime: &mut HeadlessRuntime) -> HeadlessSnapshot {
        let at = self.start + Duration::from_millis(self.next * 16);
        self.next += 1;
        runtime
            .pump_at(true, at)
            .into_snapshot()
            .expect("a captured frame carries pixels")
    }

    /// Everything `count` frames do to the retained tree and the scene.
    fn work(&mut self, runtime: &mut HeadlessRuntime, count: usize) -> [u64; 8] {
        (0..count).fold([0; 8], |sum, _| {
            let frame = self.pump(runtime);
            let frame = [
                frame.semantic_builds,
                frame.structural_patches,
                frame.measure_calls,
                frame.layout_calls,
                frame.recorded_view_contents,
                frame.live_recordings,
                frame.layer_creations,
                frame.layer_removals,
            ];
            core::array::from_fn(|index| sum[index] + frame[index])
        })
    }
}

/// The indicator `view` under a theme whose indeterminate looks move, once
/// the scene has settled.
fn settled(view: impl View) -> (HeadlessRuntime, Frames) {
    let theme = MinimalTestTheme {
        moving_progress: true,
        ..Default::default()
    };
    let mut runtime = runtime_with(view, theme);
    let mut frames = Frames::new();
    for _ in 0..4 {
        frames.pump(&mut runtime);
    }
    (runtime, frames)
}

/// A running indeterminate indicator is the engine's: it records nothing,
/// walks no tree and lays nothing out across many frames, yet its image
/// changes with time.
fn assert_runs_on_the_engine(view: impl View) {
    let (mut runtime, mut frames) = settled(view);
    let before = frames.snapshot(&mut runtime);
    let work = frames.work(&mut runtime, 120);
    assert_eq!(
        work, [0; 8],
        "a running indicator does no frame work: builds, patches, measures, layouts, \
         content recordings, live recordings, layer creations and removals"
    );
    let after = frames.snapshot(&mut runtime);
    assert_ne!(
        before.rgba8(),
        after.rgba8(),
        "the indicator moves while it does no work"
    );
}

#[test]
fn indeterminate_linear_progress_runs_its_keyframes_without_frame_work() {
    assert_runs_on_the_engine(Progress::infinity());
}

#[test]
fn indeterminate_circular_progress_runs_its_keyframes_without_frame_work() {
    assert_runs_on_the_engine(Progress::infinity().circular());
}

#[test]
fn loading_indicator_runs_its_keyframes_without_frame_work() {
    assert_runs_on_the_engine(waterui::component::progress::loading());
}

/// Rotation stages nest: a tick turned by two layers that each turn half a
/// cycle's turn is where one layer turning a whole one puts it, and the
/// nest does no frame work while it runs.
#[test]
fn nested_rotation_stages_sum() {
    let frames_at = |nested: bool| {
        let theme = MinimalTestTheme {
            moving_progress: true,
            nested_progress: nested,
            ..Default::default()
        };
        let mut runtime = runtime_with(Progress::infinity().circular(), theme);
        let mut frames = Frames::new();
        for _ in 0..4 {
            frames.pump(&mut runtime);
        }
        let work = frames.work(&mut runtime, 30);
        assert_eq!(work, [0; 8], "a nest of rotation layers does no frame work");
        let snapshot = frames.snapshot(&mut runtime);
        (snapshot, work)
    };
    let (single, _) = frames_at(false);
    let (nested, _) = frames_at(true);
    let worst = single
        .rgba8()
        .iter()
        .zip(nested.rgba8())
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .expect("a snapshot has pixels");
    assert!(
        worst <= 24,
        "the nested tick is where the single one is: worst channel difference {worst}"
    );
}

/// The colour of the fixture's solid bar: the one colour the window shows
/// besides its own background, which covers most of it.
fn bar_color(snapshot: &HeadlessSnapshot) -> [u8; 4] {
    let mut counts = std::collections::BTreeMap::new();
    for pixel in snapshot.rgba8().as_chunks::<4>().0 {
        *counts.entry(*pixel).or_insert(0_usize) += 1;
    }
    let background = *counts
        .iter()
        .max_by_key(|(_, count)| **count)
        .expect("a window has pixels")
        .0;
    counts.remove(&background);
    assert_eq!(counts.len(), 1, "the bar is one solid colour: {counts:?}");
    counts.into_keys().next().expect("one colour")
}

/// A running track built from a signal follows it: flipping the colour
/// scheme under a running indicator changes the colours it runs, and
/// records, builds and mounts nothing to do so.
#[test]
fn a_running_track_follows_the_signal_it_is_built_from() {
    let scheme = Binding::container(ColorScheme::Light);
    let theme = MinimalTestTheme {
        scheme_progress: true,
        ..Default::default()
    };
    let mut runtime = runtime_in(
        test_environment_with_scheme(scheme.clone()),
        Progress::infinity().label(""),
        theme,
    );
    let mut frames = Frames::new();
    for _ in 0..4 {
        frames.pump(&mut runtime);
    }
    let [red, _, blue, _] = bar_color(&frames.snapshot(&mut runtime));
    assert!(red > 200 && blue < 100, "the light bar is red");

    scheme.set(ColorScheme::Dark);
    let work = frames.work(&mut runtime, 8);
    assert_eq!(
        [work[0], work[1], work[5], work[6], work[7]],
        [0; 5],
        "the flip builds, patches, records and mounts nothing"
    );
    let [red, _, blue, _] = bar_color(&frames.snapshot(&mut runtime));
    assert!(blue > 200 && red < 100, "the dark bar is blue");

    // The new colours keep running: the track started again rather than
    // the operand holding the new first colour.
    let before = bar_color(&frames.snapshot(&mut runtime));
    frames.work(&mut runtime, 20);
    let after = bar_color(&frames.snapshot(&mut runtime));
    assert_ne!(before, after, "the dark colours cycle");
    assert!(
        after[2] > 200 && after[0] < 100,
        "and stay the dark scheme's"
    );
}

/// A value change moves the parts' operands: nothing is recorded again.
#[test]
fn progress_value_change_records_nothing() {
    let realizations = Rc::new(RefCell::new(Vec::new()));
    let recordings = Rc::new(Cell::new(0));
    let theme = MinimalTestTheme {
        progress_bar_realizations: Rc::clone(&realizations),
        progress_bar_recordings: Rc::clone(&recordings),
        ..Default::default()
    };
    let value = Binding::container(0.2_f64);
    let mut runtime = runtime_with(progress(value.clone()), theme);
    let mut frames = Frames::new();
    for _ in 0..4 {
        frames.pump(&mut runtime);
    }
    assert_eq!(recordings.get(), 1, "the bar records once, when it mounts");

    value.set(0.9);
    // The value label's text and the node's accessibility value follow the
    // value, so their own views lay out again; the bar is not among them.
    let work = frames.work(&mut runtime, 8);
    assert_eq!(
        recordings.get(),
        1,
        "a value change does not record the bar"
    );
    assert_eq!(
        [work[0], work[1], work[6], work[7]],
        [0, 0, 0, 0],
        "nor builds or patches the tree, nor mounts or removes a layer"
    );
    let (_, newest) = *realizations
        .borrow()
        .last()
        .expect("the look's value was read");
    assert!(
        (newest - 0.9).abs() < f64::EPSILON,
        "the part's operand retargeted to the new value, got {newest}"
    );
}

/// A tinted subtree's indicator is requested with that tint.
#[test]
fn progress_spec_carries_the_subtrees_tint() {
    let specs = Rc::new(RefCell::new(Vec::new()));
    let theme = MinimalTestTheme {
        progress_specs: Rc::clone(&specs),
        ..Default::default()
    };
    let tint = Color::srgb_f32(0.9, 0.1, 0.2);
    let mut runtime = runtime_with(
        progress(Binding::container(0.5_f64)).tint(tint.clone()),
        theme,
    );
    Frames::new().pump(&mut runtime);

    let env = pumped_test_environment();
    let specs = specs.borrow();
    let spec = specs.last().expect("the theme was asked for a look");
    assert_eq!(
        spec.tint().resolve(&env).snapshot(),
        tint.resolve(&env).snapshot()
    );
}

/// What the indicator measures is what it draws: a label band is reserved
/// only for a label the indicator has.
#[cfg(feature = "accessibility")]
mod measured_equals_drawn {
    use waterui_layout::stack::vstack;

    use super::*;

    /// The bounds the indicator is laid out in, which its measured size
    /// sets, and its bar's frame.
    fn laid_out(indicator: impl View) -> (Rect, Rect) {
        let view = vstack((indicator,));
        let realizations = Rc::new(RefCell::new(Vec::new()));
        let theme = MinimalTestTheme {
            progress_bar_realizations: Rc::clone(&realizations),
            ..Default::default()
        };
        let mut runtime = runtime_with(view, theme);
        let mut frames = Frames::new();
        for _ in 0..4 {
            frames.pump(&mut runtime);
        }
        let bounds = runtime
            .accessibility_tree()
            .and_then(|tree| {
                tree.nodes
                    .iter()
                    .find(|(_, node)| node.role() == accesskit::Role::ProgressIndicator)
                    .and_then(|(_, node)| node.bounds())
            })
            .expect("the indicator emits a node with bounds");
        let (bar, _) = *realizations
            .borrow()
            .last()
            .expect("the look realized its bar");
        (Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y1), bar)
    }

    #[test]
    fn a_progress_without_labels_reserves_no_band() {
        let (bounds, bar) = laid_out(Progress::infinity().label(""));
        assert!(
            bar.y0.abs() < 1e-3,
            "no label, no band above the bar: bar at {bar:?} in {bounds:?}"
        );
        assert!(
            (bounds.height() - bar.y1).abs() < 1e-3,
            "no value label, no band below the bar: bar at {bar:?} in {bounds:?}"
        );
    }

    #[test]
    fn a_progress_with_a_label_reserves_exactly_its_band() {
        let (bounds, bar) = laid_out(Progress::infinity().label("Syncing"));
        assert!(
            bar.y0 >= 18.0 + 10.0 - 1e-3,
            "the label band and its spacing lie above the bar: bar at {bar:?} in {bounds:?}"
        );
        assert!(
            (bounds.height() - bar.y1).abs() < 1e-3,
            "no value label, no band below the bar: bar at {bar:?} in {bounds:?}"
        );
    }

    #[test]
    fn a_determinate_progress_reserves_its_value_label_band() {
        let (bounds, bar) = laid_out(progress(Binding::container(0.5_f64)).label(""));
        assert!(
            bar.y0.abs() < 1e-3,
            "no label, no band above the bar: bar at {bar:?} in {bounds:?}"
        );
        assert!(
            bounds.height() - bar.y1 >= 6.0 - 1e-3,
            "the value label and its spacing lie below the bar: bar at {bar:?} in {bounds:?}"
        );
    }
}
