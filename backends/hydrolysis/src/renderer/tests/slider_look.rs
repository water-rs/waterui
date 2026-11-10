//! The slider's declarative look: the spec the theme is asked for, the drag
//! mappings the renderer executes, the measure the layout and the drawing
//! share, and the guarantee that dragging and value changes record nothing.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use kurbo::Rect;
use waterui::{Binding, Color, Environment, Str, ViewExt as _};
use waterui_controls::ControlSize;
use waterui_controls::slider::slider;
use waterui_core::AnyView;
use waterui_core::Signal as _;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::layout::LayoutDirection;

use super::{MinimalTestTheme, capture_root_window, pumped_test_environment, test_environment};
use crate::HeadlessRuntime;
use crate::num_cast::{f64_as_f32, i32_as_f32, u32_as_f32};
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::renderer::chrome::LiveChrome;
use crate::renderer::{HydroState, measure_view_intrinsic, normalize_layout_view};
use crate::text::{FontFamilyResolution, ParleyEngine};
use crate::theme::{DragMapping, WidgetTheme};

/// The slider's current value.
trait Current {
    fn get(&self) -> f64;
}

impl Current for Binding<f64> {
    fn get(&self) -> f64 {
        self.snapshot()
    }
}

const POINTER_ID: u64 = 7;
const WINDOW_WIDTH: u32 = 800;
const WINDOW_HEIGHT: u32 = 600;
/// The track's extent in a 800pt-wide window under the fixture look: the
/// 12pt inset the knob needs at each end.
const TRACK: (f64, f64) = (12.0, 788.0);

/// A runtime mounting `view` under `theme` in `env`, with its first frame
/// pumped.
fn runtime_with(view: AnyView, theme: MinimalTestTheme, env: Environment) -> HeadlessRuntime {
    let view = RefCell::new(Some(view));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the test view is built once")
    });
    let mut runtime =
        HeadlessRuntime::new_for_tests(env, builder, WINDOW_WIDTH, WINDOW_HEIGHT, theme);
    let _ = runtime.pump(false);
    runtime
}

/// Drives a runtime on a fixed 16 ms cadence.
struct Frames {
    start: Instant,
    next: u64,
    /// The tree work the frames pumped so far report together:
    /// `[live recordings, recorded view contents, layout calls]`.
    work: [u64; 3],
}

impl Frames {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            next: 0,
            work: [0; 3],
        }
    }

    fn pump(&mut self, runtime: &mut HeadlessRuntime) {
        let at = self.start + Duration::from_millis(self.next * 16);
        self.next += 1;
        let counters = runtime.pump_at(true, at).profile().counters().frame_work();
        self.work[0] += counters.live_recordings;
        self.work[1] += counters.recorded_view_contents;
        self.work[2] += counters.layout_calls;
    }

    fn settle(&mut self, runtime: &mut HeadlessRuntime) {
        for _ in 0..8 {
            self.pump(runtime);
        }
    }

    fn down(&mut self, runtime: &mut HeadlessRuntime, x: f32, y: f32) {
        runtime.push_input_event(InputEvent::PointerDown {
            id: POINTER_ID,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        });
        self.pump(runtime);
    }

    fn drag(&mut self, runtime: &mut HeadlessRuntime, x: f32, y: f32) {
        runtime.push_input_event(InputEvent::PointerMove {
            id: POINTER_ID,
            kind: PointerKind::Mouse,
            x,
            y,
        });
        self.pump(runtime);
    }

    fn up(&mut self, runtime: &mut HeadlessRuntime, x: f32, y: f32) {
        runtime.push_input_event(InputEvent::PointerUp {
            id: POINTER_ID,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        });
        self.pump(runtime);
    }
}

/// The y of the slider's row.
fn row_y(runtime: &HeadlessRuntime) -> f32 {
    let targets = &runtime.renderer().hit_test.pointer_targets;
    let [target] = targets.as_slice() else {
        panic!("one slider takes one pointer target, got {}", targets.len());
    };
    f64_as_f32(target.bounds.center().y)
}

/// The x of the normalized `value` on the fixture's track.
fn track_x(value: f64) -> f32 {
    f64_as_f32(value.mul_add(TRACK.1 - TRACK.0, TRACK.0))
}

fn the_chrome(runtime: &HeadlessRuntime) -> Rc<LiveChrome> {
    let chromes = runtime.renderer().hit_test.interaction.live_chromes();
    let [chrome] = <[_; 1]>::try_from(chromes).expect("one slider binds one live chrome");
    chrome
}

/// The sliders the tests drag: 0 to 100, no label.
fn volume(value: &Binding<f64>) -> AnyView {
    AnyView::new(slider("Volume", value).hide_label().range(0.0..=100.0))
}

/// A press and a drag set the value from the pointer's position along the
/// track, and neither records anything: the look's parts follow the value
/// through their operands, so the chrome and the track part are recorded once,
/// at mount.
#[test]
fn dragging_and_value_changes_record_nothing() {
    let tracks = Rc::new(RefCell::new(Vec::new()));
    let theme = MinimalTestTheme {
        slider_track_draws: Rc::clone(&tracks),
        ..Default::default()
    };
    let value = Binding::container(25.0);
    let mut runtime = runtime_with(volume(&value), theme, pumped_test_environment());
    let mut frames = Frames::new();
    frames.settle(&mut runtime);

    let chrome = the_chrome(&runtime);
    let recordings = chrome.recordings();
    let track_records = tracks.borrow().len();
    assert_eq!(track_records, 1, "the track part records once at mount");
    let y = row_y(&runtime);
    frames.work = [0; 3];

    frames.down(&mut runtime, track_x(0.25), y);
    for step in 1..=10 {
        frames.drag(
            &mut runtime,
            40.0f32.mul_add(i32_as_f32(step), track_x(0.25)),
            y,
        );
    }
    assert!(
        value.get() > 70.0,
        "the drag moved the value, got {}",
        value.get()
    );
    frames.up(&mut runtime, track_x(0.25) + 400.0, y);

    // A change from outside, as a keyboard or a model would make it.
    value.set(10.0);
    frames.settle(&mut runtime);
    value.set(90.0);
    frames.settle(&mut runtime);

    assert_eq!(
        chrome.recordings(),
        recordings,
        "a drag and a value change record no chrome"
    );
    assert_eq!(
        tracks.borrow().len(),
        track_records,
        "nor does either re-record the look's parts"
    );
    assert_eq!(
        frames.work,
        [0, 0, 0],
        "nor does either record a live source, a view's content or lay anything out"
    );
}

/// `Absolute`: the value is the pointer's position along the track, so a
/// press anywhere on the slider moves the value there.
#[test]
fn absolute_drag_follows_the_pointer() {
    let value = Binding::container(25.0);
    let mut runtime = runtime_with(
        volume(&value),
        MinimalTestTheme::default(),
        pumped_test_environment(),
    );
    let mut frames = Frames::new();
    frames.settle(&mut runtime);
    let y = row_y(&runtime);

    frames.down(&mut runtime, track_x(0.5), y);
    assert!(
        (value.get() - 50.0).abs() < 0.5,
        "a press moves the value to the pointer, got {}",
        value.get()
    );
    frames.drag(&mut runtime, 500.0, y);
    let expected = (500.0 - TRACK.0) / (TRACK.1 - TRACK.0) * 100.0;
    assert!(
        (value.get() - expected).abs() < 0.5,
        "a drag tracks the pointer to {expected}, got {}",
        value.get()
    );
    frames.up(&mut runtime, 500.0, y);
}

/// `Relative { knob_only: false }`: a press moves nothing, and a drag moves
/// the value with the pointer's travel from where it landed.
#[test]
fn relative_drag_moves_with_the_pointers_travel() {
    let value = Binding::container(25.0);
    let theme = MinimalTestTheme {
        slider_drag: Some(DragMapping::Relative { knob_only: false }),
        ..Default::default()
    };
    let mut runtime = runtime_with(volume(&value), theme, pumped_test_environment());
    let mut frames = Frames::new();
    frames.settle(&mut runtime);
    let y = row_y(&runtime);

    frames.down(&mut runtime, 400.0, y);
    assert!(
        (value.get() - 25.0).abs() < f64::EPSILON,
        "a press moves nothing by itself, got {}",
        value.get()
    );
    frames.drag(&mut runtime, 500.0, y);
    let expected = (100.0 / (TRACK.1 - TRACK.0)).mul_add(100.0, 25.0);
    assert!(
        (value.get() - expected).abs() < 0.5,
        "100pt of travel moves the value by {}, got {}",
        expected - 25.0,
        value.get()
    );
    frames.up(&mut runtime, 500.0, y);
}

/// `Relative { knob_only: true }`: only a press on the knob's target drags.
#[test]
fn knob_only_drag_ignores_a_press_off_the_knob() {
    let theme = || MinimalTestTheme {
        slider_drag: Some(DragMapping::Relative { knob_only: true }),
        ..Default::default()
    };

    let value = Binding::container(25.0);
    let mut runtime = runtime_with(volume(&value), theme(), pumped_test_environment());
    let mut frames = Frames::new();
    frames.settle(&mut runtime);
    let y = row_y(&runtime);
    frames.down(&mut runtime, 600.0, y);
    frames.drag(&mut runtime, 700.0, y);
    frames.up(&mut runtime, 700.0, y);
    assert!(
        (value.get() - 25.0).abs() < f64::EPSILON,
        "a press off the knob drags nothing, got {}",
        value.get()
    );

    let value = Binding::container(25.0);
    let mut runtime = runtime_with(volume(&value), theme(), pumped_test_environment());
    let mut frames = Frames::new();
    frames.settle(&mut runtime);
    let y = row_y(&runtime);
    frames.down(&mut runtime, track_x(0.25), y);
    frames.drag(&mut runtime, track_x(0.25) + 100.0, y);
    frames.up(&mut runtime, track_x(0.25) + 100.0, y);
    assert!(
        value.get() > 35.0,
        "a press on the knob drags it, got {}",
        value.get()
    );
}

/// A drag rounds the value to the step, anchored at the range's start.
#[test]
fn drag_rounds_to_the_step() {
    let value = Binding::container(0.0);
    let view = AnyView::new(
        slider("Volume", &value)
            .hide_label()
            .range(0.0..=100.0)
            .step(10.0),
    );
    let mut runtime = runtime_with(view, MinimalTestTheme::default(), pumped_test_environment());
    let mut frames = Frames::new();
    frames.settle(&mut runtime);
    let y = row_y(&runtime);

    frames.down(&mut runtime, track_x(0.0), y);
    frames.drag(&mut runtime, 500.0, y);
    assert!(
        (value.get() - 60.0).abs() < f64::EPSILON,
        "62.9 rounds to the step at 60, got {}",
        value.get()
    );
    frames.up(&mut runtime, 500.0, y);
}

/// In a right-to-left layout the minimum is on the right: the same pointer
/// position sets the mirrored value.
#[test]
fn drag_mirrors_in_a_right_to_left_layout() {
    let mut env = pumped_test_environment();
    env.insert(LayoutDirection::RightToLeft);
    let value = Binding::container(50.0);
    let mut runtime = runtime_with(volume(&value), MinimalTestTheme::default(), env);
    let mut frames = Frames::new();
    frames.settle(&mut runtime);
    let y = row_y(&runtime);

    frames.down(&mut runtime, track_x(0.25), y);
    assert!(
        (value.get() - 75.0).abs() < 0.5,
        "a quarter in from the left is three quarters toward the maximum, got {}",
        value.get()
    );
    frames.up(&mut runtime, track_x(0.25), y);
}

/// The slider's measure and its drawing read the same look: laid out at its
/// intrinsic size, the track is exactly the look's minimum, the knob's row
/// ends at the bottom, and the label band lies above it.
#[test]
fn measurement_equals_drawing() {
    let env = test_environment();
    let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme::default());
    let view = || {
        slider("Volume", &Binding::container(0.5))
            .range(0.0..=1.0)
            .min_value_label("Quiet")
            .max_value_label("Loud")
    };
    let mut state = HydroState::new(ParleyEngine::system(FontFamilyResolution::Strict));
    let size = measure_view_intrinsic(
        &normalize_layout_view(AnyView::new(view()), &env),
        &mut state,
        &env,
        &theme,
    );

    let tracks = Rc::new(RefCell::new(Vec::new()));
    let mut renderer = super::test_renderer_with_theme(MinimalTestTheme {
        slider_track_draws: Rc::clone(&tracks),
        ..Default::default()
    });
    let bounds = Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height));
    capture_root_window(&mut renderer, view(), &env, bounds);

    let track = tracks.borrow()[0];
    assert!(
        approx::relative_eq!(track.width(), 72.0, epsilon = 0.01),
        "the track takes exactly the room the measure left it, got {track:?}"
    );
    assert!(
        approx::relative_eq!(track.y1, bounds.y1, epsilon = 0.01),
        "the row ends at the bottom of the measured height, got {track:?} in {bounds:?}"
    );
    assert!(
        approx::relative_eq!(track.height(), 44.0, epsilon = 0.01),
        "the row is the look's row height, got {track:?}"
    );
    assert!(
        track.y0 > 6.0,
        "the label band lies above the row, got {track:?}"
    );
}

/// The label band is reserved only when the look draws it.
#[test]
fn no_band_is_reserved_for_a_label_the_look_does_not_draw() {
    let env = test_environment();
    let measure = |without_label: bool| {
        let theme: Rc<dyn WidgetTheme> = Rc::new(MinimalTestTheme {
            slider_without_label: without_label,
            ..Default::default()
        });
        let mut state = HydroState::new(ParleyEngine::system(FontFamilyResolution::Strict));
        measure_view_intrinsic(
            &normalize_layout_view(
                AnyView::new(slider("Volume", &Binding::container(0.5)).range(0.0..=1.0)),
                &env,
            ),
            &mut state,
            &env,
            &theme,
        )
    };
    let drawn = measure(false);
    let undrawn = measure(true);
    assert!(
        approx::relative_eq!(undrawn.height, 44.0, epsilon = 0.01),
        "without a drawn label the slider is its row, got {undrawn:?}"
    );
    assert!(
        drawn.height > undrawn.height + 6.0,
        "a drawn label adds its height and the spacing, got {drawn:?} against {undrawn:?}"
    );
}

/// A slider's spec carries what the look is built from: its size, range,
/// step, direction, label presence and its subtree's tint.
#[test]
fn the_spec_carries_the_tint_and_the_structure() {
    let specs = Rc::new(RefCell::new(Vec::new()));
    let env = test_environment();
    let mut renderer = super::test_renderer_with_theme(MinimalTestTheme {
        slider_specs: Rc::clone(&specs),
        ..Default::default()
    });
    let tint = Color::srgb(200, 30, 30);
    capture_root_window(
        &mut renderer,
        slider("Volume", &Binding::container(5.0))
            .range(0.0..=10.0)
            .step(1.0)
            .size(ControlSize::Large)
            .value_indicator(|value| Str::from(format!("{value:.0}")))
            .tint(tint.clone()),
        &env,
        Rect::new(0.0, 0.0, 400.0, 100.0),
    );

    let specs = specs.borrow();
    let spec = specs.first().expect("the theme was asked for the look");
    assert_eq!(spec.size(), ControlSize::Large);
    assert_eq!(*spec.range(), 0.0..=10.0);
    assert_eq!(spec.step(), Some(1.0));
    assert_eq!(spec.direction(), LayoutDirection::LeftToRight);
    assert!(spec.has_label());
    assert!(spec.has_value_indicator());
    assert!(!spec.has_value_labels());
    assert_eq!(
        spec.tint().resolve(&env).snapshot(),
        tint.resolve(&env).snapshot(),
        "the spec carries the subtree's tint"
    );
}

/// A press off a knob-only slider's knob reaches what lies beneath it, as a
/// scroll view's drag must; a press on the knob does not.
#[test]
fn knob_only_press_off_the_knob_reaches_the_target_beneath() {
    use std::cell::Cell;
    use waterui::ViewExt as _;
    use waterui_controls::button::button;
    use waterui_layout::stack::zstack;

    let taps = Rc::new(Cell::new(0_u32));
    let value = Binding::container(25.0);
    let beneath = {
        let taps = Rc::clone(&taps);
        AnyView::new(button("Beneath").action(move || taps.set(taps.get() + 1)))
            .size(u32_as_f32(WINDOW_WIDTH), u32_as_f32(WINDOW_HEIGHT))
    };
    let view = AnyView::new(zstack((beneath, volume(&value))));
    let theme = MinimalTestTheme {
        slider_drag: Some(DragMapping::Relative { knob_only: true }),
        ..Default::default()
    };
    let mut runtime = runtime_with(view, theme, pumped_test_environment());
    let mut frames = Frames::new();
    frames.settle(&mut runtime);
    // The slider's target is the wide track row; the button's is its own
    // label-sized box, centred and overlapping the row away from the knob.
    let button = runtime
        .renderer()
        .hit_test
        .pointer_targets
        .iter()
        .map(|target| target.bounds)
        .min_by(|left, right| left.width().total_cmp(&right.width()))
        .expect("the button registers a target")
        .center();
    let (x, y) = (f64_as_f32(button.x), f64_as_f32(button.y));

    frames.down(&mut runtime, x, y);
    frames.up(&mut runtime, x, y);
    assert_eq!(
        taps.get(),
        1,
        "a press off the knob reaches the target beneath"
    );
    assert!(
        (value.get() - 25.0).abs() < f64::EPSILON,
        "and the slider drags nothing, got {}",
        value.get()
    );

    frames.down(&mut runtime, track_x(0.25), y);
    frames.up(&mut runtime, track_x(0.25), y);
    assert_eq!(taps.get(), 1, "a press on the knob is the slider's");
}

/// The accessibility tree follows an external value change although nothing
/// re-rendered the slider: the change comes after an unrelated re-flush.
#[cfg(feature = "accessibility")]
#[test]
fn external_value_change_reaches_the_accessibility_tree() {
    use accesskit::Role;
    use waterui::component::text;
    use waterui_core::Str;
    use waterui_layout::stack::vstack;

    let value = Binding::container(25.0);
    let other = Binding::container(Str::from("before"));
    let view = AnyView::new(vstack((volume(&value), text(other.clone()))));
    let mut runtime = runtime_with(view, MinimalTestTheme::default(), pumped_test_environment());
    let mut frames = Frames::new();
    frames.settle(&mut runtime);

    let slider_value = |update: &accesskit::TreeUpdate| {
        update
            .nodes
            .iter()
            .find(|(_, node)| node.role() == Role::Slider)
            .and_then(|(_, node)| node.numeric_value())
    };
    let tree_after = |frames: &mut Frames, runtime: &mut HeadlessRuntime| {
        let at = frames.start + Duration::from_millis(frames.next * 16);
        frames.next += 1;
        runtime
            .pump_at(true, at)
            .into_tree_update()
            .expect("the frame published a tree update")
    };

    // An unrelated change re-flushes the window without re-rendering the slider.
    other.set(Str::from("after"));
    let update = tree_after(&mut frames, &mut runtime);
    assert_eq!(slider_value(&update), Some(25.0));

    value.set(80.0);
    let update = tree_after(&mut frames, &mut runtime);
    assert_eq!(
        slider_value(&update),
        Some(80.0),
        "the tree follows the binding without a re-render"
    );
}
