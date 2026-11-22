//! Stepper looks (water-rs/waterui#4041): a stepper whose theme hands out a
//! [`StepperLook`] records each end's chrome once, and a hover, press or
//! value change afterwards moves only live operands. Each end is disabled at
//! its limit of the range, the ends lie in the order the layout direction
//! gives, and measurement and drawing read the one look.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use crate::draw::{Draw, Paint, WorkingColor};
use cherenkov_record::ShapeData;
use kurbo::{Rect, RoundedRect};
use nami::{Binding, Signal as _};
use waterui::{Color, ViewExt as _};
use waterui_controls::stepper::stepper;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::interaction::InteractionState;
use waterui_core::layout::LayoutDirection;

use super::chrome_primitives::Frames;
use super::{MinimalTestTheme, pumped_test_environment, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::renderer::chrome::LiveChrome;
use crate::testing::plain_stepper_look;
use crate::theme::{Custom, Driven, StepperLook};

const POINTER: u64 = 1;
const WINDOW: u32 = 320;
/// The ends of the plain look: each 14 points square, 4 points apart.
const END: f64 = 14.0;
const GAP: f64 = 4.0;
/// The two ends and the gap between them.
const PAIR: f64 = END + GAP + END;

fn red() -> Paint {
    Paint::from(WorkingColor::new([1.0, 0.0, 0.0, 1.0]))
}

/// The plain look, each end drawing a red square on its frame and listing
/// the frame it drew on: decrement first, then increment.
fn look(drawn: &Rc<RefCell<Vec<Rect>>>) -> StepperLook {
    let plain = plain_stepper_look();
    let icon = || {
        let drawn = Rc::clone(drawn);
        Custom::new(
            (Driven::constant(red()),),
            move |recorder, frame, (paint,)| {
                drawn.borrow_mut().push(frame);
                recorder.fill(RoundedRect::from_rect(frame, 0.0), paint);
            },
        )
    };
    StepperLook::new(
        plain.decrement().clone().custom(icon()),
        plain.increment().clone().custom(icon()),
        plain.button(),
    )
    .spacing(plain.get_spacing())
    .label_spacing(plain.get_label_spacing())
}

struct Fixture {
    runtime: HeadlessRuntime,
    frames: Frames,
    value: Binding<i32>,
    drawn: Rc<RefCell<Vec<Rect>>>,
}

impl Fixture {
    fn new(
        view: impl FnOnce(&Binding<i32>) -> AnyView,
        value: i32,
        direction: LayoutDirection,
    ) -> Self {
        let value = Binding::container(value);
        let drawn = Rc::new(RefCell::new(Vec::new()));
        let theme = MinimalTestTheme::default().with_stepper_look(look(&drawn));
        let view = RefCell::new(Some(view(&value)));
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            view.borrow_mut()
                .take()
                .expect("the fixture view is built once")
        });
        let mut env = pumped_test_environment();
        env.insert(direction);
        let mut runtime = HeadlessRuntime::new_for_tests(env, builder, WINDOW, WINDOW, theme);
        let mut frames = Frames {
            start: Instant::now(),
            next: 0,
        };
        // Mount plus a few steady frames settle the scene before input lands.
        for _ in 0..4 {
            frames.pump(&mut runtime);
        }
        Self {
            runtime,
            frames,
            value,
            drawn,
        }
    }

    /// A labelled stepper over `0..=5`.
    fn ranged(value: i32) -> Self {
        Self::new(
            |value| AnyView::new(stepper("Quantity", value).range(0..=5)),
            value,
            LayoutDirection::LeftToRight,
        )
    }

    /// The two ends' frames as the look last drew them: decrement, increment.
    fn ends(&self) -> [Rect; 2] {
        let drawn = self.drawn.borrow();
        let [decrement, increment] = drawn[drawn.len() - 2..] else {
            unreachable!("the slice holds two frames")
        };
        [decrement, increment]
    }

    /// The two ends' live chromes, the leftmost first.
    fn chromes(&self) -> [Rc<LiveChrome>; 2] {
        let mut chromes = self.runtime.renderer().hit_test.interaction.live_chromes();
        chromes.sort_by(|a, b| left_of(a).total_cmp(&left_of(b)));
        <[_; 2]>::try_from(chromes).expect("a stepper binds one live chrome per end")
    }

    /// The centres of the touch targets the stepper registers, the leftmost
    /// first: one per enabled end.
    fn targets(&self) -> Vec<(f32, f32)> {
        let mut bounds: Vec<Rect> = self
            .runtime
            .renderer()
            .hit_test
            .pointer_targets
            .iter()
            .map(|target| target.bounds)
            .collect();
        bounds.sort_by(|a, b| a.x0.total_cmp(&b.x0));
        bounds
            .into_iter()
            .map(|bounds| {
                let center = bounds.center();
                (
                    crate::num_cast::f64_as_f32(center.x),
                    crate::num_cast::f64_as_f32(center.y),
                )
            })
            .collect()
    }

    fn input(&mut self, event: impl FnOnce(u64, PointerKind) -> InputEvent) {
        self.runtime
            .push_input_event(event(POINTER, PointerKind::Mouse));
    }

    fn move_to(&mut self, (x, y): (f32, f32)) {
        self.input(|id, kind| InputEvent::PointerMove { id, kind, x, y });
    }

    fn down(&mut self, (x, y): (f32, f32)) {
        self.input(|id, kind| InputEvent::PointerDown {
            id,
            kind,
            x,
            y,
            button: PointerButton::Primary,
        });
    }

    fn up(&mut self, (x, y): (f32, f32)) {
        self.input(|id, kind| InputEvent::PointerUp {
            id,
            kind,
            x,
            y,
            button: PointerButton::Primary,
        });
    }

    /// Frames past every motion the look declares.
    fn settle(&mut self) {
        for _ in 0..48 {
            self.frames.pump(&mut self.runtime);
        }
    }
}

/// The left edge of a chrome's silhouette.
fn left_of(chrome: &Rc<LiveChrome>) -> f64 {
    match chrome.shape() {
        ShapeData::RoundedRect(shape) => shape.rect().x0,
        other => panic!("the plain look's silhouette is a rounded rectangle, got {other:?}"),
    }
}

#[test]
fn press_on_each_end_records_nothing() {
    let mut fixture = Fixture::ranged(2);
    let chromes = fixture.chromes();
    let recordings = chromes.clone().map(|chrome| chrome.recordings());
    assert_eq!(
        recordings,
        [2, 2],
        "the mount records each end once: its surface below and its layers above"
    );

    for (end, expected) in [(0, 1), (1, 2)] {
        let target = fixture.targets()[end];
        fixture.move_to(target);
        assert_eq!(
            fixture.frames.work(&mut fixture.runtime, 8),
            [0, 0, 0],
            "hovering end {end} records nothing and walks no tree"
        );
        fixture.down(target);
        assert_eq!(
            fixture.frames.work(&mut fixture.runtime, 8),
            [0, 0, 0],
            "pressing end {end} records nothing and walks no tree"
        );
        assert!(
            chromes[end].state().contains(InteractionState::PRESSED),
            "the press landed on end {end}"
        );
        fixture.up(target);
        fixture.settle();
        assert_eq!(fixture.value.snapshot(), expected, "end {end} stepped once");
    }

    for (kept, now) in chromes.iter().zip(fixture.chromes()) {
        assert!(Rc::ptr_eq(kept, &now), "the cycle keeps the mounted chrome");
    }
    assert_eq!(
        chromes.map(|chrome| chrome.recordings()),
        recordings,
        "press, release and the value change record no chrome"
    );
}

/// Each end is disabled at its limit of the range, through its own state, and
/// leaves the limit without recording again.
#[test]
fn an_end_at_its_limit_is_disabled() {
    let mut fixture = Fixture::ranged(0);
    let [decrement, increment] = fixture.chromes();
    let recordings = [decrement.recordings(), increment.recordings()];
    assert!(decrement.state().contains(InteractionState::DISABLED));
    assert!(!increment.state().contains(InteractionState::DISABLED));
    assert_eq!(
        fixture.targets().len(),
        1,
        "the end at its limit takes no taps"
    );

    let tapped = fixture.targets()[0];
    fixture.down(tapped);
    fixture.up(tapped);
    fixture.settle();
    assert_eq!(fixture.value.snapshot(), 1, "the other end stepped");
    assert!(
        !decrement.state().contains(InteractionState::DISABLED),
        "the decrement end left its limit"
    );
    assert_eq!(fixture.targets().len(), 2);

    fixture.value.set(5);
    fixture.settle();
    assert!(!decrement.state().contains(InteractionState::DISABLED));
    assert!(increment.state().contains(InteractionState::DISABLED));
    assert_eq!(fixture.targets().len(), 1);

    let [kept_decrement, kept_increment] = fixture.chromes();
    assert!(Rc::ptr_eq(&decrement, &kept_decrement) && Rc::ptr_eq(&increment, &kept_increment));
    assert_eq!(
        [decrement.recordings(), increment.recordings()],
        recordings,
        "a value change records no chrome"
    );

    // The one target left is the decrement end's: the increment end at its
    // limit registers none, so a tap on it reaches nothing.
    let [decrement_frame, _] = fixture.ends();
    let [target] = <[_; 1]>::try_from(fixture.targets()).expect("one end takes taps");
    assert!((f64::from(target.0) - decrement_frame.center().x).abs() < 1e-3);
}

/// A disabled stepper disables both ends, whatever the value.
#[test]
fn a_disabled_stepper_disables_both_ends() {
    let mut fixture = Fixture::new(
        |value| AnyView::new(stepper("Quantity", value).range(0..=5).disabled(true)),
        2,
        LayoutDirection::LeftToRight,
    );
    fixture.settle();
    for chrome in fixture.chromes() {
        assert!(chrome.state().contains(InteractionState::DISABLED));
    }
    assert_eq!(
        fixture.targets().len(),
        0,
        "a disabled stepper takes no taps"
    );
}

/// The decrement end leads the pair: left in a left-to-right layout, right
/// in a right-to-left one, with the label on the other side.
#[test]
fn right_to_left_reverses_the_pair() {
    for (direction, decrement_leads_left) in [
        (LayoutDirection::LeftToRight, true),
        (LayoutDirection::RightToLeft, false),
    ] {
        let fixture = Fixture::new(
            |value| AnyView::new(stepper("Quantity", value)),
            2,
            direction,
        );
        let [decrement, increment] = fixture.ends();
        assert_eq!(
            decrement.x0 < increment.x0,
            decrement_leads_left,
            "{direction:?}: decrement {decrement:?}, increment {increment:?}"
        );
        assert!(
            (decrement.x1.max(increment.x1) - decrement.x0.min(increment.x0) - PAIR).abs() < 1e-6,
            "{direction:?}: the pair is as wide as the look declares"
        );
        // The pair lies at the trailing edge, the label before it.
        let pair = decrement.union(increment);
        let window = f64::from(WINDOW);
        match direction {
            LayoutDirection::LeftToRight => assert!((pair.x1 - window).abs() < 1.0, "{pair:?}"),
            LayoutDirection::RightToLeft => assert!(pair.x0.abs() < 1.0, "{pair:?}"),
        }
    }
}

/// The ends are drawn at the size the look declares, and take touches on
/// exactly that frame, in a row of any proposed height.
#[test]
fn measurement_equals_drawing_at_every_proposed_height() {
    for height in [6.0_f32, 10.0, 14.0, 30.0, 80.0] {
        let fixture = Fixture::new(
            |value| {
                AnyView::new(
                    AnyView::new(stepper("Quantity", value).hide_label()).size(200.0, height),
                )
            },
            2,
            LayoutDirection::LeftToRight,
        );
        let [decrement, increment] = fixture.ends();
        assert!(
            (decrement.width() - END).abs() < 1e-6 && (increment.width() - END).abs() < 1e-6,
            "height {height}: the ends keep the look's width, drew {decrement:?} and {increment:?}"
        );
        assert!(
            (increment.x0 - decrement.x1 - GAP).abs() < 1e-6,
            "height {height}: the ends keep the look's spacing"
        );
        assert!(
            (decrement.height() - increment.height()).abs() < 1e-6
                && decrement.height() <= END + 1e-6,
            "height {height}: the ends are as tall as the row allows, no taller than the look"
        );
        let targets = fixture.targets();
        assert_eq!(targets.len(), 2);
        // Frames are in the node's space and targets in the window's, so
        // compare the distance between the ends.
        let apart = f64::from(targets[1].0 - targets[0].0);
        assert!(
            (apart - (increment.center().x - decrement.center().x)).abs() < 1e-3,
            "height {height}: the touch targets lie on the drawn ends"
        );
    }
}

/// The spec carries what the look is built from: the direction and the
/// subtree's tint.
#[test]
fn the_spec_carries_the_tint_and_the_structure() {
    let specs = Rc::new(RefCell::new(Vec::new()));
    let theme = MinimalTestTheme {
        stepper_specs: Rc::clone(&specs),
        ..Default::default()
    };
    let env = test_environment();
    let mut renderer = super::test_renderer_with_theme(theme);
    let tint = Color::srgb(200, 30, 30);
    super::capture_root_window(
        &mut renderer,
        stepper("Quantity", &Binding::container(1)).tint(tint.clone()),
        &env,
        Rect::new(0.0, 0.0, 400.0, 100.0),
    );

    let specs = specs.borrow();
    let spec = specs.first().expect("the theme was asked for the look");
    assert_eq!(spec.direction(), LayoutDirection::LeftToRight);
    assert_eq!(
        spec.tint().resolve(&env).snapshot(),
        tint.resolve(&env).snapshot(),
        "the spec carries the subtree's tint"
    );
}
