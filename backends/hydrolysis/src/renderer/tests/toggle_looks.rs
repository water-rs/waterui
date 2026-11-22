//! Toggle looks (water-rs/waterui#4038): a switch whose theme hands out a
//! [`SwitchLook`] records its parts once, and a toggle, hover, press or drag
//! afterwards moves only their live operands. The knob follows a drag, each
//! operand runs the motion it declares, and measurement and drawing read the
//! one look.

use core::time::Duration;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use cherenkov_record::{Draw, Paint, WorkingColor};
use kurbo::{Rect, RoundedRect};
use nami::{Binding, Signal as _};
use waterui::animation::Animation;
use waterui_controls::toggle::toggle;
use waterui_core::animation::ChangeAnimation;
use waterui_core::interaction::InteractionState;
use waterui_core::layout::LayoutDirection;
use waterui_layout::stack::vstack;

use super::chrome_primitives::{Frames, runtime_with};
use super::{MinimalTestTheme, bit};
use crate::HeadlessRuntime;
use crate::HeadlessSnapshot;
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::renderer::live_toggle::LiveToggle;
use crate::theme::{
    Custom, Driven, HitRegion, Inputs, Motion, SwitchDrag, SwitchLook, ToggleControl, ValueMotion,
};

const POINTER: u64 = 1;
/// The knob's travel along the track.
const TRAVEL: f64 = 20.0;
const KNOB: f64 = 10.0;

fn red() -> Paint {
    Paint::from(WorkingColor::new([1.0, 0.0, 0.0, 1.0]))
}

fn blue() -> Paint {
    Paint::from(WorkingColor::new([0.0, 0.0, 1.0, 1.0]))
}

/// The two value motions a look's operands run with.
fn linear(millis: u64) -> Motion {
    let mut motion = Motion::uniform(Animation::linear(Duration::from_millis(15)));
    motion.value = ValueMotion::Own(Animation::linear(Duration::from_millis(millis)));
    motion
}

/// A 40 by 20 switch drawn as a red knob over a blue strip, on different
/// value motions: the knob's takes 100 ms, the strip's 400 ms. Every
/// recording lists the frame it recorded on.
fn look(recorded: Rc<RefCell<Vec<Rect>>>, direction: LayoutDirection) -> SwitchLook {
    let drag = SwitchDrag::new(TRAVEL, direction);
    let knob = Driven::new(move |inputs: &Inputs<'_, bool>| {
        let at = drag
            .progress(*inputs.value, inputs.drag)
            .mul_add(TRAVEL, inputs.frame.x0);
        RoundedRect::new(at, inputs.frame.y0, at + KNOB, inputs.frame.y0 + KNOB, 0.0)
    })
    .motion(linear(100));
    let strip = Driven::new(move |inputs: &Inputs<'_, bool>| {
        let at = inputs.frame.x0 + if *inputs.value { TRAVEL } else { 0.0 };
        RoundedRect::new(
            at,
            inputs.frame.y0 + KNOB,
            at + KNOB,
            inputs.frame.y0 + KNOB + KNOB,
            0.0,
        )
    })
    .motion(linear(400));
    let control = ToggleControl::new(kurbo::Size::new(40.0, 20.0), linear(100))
        .hit(
            HitRegion::NONE
                .slop(10.0)
                .cancel_distance(70.0)
                .minimum(kurbo::Size::new(44.0, 44.0)),
        )
        .part(Custom::new(
            (knob, strip),
            move |recorder, frame, (knob, strip)| {
                recorded.borrow_mut().push(frame);
                recorder.fill(knob, red());
                recorder.fill(strip, blue());
            },
        ));
    SwitchLook::new(control, drag)
}

struct Fixture {
    runtime: HeadlessRuntime,
    frames: Frames,
    value: Binding<bool>,
    recorded: Rc<RefCell<Vec<Rect>>>,
}

fn fixture(value: Binding<bool>) -> Fixture {
    let recorded = Rc::new(RefCell::new(Vec::new()));
    let theme = MinimalTestTheme::default()
        .with_switch_look(look(Rc::clone(&recorded), LayoutDirection::LeftToRight));
    let mut runtime = runtime_with(toggle("", &value).switch(), theme);
    let mut frames = Frames {
        start: Instant::now(),
        next: 0,
    };
    // Mount plus a few steady frames settle the scene before input lands.
    for _ in 0..4 {
        frames.pump(&mut runtime);
    }
    Fixture {
        runtime,
        frames,
        value,
        recorded,
    }
}

impl Fixture {
    /// The one mounted live toggle.
    fn toggle(&self) -> Rc<LiveToggle> {
        let toggles = self.runtime.renderer().hit_test.interaction.live_toggles();
        let [toggle] = <[_; 1]>::try_from(toggles).expect("one switch binds a live toggle");
        toggle
    }

    /// The centre of the switch's touch target, in window points.
    fn center(&self) -> (f32, f32) {
        let target = self
            .runtime
            .renderer()
            .hit_test
            .pointer_targets
            .iter()
            .find(|target| target.track.is_some())
            .expect("a switch registers a tracked target");
        let center = target.bounds.center();
        (
            crate::num_cast::f64_as_f32(center.x),
            crate::num_cast::f64_as_f32(center.y),
        )
    }

    fn mouse(&mut self, event: impl FnOnce(u64, PointerKind) -> InputEvent) {
        self.runtime
            .push_input_event(event(POINTER, PointerKind::Mouse));
    }

    fn move_to(&mut self, x: f32, y: f32) {
        self.mouse(|id, kind| InputEvent::PointerMove { id, kind, x, y });
    }

    fn down(&mut self, x: f32, y: f32) {
        self.mouse(|id, kind| InputEvent::PointerDown {
            id,
            kind,
            x,
            y,
            button: PointerButton::Primary,
        });
    }

    fn up(&mut self, x: f32, y: f32) {
        self.mouse(|id, kind| InputEvent::PointerUp {
            id,
            kind,
            x,
            y,
            button: PointerButton::Primary,
        });
    }

    fn pump(&mut self, count: usize) {
        for _ in 0..count {
            self.frames.pump(&mut self.runtime);
        }
    }

    /// Frames past every motion the look declares.
    fn settle(&mut self) {
        self.pump(48);
    }
}

/// A pixel column as a coordinate.
fn column(x: usize) -> f64 {
    f64::from(u32::try_from(x).expect("a window column fits u32"))
}

/// The left edge of the pixels `matches` selects.
fn left_of(snapshot: &HeadlessSnapshot, matches: impl Fn([u8; 4]) -> bool) -> f64 {
    let width = snapshot.width() as usize;
    snapshot
        .rgba8()
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        .filter(|(_, pixel)| matches(**pixel))
        .map(|(index, _)| index % width)
        .min()
        .map(column)
        .expect("the part is drawn")
}

fn is_red([r, g, b, _]: [u8; 4]) -> bool {
    r > 200 && g < 60 && b < 60
}

fn is_blue([r, g, b, _]: [u8; 4]) -> bool {
    b > 200 && r < 60 && g < 60
}

/// Where the knob and the strip are now.
fn parts_at(fixture: &mut Fixture) -> (f64, f64) {
    let snapshot = fixture.frames.snapshot(&mut fixture.runtime);
    (left_of(&snapshot, is_red), left_of(&snapshot, is_blue))
}

#[test]
fn hover_press_and_toggle_record_nothing() {
    let mut fixture = fixture(Binding::container(false));
    let toggle = fixture.toggle();
    let recordings = toggle.recordings();
    assert_eq!(recordings, 1, "the mount records the parts once");
    let (x, y) = fixture.center();

    fixture.move_to(x, y);
    assert_eq!(
        fixture.frames.work(&mut fixture.runtime, 8),
        [0, 0, 0],
        "hover records nothing and walks no tree"
    );
    fixture.down(x, y);
    assert_eq!(
        fixture.frames.work(&mut fixture.runtime, 8),
        [0, 0, 0],
        "a press records nothing and walks no tree"
    );
    fixture.up(x, y);
    fixture.settle();

    assert!(fixture.value.snapshot(), "the tap toggled the switch");
    assert!(
        Rc::ptr_eq(&toggle, &fixture.toggle()),
        "the toggle keeps the mounted parts"
    );
    assert_eq!(toggle.recordings(), recordings, "a toggle records nothing");
    assert_eq!(
        fixture.frames.work(&mut fixture.runtime, 8),
        [0, 0, 0],
        "a settled toggle walks no tree"
    );
}

/// Two operands of one toggle run the springs their drivens declare: the
/// knob's 100 ms and the strip's 400 ms.
#[test]
fn operands_run_their_own_motion() {
    let mut fixture = fixture(Binding::container(false));
    let (rest_knob, rest_strip) = parts_at(&mut fixture);

    fixture.value.set(true);
    fixture.pump(4);
    let (knob, strip) = parts_at(&mut fixture);
    let (knob, strip) = (knob - rest_knob, strip - rest_strip);
    assert!(
        knob > 0.0 && knob < TRAVEL,
        "the knob is mid-travel, moved {knob}"
    );
    assert!(
        strip > 0.0 && strip < knob,
        "the slower strip trails the knob: knob {knob}, strip {strip}"
    );

    fixture.settle();
    let (knob, strip) = parts_at(&mut fixture);
    assert!((knob - rest_knob - TRAVEL).abs() < 1.5);
    assert!((strip - rest_strip - TRAVEL).abs() < 1.5);
}

/// A look-owned value motion overrides the animation the change names, and
/// snaps when the change disables animation.
#[test]
fn own_motion_overrides_the_caller_and_snaps_without_animation() {
    let mut named = fixture(Binding::container(false));
    let (rest, _) = parts_at(&mut named);
    // The change names a 10 s animation; the look's 100 ms still runs.
    named.toggle().change_value(
        true,
        &ChangeAnimation::Animated(Animation::linear(Duration::from_secs(10))),
    );
    named.pump(8);
    let (knob, _) = parts_at(&mut named);
    assert!(
        (knob - rest - TRAVEL).abs() < 1.5,
        "the look's motion overrides the caller's: moved {}",
        knob - rest
    );

    let mut snapped = fixture(Binding::container(false));
    let (rest, _) = parts_at(&mut snapped);
    snapped
        .toggle()
        .change_value(true, &ChangeAnimation::Disabled);
    let (knob, strip) = parts_at(&mut snapped);
    assert!(
        (knob - rest - TRAVEL).abs() < 1.5 && (strip - rest - TRAVEL).abs() < 1.5,
        "a change that disables animation snaps both operands"
    );
}

/// The knob follows the finger, and a release settles it on the nearer end.
#[test]
fn drag_moves_the_knob_and_settles_on_the_nearer_end() {
    let mut fixture = fixture(Binding::container(false));
    let (rest, _) = parts_at(&mut fixture);
    let (x, y) = fixture.center();

    fixture.down(x, y);
    fixture.move_to(x + 14.0, y);
    fixture.pump(1);
    let (knob, _) = parts_at(&mut fixture);
    assert!(
        (knob - rest - 14.0).abs() < 1.5,
        "the knob follows the finger, moved {}",
        knob - rest
    );
    assert_eq!(
        fixture.toggle().state() & InteractionState::DRAGGED,
        InteractionState::DRAGGED,
        "the toggle reads as dragged"
    );

    // Back under the half way point: the release settles on off.
    fixture.move_to(x + 4.0, y);
    fixture.up(x + 4.0, y);
    fixture.settle();
    assert!(!fixture.value.snapshot());
    let (knob, _) = parts_at(&mut fixture);
    assert!((knob - rest).abs() < 1.5, "settled on the off end");

    fixture.down(x, y);
    fixture.move_to(x + 16.0, y);
    fixture.up(x + 16.0, y);
    fixture.settle();
    assert!(fixture.value.snapshot(), "settled on the on end");
    let (knob, _) = parts_at(&mut fixture);
    assert!((knob - rest - TRAVEL).abs() < 1.5);
    assert!(
        !fixture.toggle().state().contains(InteractionState::DRAGGED),
        "the drag ended"
    );
}

/// A release beyond the cancel distance neither toggles nor settles on the
/// dragged position.
#[test]
fn release_beyond_the_cancel_distance_settles_back() {
    let mut fixture = fixture(Binding::container(false));
    let (x, y) = fixture.center();
    fixture.down(x, y);
    fixture.move_to(x + 16.0, y);
    fixture.move_to(x + 16.0, y + 200.0);
    fixture.up(x + 16.0, y + 200.0);
    fixture.settle();
    assert!(!fixture.value.snapshot());
}

/// Measurement and drawing read one look: the toggle measures the control's
/// size, and the parts record on a frame of exactly that size.
#[test]
fn measurement_equals_drawing() {
    let fixture = fixture(Binding::container(false));
    let recorded = fixture.recorded.borrow();
    let frame = recorded.last().expect("the parts recorded");
    assert_eq!(frame.size(), kurbo::Size::new(40.0, 20.0));
    let (_, _) = fixture.center();
    let target = fixture
        .runtime
        .renderer()
        .hit_test
        .pointer_targets
        .iter()
        .find(|target| target.track.is_some())
        .expect("a tracked target")
        .bounds;
    assert_eq!(
        target.center(),
        frame.center(),
        "the touch target is the control's frame grown about its centre"
    );
    assert_eq!(target.size(), kurbo::Size::new(44.0, 44.0));
}

/// Toggles whose bindings share a `SignalIdentity` (water-rs/waterui#2232)
/// each own their parts: the off switch never shows the on switch's state.
#[test]
fn toggles_sharing_a_mapping_call_site_own_their_parts() {
    let source = Binding::container(0b01_u8);
    assert_eq!(
        bit(&source, 1).identity(),
        bit(&source, 2).identity(),
        "same-site mappings must share a SignalIdentity for this test to cover the bug"
    );
    let recorded = Rc::new(RefCell::new(Vec::new()));
    let theme = MinimalTestTheme::default()
        .with_switch_look(look(Rc::clone(&recorded), LayoutDirection::LeftToRight));
    let mut runtime = runtime_with(
        vstack((
            toggle("A", &bit(&source, 1)).switch(),
            toggle("B", &bit(&source, 2)).switch(),
        )),
        theme,
    );
    let mut frames = Frames {
        start: Instant::now(),
        next: 0,
    };
    for _ in 0..4 {
        frames.pump(&mut runtime);
    }
    let toggles = runtime.renderer().hit_test.interaction.live_toggles();
    assert_eq!(toggles.len(), 2, "each switch owns its parts");
    let ends = knob_ends(&frames.snapshot(&mut runtime));
    assert_eq!(ends.len(), 2, "A is on and B is off: {ends:?}");
    assert!((ends[1] - ends[0] - TRAVEL).abs() < 1.5);

    bit(&source, 2).set(true);
    for _ in 0..48 {
        frames.pump(&mut runtime);
    }
    let ends = knob_ends(&frames.snapshot(&mut runtime));
    assert_eq!(ends.len(), 1, "both are on: {ends:?}");
}

/// The distinct left edges of the knobs drawn, each row's first red pixel,
/// merged within a pixel.
fn knob_ends(snapshot: &HeadlessSnapshot) -> Vec<f64> {
    let width = snapshot.width() as usize;
    let mut ends: Vec<f64> = Vec::new();
    for row in snapshot.rgba8().as_chunks::<4>().0.chunks(width) {
        let Some(x) = row.iter().position(|pixel| is_red(*pixel)) else {
            continue;
        };
        let x = column(x);
        if !ends.iter().any(|end| (end - x).abs() < 1.5) {
            ends.push(x);
        }
    }
    ends.sort_by(f64::total_cmp);
    ends
}
