//! The switch of water-rs/hydrolysis-glass specification §8.2 under a test
//! theme (water-rs/waterui#4038): two operands of one switch on different
//! springs, the track fade's direction-aware spring, a press lens over a
//! backdrop stage, a drag, a group opacity, the tint and a right-to-left
//! layout.
//!
//! The look below states §8.2's geometry, colours and motion with the
//! toggle looks' vocabulary. The tests sample the rendered switch at the
//! instants the specification gives and compare with its numbers, the knob's
//! position from its left edge in the pixels and the track fade from the
//! track's colour between its two settled ends.

use core::time::Duration;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use cherenkov_record::{
    BackdropShaderSource, CaptureClass, CaptureLevels, CaptureScale, Draw as _, MaterialCapture,
    MaterialGrouping, MaterialShader,
};
use cherenkov_record::{MaterialEffect, Paint, ShapeData, WorkingColor};
use kurbo::{Rect, RoundedRect, Size};
use nami::{Binding, Signal as _};
use waterui::ViewExt as _;
use waterui::animation::Animation;
use waterui::graphics::Color;
use waterui_controls::toggle::toggle;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::interaction::InteractionState;
use waterui_core::layout::LayoutDirection;
use waterui_core::{AnyView, Environment};

use super::chrome_primitives::tree_work;
use super::{ChromePlan, MinimalTestTheme, pumped_test_environment};
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::theme::{
    Backdrop, Custom, Driven, HitRegion, Inputs, Motion, SwitchDrag, SwitchLook, ToggleBackdrop,
    ToggleControl, ToggleSpec, Transition, ValueMotion,
};
use crate::{HeadlessRuntime, HeadlessSnapshot};

/// The window, in points; `SCALE` device pixels per point.
const WIDTH: u32 = 200;
const HEIGHT: u32 = 80;
const SCALE: f64 = 4.0;
const POINTER: u64 = 1;

/// §8.2's geometry, in points.
const FRAME: Size = Size::new(61.0, 28.0);
const TRACK: Size = Size::new(63.0, 28.0);
const KNOB: Size = Size::new(37.0, 24.0);
const INSET: f64 = 2.0;
const TRAVEL: f64 = 22.0;
/// §8.2's lens growth: the knob scales by `1 + 0.35·p`.
const LENS_GROWTH: f64 = 0.35;

const GLASS_SHADER: MaterialShader = MaterialShader::new(0);
const GLASS: CaptureClass = CaptureClass::new(0);
/// `backdrop_sample + params[0]`: an effect that is visible in the pixels.
const TINT_WGSL: &str = "fn backdrop_effect(px: BackdropPixel, params: array<vec4<f32>, 16>) -> vec4<f32> { return backdrop_sample(px.p) + params[0]; }\n";

/// A critically damped spring of natural frequency `omega` on a unit mass.
fn critical(omega: f32) -> Animation {
    damped(omega, 1.0)
}

/// A spring of natural frequency `omega` and damping ratio `zeta` on a unit
/// mass: stiffness `ω0²`, damping `2ζω0`.
fn damped(omega: f32, zeta: f32) -> Animation {
    Animation::spring(omega * omega, 2.0 * zeta * omega)
}

/// §8.2's motion: the toggle spring owns every value change, the press
/// and release springs the press.
fn motion() -> Motion {
    Motion {
        hover: Transition::symmetric(critical(21.0)),
        focus: Transition::symmetric(critical(21.0)),
        press: Transition::new(damped(28.2, 0.65), damped(15.8, 0.385)),
        drag: Transition::symmetric(critical(21.0)),
        value: ValueMotion::Own(critical(21.0)),
    }
}

/// The track of a switch laid out `direction` on `frame`: it begins at the
/// frame's leading edge and overhangs its trailing edge.
fn track_of(frame: Rect, direction: LayoutDirection) -> Rect {
    let x0 = match direction {
        LayoutDirection::LeftToRight => frame.x0,
        LayoutDirection::RightToLeft => frame.x1 - TRACK.width,
    };
    Rect::new(x0, frame.y0, x0 + TRACK.width, frame.y0 + TRACK.height)
}

/// The knob at rest on the track, growing about its centre while pressed.
fn knob_of(drag: SwitchDrag, direction: LayoutDirection, inputs: &Inputs<'_, bool>) -> RoundedRect {
    let q = drag.progress(*inputs.value, inputs.drag);
    let track = track_of(inputs.frame, direction);
    let x0 = match direction {
        LayoutDirection::LeftToRight => q.mul_add(TRAVEL, track.x0 + INSET),
        LayoutDirection::RightToLeft => (-q).mul_add(TRAVEL, track.x1 - INSET - KNOB.width),
    };
    let rest = Rect::new(
        x0,
        track.y0 + INSET,
        x0 + KNOB.width,
        track.y0 + INSET + KNOB.height,
    );
    let grown = if inputs.state.contains(InteractionState::PRESSED) {
        1.0 + LENS_GROWTH
    } else {
        1.0
    };
    let lens = Rect::from_center_size(rest.center(), rest.size() * grown);
    RoundedRect::from_rect(lens, lens.height() / 2.0)
}

/// §8.2's switch: flat track and knob at rest, the on fill drawn over the off
/// fill at the fade's opacity, the knob a glass lens while pressed, the whole
/// switch at half opacity when disabled.
fn glass_switch(spec: &ToggleSpec, env: &Environment) -> SwitchLook {
    let direction = spec.direction();
    let drag = SwitchDrag::new(TRAVEL, direction);
    let tint = spec.tint().resolve(env);
    let off = Color::srgb(60, 60, 67).resolve(env).snapshot();
    let off = Paint::from(off.with_alpha(0.30));

    let fade_in = ValueMotion::Own(critical(11.2));
    let fade_out = ValueMotion::Own(critical(20.2));
    let fade = Driven::follow(
        tint,
        move |tint: &WorkingColor, inputs: &Inputs<'_, bool>| {
            let c = crate::num_cast::f64_as_f32(drag.progress(*inputs.value, inputs.drag));
            Paint::from(tint.with_alpha(tint.components[3] * c))
        },
    )
    .value_motion(move |on: &bool| {
        if *on {
            fade_in.clone()
        } else {
            fade_out.clone()
        }
    });
    let track = Custom::new(
        (Driven::constant(off), fade),
        move |recorder, frame, (off, fade)| {
            let track = track_of(frame, direction);
            let capsule = RoundedRect::from_rect(track, track.height() / 2.0);
            recorder.fill(capsule, off);
            recorder.fill(capsule, fade);
        },
    );

    let lens = Driven::new(move |inputs: &Inputs<'_, bool>| {
        ShapeData::of(&knob_of(drag, direction, inputs))
    });
    let tinted = Driven::new(|inputs: &Inputs<'_, ()>| {
        let pressed = inputs.state.contains(InteractionState::PRESSED);
        MaterialEffect::new(vec![if pressed { -0.5 } else { 0.0 }, 0.0, 0.0, 0.0])
    });

    let white = Driven::new(|inputs: &Inputs<'_, bool>| {
        let pressed = inputs.state.contains(InteractionState::PRESSED);
        Paint::from(WorkingColor::new([
            1.0,
            1.0,
            1.0,
            if pressed { 0.0 } else { 1.0 },
        ]))
    });
    let knob = Driven::new(move |inputs: &Inputs<'_, bool>| knob_of(drag, direction, inputs));
    let fill = Custom::new((knob, white), |recorder, _frame, (knob, white)| {
        recorder.fill(knob, white);
    });

    let control = ToggleControl::new(FRAME, motion())
        .reach(3.0)
        .hit(
            HitRegion::NONE
                .slop(10.0)
                .cancel_distance(70.0)
                .minimum(Size::new(44.0, 44.0)),
        )
        .group_opacity(Driven::new(|inputs: &Inputs<'_, bool>| {
            if inputs.state.contains(InteractionState::DISABLED) {
                0.5
            } else {
                1.0
            }
        }))
        .part(track)
        .backdrop(ToggleBackdrop::new(
            lens,
            Backdrop::new(GLASS_SHADER, GLASS, tinted),
        ))
        .part(fill);
    SwitchLook::new(control, drag)
}

/// The shader and capture class the lens uses.
fn plan() -> ChromePlan {
    ChromePlan {
        shaders: vec![(GLASS_SHADER, BackdropShaderSource::wgsl(TINT_WGSL))],
        captures: vec![(
            GLASS,
            MaterialCapture {
                scale: CaptureScale::new(0.5).expect("0.5 is a valid capture scale"),
                levels: CaptureLevels::ONE,
                pyramid: cherenkov::CapturePyramid::Box,
                grouping: MaterialGrouping::Solo,
                blend_space: cherenkov::BlendSpace::Linear,
            },
        )],
        ..ChromePlan::default()
    }
}

fn theme() -> MinimalTestTheme {
    MinimalTestTheme {
        chrome: plan(),
        ..MinimalTestTheme::default()
    }
    .with_switch(glass_switch)
}

/// The blue the tinted switches draw their on fill with.
fn blue() -> Color {
    Color::srgb(0, 0, 255)
}

/// A switch in a window, pumped on explicit instants.
struct Rig {
    runtime: HeadlessRuntime,
    start: Instant,
    /// The instant of the last frame, in milliseconds from `start`.
    now: u64,
    /// The instant the current motion began.
    origin: u64,
    value: Binding<bool>,
    direction: LayoutDirection,
}

impl Rig {
    fn new(
        view: impl FnOnce(&Binding<bool>) -> AnyView,
        value: bool,
        direction: LayoutDirection,
    ) -> Self {
        let value = Binding::container(value);
        let view = RefCell::new(Some(view(&value)));
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            view.borrow_mut()
                .take()
                .expect("the fixture view is built once")
        });
        let mut env = pumped_test_environment();
        env.insert(direction);
        let runtime = HeadlessRuntime::new_for_tests(env, builder, WIDTH, HEIGHT, theme())
            .with_scale_factor(SCALE);
        let mut rig = Self {
            runtime,
            start: Instant::now(),
            now: 0,
            origin: 0,
            value,
            direction,
        };
        rig.settle();
        rig
    }

    /// A left-to-right switch tinted blue.
    fn blue(value: bool) -> Self {
        Self::new(
            |value| AnyView::new(toggle("", value).switch().tint(blue())),
            value,
            LayoutDirection::LeftToRight,
        )
    }

    fn pump(&mut self, at: u64) -> crate::HeadlessPumpResult {
        assert!(at >= self.now, "frames pump forward in time");
        self.now = at;
        self.runtime
            .pump_at(true, self.start + Duration::from_millis(at))
    }

    /// Frames past every motion the look declares.
    fn settle(&mut self) {
        for _ in 0..80 {
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

    /// The next frame, 16 ms on.
    fn next(&mut self) -> HeadlessSnapshot {
        self.pump(self.now + 16)
            .into_snapshot()
            .expect("a captured frame carries pixels")
    }

    /// The switch's frame, in points: its touch target's centre.
    fn frame(&self) -> Rect {
        let target = self
            .runtime
            .renderer()
            .hit_test
            .pointer_targets
            .iter()
            .find(|target| target.track.is_some())
            .expect("a switch registers a tracked target");
        Rect::from_center_size(target.bounds.center(), FRAME)
    }

    fn track(&self) -> Rect {
        track_of(self.frame(), self.direction)
    }

    fn input(&mut self, event: InputEvent) {
        self.runtime.push_input_event(event);
    }

    fn move_to(&mut self, x: f64, y: f64) {
        let (x, y) = (narrow(x), narrow(y));
        self.input(InputEvent::PointerMove {
            id: POINTER,
            kind: PointerKind::Mouse,
            x,
            y,
        });
    }

    fn down(&mut self, x: f64, y: f64) {
        let (x, y) = (narrow(x), narrow(y));
        self.input(InputEvent::PointerDown {
            id: POINTER,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        });
    }

    fn up(&mut self, x: f64, y: f64) {
        let (x, y) = (narrow(x), narrow(y));
        self.input(InputEvent::PointerUp {
            id: POINTER,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        });
    }

    /// The tree work `count` frames report together.
    fn work(&mut self, count: usize) -> [u64; 3] {
        (0..count).fold([0; 3], |sum, _| {
            let frame = tree_work(self.pump(self.now + 16).profile().counters().frame_work());
            [sum[0] + frame[0], sum[1] + frame[1], sum[2] + frame[2]]
        })
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

/// Whether a pixel is the knob's white.
fn is_white([r, _, b, _]: [u8; 4]) -> bool {
    r > 230 && b > 230
}

/// The knob's leading edge as a fraction of its travel, for the knob drawn
/// on `track` in `snapshot`: the first white pixel along the track's centre
/// row from the end the knob starts at.
fn progress(snapshot: &HeadlessSnapshot, track: Rect, direction: LayoutDirection) -> f64 {
    let y = track.center().y;
    let points = |step: usize| crate::num_cast::usize_as_f64(step) / SCALE;
    let along = |step: usize| match direction {
        LayoutDirection::LeftToRight => track.x0 + points(step),
        LayoutDirection::RightToLeft => track.x1 - points(step),
    };
    // From the second pixel in: the track's own end pixel is the page's.
    let step = (2..device(track.width()))
        .find(|step| is_white(pixel(snapshot, along(*step), y)))
        .expect("the knob is drawn");
    (points(step) - INSET) / TRAVEL
}

/// The track's red channel at its leading end, which the knob never covers
/// at rest: the on fill is blue, so the channel falls as the fade rises.
fn leading_red(snapshot: &HeadlessSnapshot, track: Rect, direction: LayoutDirection) -> f64 {
    let x = match direction {
        LayoutDirection::LeftToRight => track.x0 + 1.0,
        LayoutDirection::RightToLeft => track.x1 - 1.0,
    };
    f64::from(pixel(snapshot, x, track.center().y)[0]) / 255.0
}

/// An sRGB-encoded channel as light.
fn light(channel: f64) -> f64 {
    if channel <= 0.040_45 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

/// The fade `c` a track sample shows, from the track's settled off and on
/// samples: the on fill is drawn over the off fill, so the sample is their
/// mix in light.
fn fade(sample: f64, off: f64, on: f64) -> f64 {
    (light(sample) - light(off)) / (light(on) - light(off))
}

/// `value` within `tolerance` of `want`.
#[track_caller]
fn near(what: &str, value: f64, want: f64, tolerance: f64) {
    assert!(
        (value - want).abs() <= tolerance,
        "{what}: {value:.3}, §8.2 gives {want:.3} (± {tolerance})"
    );
}

/// Where a critically damped spring of `omega` is `seconds` after it began
/// from rest, as a fraction of its travel.
fn spring_at(omega: f64, seconds: f64) -> f64 {
    let decay = (-omega * seconds).exp();
    (-omega.mul_add(seconds, 1.0)).mul_add(decay, 1.0)
}

/// §8.2's toggle motion: from rest the knob covers 28 % of its travel after
/// 0.05 s, 62 % after 0.1 s and 99 % after 0.3 s, and does not overshoot.
#[test]
fn the_knob_runs_the_toggle_spring() {
    let mut rig = Rig::blue(false);
    let track = rig.track();
    rig.value.set(true);
    rig.begin();
    let direction = rig.direction;
    for (after, want) in [(50, 0.28), (100, 0.62), (300, 0.99)] {
        let q = progress(&rig.sample(after), track, direction);
        near(&format!("q at {after} ms"), q, want, 0.05);
        assert!(q <= 1.0 + 0.5 / TRAVEL, "the knob does not overshoot: {q}");
    }
    near("the formula at 50 ms", spring_at(21.0, 0.05), 0.28, 0.005);

    // The motion is the switch's own: turning off runs the same spring.
    rig.value.set(false);
    rig.settle();
    rig.value.set(true);
    rig.settle();
    rig.value.set(false);
    rig.begin();
    let q = progress(&rig.sample(100), track, direction);
    near("q at 100 ms turning off", q, 1.0 - 0.62, 0.05);
}

/// §8.2's track fade: a critically damped spring of ω0 = 11.2 rad/s turning
/// on and 20.2 rad/s turning off, so `c` reaches 0.31 after 0.1 s and 0.85
/// after 0.3 s turning on and 0.91 of the way after 0.2 s turning off.
#[test]
fn the_track_fade_runs_a_spring_per_direction() {
    let mut rig = Rig::blue(false);
    let track = rig.track();
    let direction = rig.direction;
    let off = leading_red(&rig.sample(0), track, direction);
    rig.value.set(true);
    rig.settle();
    let on = leading_red(&rig.sample(0), track, direction);
    assert!(
        off - on > 0.5,
        "the on fill replaces the off fill: {off} {on}"
    );

    rig.value.set(false);
    rig.begin();
    let c = |snapshot: &HeadlessSnapshot| fade(leading_red(snapshot, track, direction), off, on);
    near("c at 0 ms", c(&rig.sample(0)), 1.0, 0.03);
    // Turning off is the faster spring: at 100 ms it has left 0.40 of the
    // fade, where turning on has risen to 0.31 only.
    near(
        "c at 100 ms turning off",
        c(&rig.sample(100)),
        1.0 - spring_at(20.2, 0.1),
        0.04,
    );
    near(
        "c at 200 ms turning off",
        c(&rig.sample(200)),
        1.0 - 0.91,
        0.04,
    );
    rig.settle();

    rig.value.set(true);
    rig.begin();
    near("c at 100 ms turning on", c(&rig.sample(100)), 0.31, 0.04);
    near("c at 300 ms turning on", c(&rig.sample(300)), 0.85, 0.04);
}

/// A drag moves the knob by the finger's offset over the travel, the track
/// fade follows it as `c = q`, and a release settles on the nearer end.
#[test]
fn a_drag_follows_the_finger_and_settles_on_the_nearer_end() {
    let mut rig = Rig::blue(false);
    let track = rig.track();
    let direction = rig.direction;
    let centre = rig.frame().center();
    let off = leading_red(&rig.sample(0), track, direction);
    rig.value.set(true);
    rig.settle();
    let on = leading_red(&rig.sample(0), track, direction);
    rig.value.set(false);
    rig.settle();

    // The finger moves half the travel: the fade, which a press does not
    // touch, follows the knob without animation.
    rig.down(centre.x, centre.y);
    rig.move_to(centre.x + TRAVEL / 2.0, centre.y);
    rig.pump(rig.now + 16);
    let snapshot = rig.pump(rig.now + 16).into_snapshot().expect("pixels");
    near(
        "c follows q",
        fade(leading_red(&snapshot, track, direction), off, on),
        0.5,
        0.06,
    );

    // Released at the half way point: on.
    rig.up(centre.x + TRAVEL / 2.0, centre.y);
    rig.settle();
    assert!(
        rig.value.snapshot(),
        "the release settled on the nearer end"
    );
    let snapshot = rig.sample(0);
    near(
        "q settled",
        progress(&snapshot, track, direction),
        1.0,
        0.05,
    );
}

/// A press moves the lens' operands with no recording and no tree work: the
/// knob grows to `1 + 0.35` of its size, overhanging the track, and the
/// glass shows where the white fill leaves it.
#[test]
fn a_press_updates_the_lens_with_no_recording() {
    let mut rig = Rig::blue(false);
    let track = rig.track();
    let centre = rig.frame().center();
    // The row above the track, inside the pressed lens' overhang.
    let above = (centre.x - 30.5 + INSET + KNOB.width / 2.0, track.y0 - 1.0);
    let at_rest = rig.sample(0);
    let rest = pixel(&at_rest, above.0, above.1);

    rig.down(centre.x, centre.y);
    assert_eq!(
        rig.work(12),
        [0, 0, 0],
        "a press records nothing and walks no tree"
    );
    let pressed = rig.next();
    assert_ne!(
        pixel(&pressed, above.0, above.1),
        rest,
        "the lens overhangs the track"
    );
    // The white fill gave way to the glass at the knob's centre.
    let knob = pixel(&pressed, centre.x, centre.y);
    assert!(
        !is_white(knob),
        "the pressed knob shows the glass: {knob:?}"
    );

    rig.up(centre.x, centre.y);
    rig.settle();
    assert_eq!(
        pixel(&rig.sample(0), above.0, above.1),
        rest,
        "released, the lens returns to the knob"
    );
}

/// §8.2: a disabled switch composites at 0.5 as a whole, so the knob over the
/// on track reads as one half-opaque white, not as the knob half over a half
/// track.
#[test]
fn a_disabled_switch_composites_once() {
    // A disabled switch registers no touch target; the enabled one beside
    // it in the same layout gives the frame.
    let track = Rig::blue(true).track();
    let mut rig = Rig::new(
        |value| {
            AnyView::new(
                toggle("", value)
                    .switch()
                    .tint(Color::srgb(255, 0, 0))
                    .disabled(true),
            )
        },
        true,
        LayoutDirection::LeftToRight,
    );
    let snapshot = rig.sample(0);
    // The knob rests at the trailing end.
    let [r, g, b, _] = pixel(
        &snapshot,
        track.x1 - INSET - KNOB.width / 2.0,
        track.center().y,
    );
    assert!(
        r.abs_diff(g) <= 3 && g.abs_diff(b) <= 3,
        "the knob blends once over the page: ({r}, {g}, {b})"
    );
}

/// The spec carries the tint the subtree sets, else the accent.
#[test]
fn a_tinted_toggle_spec_carries_the_tint() {
    let specs: Rc<RefCell<Vec<WorkingColor>>> = Rc::default();
    let seen = Rc::clone(&specs);
    let theme = MinimalTestTheme {
        chrome: plan(),
        ..MinimalTestTheme::default()
    }
    .with_switch(move |spec, env| {
        seen.borrow_mut().push(spec.tint().resolve(env).snapshot());
        glass_switch(spec, env)
    });
    let value = Binding::container(false);
    let view = RefCell::new(Some(AnyView::new(vstack_of(&value))));
    let builder =
        AnyViewBuilder::<AnyView>::new(move || view.borrow_mut().take().expect("built once"));
    let mut runtime =
        HeadlessRuntime::new_for_tests(pumped_test_environment(), builder, WIDTH, HEIGHT, theme);
    let _ = runtime.pump_snapshot();
    let specs = specs.borrow();
    let accent = Color::new(waterui::theme::color::Accent)
        .resolve(&pumped_test_environment())
        .snapshot();
    let blue = blue().resolve(&pumped_test_environment()).snapshot();
    assert!(
        specs.contains(&blue),
        "the tinted switch's spec carries the tint: {specs:?}"
    );
    assert!(
        specs.contains(&accent),
        "the untinted switch's spec carries the accent: {specs:?}"
    );
}

fn vstack_of(value: &Binding<bool>) -> impl waterui::View {
    waterui_layout::stack::vstack((
        toggle("A", value).switch().tint(blue()),
        toggle("B", value).switch(),
    ))
}

/// §8.2: in a right-to-left layout the label lies at the frame's right and
/// the track at its left, and off is at the track's trailing end.
#[test]
fn right_to_left_mirrors_the_switch() {
    let view = |value: &Binding<bool>| AnyView::new(toggle("Label", value).switch().tint(blue()));
    let target = |rig: &Rig| rig.frame().center().x;

    let left_to_right = Rig::new(view, false, LayoutDirection::LeftToRight);
    let mut right_to_left = Rig::new(view, false, LayoutDirection::RightToLeft);
    let middle = f64::from(WIDTH) / 2.0;
    assert!(
        target(&left_to_right) > middle,
        "the track trails in left-to-right"
    );
    assert!(
        target(&right_to_left) < middle,
        "the track trails in right-to-left, on the left"
    );

    // Off sits at the track's trailing end: the left one's knob is leftmost.
    let track = right_to_left.track();
    let snapshot = right_to_left.sample(0);
    near(
        "q off, mirrored",
        progress(&snapshot, track, LayoutDirection::RightToLeft),
        0.0,
        0.05,
    );
    right_to_left.value.set(true);
    right_to_left.settle();
    let snapshot = right_to_left.sample(0);
    near(
        "q on, mirrored",
        progress(&snapshot, track, LayoutDirection::RightToLeft),
        1.0,
        0.05,
    );
}
