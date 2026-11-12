//! Text field looks (water-rs/waterui#4042): a field whose theme hands out a
//! [`TextFieldLook`] records its container once, and a hover or a focus
//! change afterwards moves only live operands. Measurement and drawing read
//! the one look, a secure field wears the same look, and the spec carries the
//! layout direction.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use cherenkov_record::ShapeData;
use kurbo::Rect;
use nami::Binding;
use waterui_controls::TextField;
use waterui_core::AnyView;
use waterui_core::Str;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::interaction::{InteractionState, StateValue};
use waterui_core::layout::LayoutDirection;
use waterui_form::secure::{Secure, secure};
use waterui_layout::stack::vstack;

use super::chrome_primitives::Frames;
use super::{MinimalTestTheme, invisible_chrome, pumped_test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerKind};
use crate::renderer::chrome::LiveChrome;
use crate::testing::plain_text_field_look;
use crate::theme::{Border, Driven, InputFieldMetrics, TextFieldLook, TextFieldSpec};

const POINTER: u64 = 1;
const WINDOW: u32 = 320;
const METRICS: InputFieldMetrics = InputFieldMetrics {
    label_height: 14.0,
    min_width: 150.0,
    min_height: 40.0,
    horizontal_inset: 8.0,
    vertical_inset: 6.0,
};

/// The plain look with an active indicator that thickens on focus.
fn look() -> TextFieldLook {
    let indicator = StateValue::new(Border::new(
        1.0,
        cherenkov_record::WorkingColor::new([0.0; 4]),
    ))
    .when(
        InteractionState::FOCUSED,
        Border::new(
            2.0,
            cherenkov_record::WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
        ),
    );
    plain_text_field_look(invisible_chrome(), METRICS).active_indicator(Driven::from(indicator))
}

struct Fixture {
    runtime: HeadlessRuntime,
    frames: Frames,
    specs: Rc<RefCell<Vec<TextFieldSpec>>>,
}

impl Fixture {
    fn new(view: AnyView, direction: LayoutDirection) -> Self {
        let theme = MinimalTestTheme::default().with_text_field(|_, _| look());
        let specs = Rc::clone(&theme.text_field_specs);
        let view = RefCell::new(Some(view));
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
        for _ in 0..4 {
            frames.pump(&mut runtime);
        }
        Self {
            runtime,
            frames,
            specs,
        }
    }

    fn text(direction: LayoutDirection) -> Self {
        let value = Binding::container(Str::default());
        Self::new(
            AnyView::new(vstack((TextField::new("Name", &value),))),
            direction,
        )
    }

    fn secure() -> Self {
        let value = Binding::container(Secure::new(String::new()));
        Self::new(
            AnyView::new(vstack((secure("Password", &value),))),
            LayoutDirection::LeftToRight,
        )
    }

    /// The field's live chrome.
    fn chrome(&self) -> Rc<LiveChrome> {
        let chromes = self.runtime.renderer().hit_test.interaction.live_chromes();
        <[_; 1]>::try_from(chromes).expect("a field binds one live chrome")[0].clone()
    }

    fn rect(&self) -> Rect {
        match self.chrome().shape() {
            ShapeData::RoundedRect(shape) => shape.rect(),
            other => {
                panic!("the plain container's silhouette is a rounded rectangle, got {other:?}")
            }
        }
    }

    fn input(&mut self, event: InputEvent) {
        self.runtime.push_input_event(event);
    }

    fn pointer(
        &mut self,
        at: kurbo::Point,
        event: impl FnOnce(u64, PointerKind, f32, f32) -> InputEvent,
    ) {
        let (x, y) = (
            crate::num_cast::f64_as_f32(at.x),
            crate::num_cast::f64_as_f32(at.y),
        );
        self.input(event(POINTER, PointerKind::Mouse, x, y));
    }

    /// Moves keyboard focus onto the field's text input, as a key does.
    fn focus_field(&mut self) {
        let renderer = self.runtime.renderer_mut();
        let key = renderer.text_editing.text_input_targets[0]
            .interaction_key
            .clone();
        assert!(renderer.set_keyboard_focus(Some(key), true));
    }

    fn settle(&mut self) {
        for _ in 0..48 {
            self.frames.pump(&mut self.runtime);
        }
    }
}

#[test]
fn hover_and_focus_record_nothing() {
    let mut fixture = Fixture::text(LayoutDirection::LeftToRight);
    let chrome = fixture.chrome();
    let recordings = chrome.recordings();
    assert!(recordings > 0, "the mount records the container");
    let center = fixture.rect().center();

    fixture.pointer(center, |id, kind, x, y| InputEvent::PointerMove {
        id,
        kind,
        x,
        y,
    });
    assert_eq!(
        fixture.frames.work(&mut fixture.runtime, 8),
        [0, 0, 0],
        "hovering the field records nothing and walks no tree"
    );

    fixture.focus_field();
    fixture.settle();
    assert!(
        chrome.state().contains(InteractionState::FOCUSED),
        "the field took keyboard focus"
    );
    assert!(
        Rc::ptr_eq(&chrome, &fixture.chrome()),
        "focusing keeps the mounted chrome"
    );
    assert_eq!(
        chrome.recordings(),
        recordings,
        "the focus change, the active indicator and the label floating record no chrome"
    );
}

#[test]
fn text_and_secure_fields_lay_out_by_the_look() {
    let rects = [
        Fixture::text(LayoutDirection::LeftToRight).rect(),
        Fixture::secure().rect(),
    ];
    for rect in rects {
        assert!(
            rect.height() >= METRICS.min_height - 1e-6,
            "the container is at least as tall as the look's metrics ask: {rect:?}"
        );
        assert!(
            rect.width() >= METRICS.min_width - 1e-6,
            "the container is at least as wide as the look's metrics ask: {rect:?}"
        );
    }
    assert_eq!(
        rects[0].size(),
        rects[1].size(),
        "a secure field measures as a text field does: they share the look"
    );
}

#[test]
fn the_spec_carries_the_layout_direction() {
    for direction in [LayoutDirection::LeftToRight, LayoutDirection::RightToLeft] {
        let fixture = Fixture::text(direction);
        let specs = fixture.specs.borrow();
        assert!(!specs.is_empty(), "the field asked for its look");
        assert!(
            specs.iter().all(|spec| spec.direction() == direction),
            "{direction:?}: {specs:?}"
        );
    }
}

#[test]
fn a_field_requests_its_look_once_per_spec() {
    let mut fixture = Fixture::text(LayoutDirection::LeftToRight);
    let asked = fixture.specs.borrow().len();
    fixture.settle();
    assert_eq!(
        fixture.specs.borrow().len(),
        asked,
        "steady frames keep the look the field holds"
    );
}
