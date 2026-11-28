//! Ambient `ViewExt::control_size` inheritance and explicit `.size(...)`
//! override across all eight sized controls (`water-rs/waterui#4426`).

use std::cell::RefCell;
use std::num::NonZeroI32;
use std::rc::Rc;
use std::time::Instant;

use jiff::civil::Date;
use nami::Binding;
use waterui::ViewExt as _;
use waterui::component::menu::Menu;
use waterui_controls::ControlSize;
use waterui_controls::button::button;
use waterui_controls::slider::slider;
use waterui_controls::stepper::stepper;
use waterui_controls::text_field::field;
use waterui_controls::toggle::toggle;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::Id;
use waterui_core::{AnyView, Str};
use waterui_form::picker::date::DatePicker;
use waterui_form::picker::{Picker, PickerItem, PickerStyle};
use waterui_layout::stack::vstack;

use super::chrome_primitives::Frames;
use super::{MinimalTestTheme, pumped_test_environment};
use crate::HeadlessRuntime;

const WINDOW: u32 = 420;

fn run_control_size_scene(view: AnyView) -> MinimalTestTheme {
    let theme = MinimalTestTheme::default();
    let recorded_button = Rc::clone(&theme.button_specs);
    let recorded_toggle = Rc::clone(&theme.toggle_specs);
    let recorded_slider = Rc::clone(&theme.slider_specs);
    let recorded_picker = Rc::clone(&theme.picker_specs);
    let recorded_segmented = Rc::clone(&theme.segmented_specs);
    let recorded_stepper = Rc::clone(&theme.stepper_specs);
    let recorded_text_field = Rc::clone(&theme.text_field_specs);

    let view = RefCell::new(Some(view));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the control_size fixture view is built once")
    });
    let env = pumped_test_environment();
    let mut runtime = HeadlessRuntime::new_for_tests(env, builder, WINDOW, WINDOW, theme);
    let mut frames = Frames {
        start: Instant::now(),
        next: 0,
    };
    for _ in 0..3 {
        frames.pump(&mut runtime);
    }

    MinimalTestTheme {
        button_specs: recorded_button,
        toggle_specs: recorded_toggle,
        slider_specs: recorded_slider,
        picker_specs: recorded_picker,
        segmented_specs: recorded_segmented,
        stepper_specs: recorded_stepper,
        text_field_specs: recorded_text_field,
        ..MinimalTestTheme::default()
    }
}

fn id(raw: i32) -> Id {
    Id::from(NonZeroI32::new(raw).expect("non-zero test id"))
}

fn picker_items() -> Vec<PickerItem<Id>> {
    vec![
        PickerItem::new(id(1), "One".into()),
        PickerItem::new(id(2), "Two".into()),
    ]
}

#[test]
fn ambient_control_size_propagates_to_all_eight_controls() {
    let on = Binding::new(false);
    let slider_val = Binding::new(0.5_f64);
    let choice = Binding::new(id(1));
    let step = Binding::new(1_i32);
    let text_val = Binding::new(Str::from_static("hello"));
    let date = Binding::new(Date::new(2025, 6, 15).expect("valid test date"));

    // `Large` is distinct from both defaults (`Small` and Slider's `ExtraSmall`).
    let view = vstack((
        button("Save").action(|| {}),
        toggle("Enable", &on),
        Menu::new("Actions", (button("Ping").action(|| {}),)),
        slider("Volume", &slider_val),
        Picker::new("Choice", picker_items(), &choice).style(PickerStyle::Menu),
        Picker::new("Mode", picker_items(), &choice).style(PickerStyle::Segmented),
        stepper("Count", &step),
        field("Email", &text_val),
        DatePicker::new("Date", &date),
    ))
    .control_size(ControlSize::Large);

    let recorded = run_control_size_scene(AnyView::new(view));

    let buttons = recorded.button_specs.borrow();
    assert!(
        buttons.len() >= 2,
        "expected specs for both Button and Menu trigger, got {}",
        buttons.len()
    );
    assert!(
        buttons.iter().all(|s| s.size() == ControlSize::Large),
        "Button and Menu trigger must inherit ambient Large: {buttons:?}"
    );

    let toggles = recorded.toggle_specs.borrow();
    assert!(!toggles.is_empty(), "expected ToggleSpec");
    assert!(
        toggles.iter().all(|s| s.size() == ControlSize::Large),
        "Toggle must inherit ambient Large: {toggles:?}"
    );

    let sliders = recorded.slider_specs.borrow();
    assert!(!sliders.is_empty(), "expected SliderSpec");
    assert!(
        sliders.iter().all(|s| s.size() == ControlSize::Large),
        "Slider must inherit ambient Large: {sliders:?}"
    );

    let pickers = recorded.picker_specs.borrow();
    assert!(
        pickers.len() >= 2,
        "expected PickerSpec for both Menu Picker and DatePicker, got {}",
        pickers.len()
    );
    assert!(
        pickers.iter().all(|s| s.size() == ControlSize::Large),
        "Picker and DatePicker must inherit ambient Large: {pickers:?}"
    );

    let segmented = recorded.segmented_specs.borrow();
    assert!(!segmented.is_empty(), "expected SegmentedSpec");
    assert!(
        segmented.iter().all(|s| s.size() == ControlSize::Large),
        "Segmented Picker must inherit ambient Large: {segmented:?}"
    );

    let steppers = recorded.stepper_specs.borrow();
    assert!(!steppers.is_empty(), "expected StepperSpec");
    assert!(
        steppers.iter().all(|s| s.size() == ControlSize::Large),
        "Stepper must inherit ambient Large: {steppers:?}"
    );

    let fields = recorded.text_field_specs.borrow();
    assert!(
        fields.len() >= 2,
        "expected TextFieldSpec for both TextField and DatePicker, got {}",
        fields.len()
    );
    assert!(
        fields.iter().all(|s| s.size() == ControlSize::Large),
        "TextField and DatePicker field look must inherit ambient Large: {fields:?}"
    );
}

#[test]
fn explicit_size_overrides_ambient_control_size_on_all_eight_controls() {
    let on = Binding::new(false);
    let slider_val = Binding::new(0.5_f64);
    let choice = Binding::new(id(1));
    let step = Binding::new(1_i32);
    let text_val = Binding::new(Str::from_static("hello"));
    let date = Binding::new(Date::new(2025, 6, 15).expect("valid test date"));

    // Explicit `Medium` differs from both ambient `Large` and every control's default.
    let view = vstack((
        button("Save").size(ControlSize::Medium).action(|| {}),
        toggle("Enable", &on).size(ControlSize::Medium),
        Menu::new("Actions", (button("Ping").action(|| {}),)).size(ControlSize::Medium),
        slider("Volume", &slider_val).size(ControlSize::Medium),
        Picker::new("Choice", picker_items(), &choice)
            .style(PickerStyle::Menu)
            .size(ControlSize::Medium),
        Picker::new("Mode", picker_items(), &choice)
            .style(PickerStyle::Segmented)
            .size(ControlSize::Medium),
        stepper("Count", &step).size(ControlSize::Medium),
        field("Email", &text_val).size(ControlSize::Medium),
        DatePicker::new("Date", &date).size(ControlSize::Medium),
    ))
    .control_size(ControlSize::Large);

    let recorded = run_control_size_scene(AnyView::new(view));

    let buttons = recorded.button_specs.borrow();
    assert!(buttons.len() >= 2);
    assert!(
        buttons.iter().all(|s| s.size() == ControlSize::Medium),
        "explicit Button/Menu size must override ambient: {buttons:?}"
    );

    let toggles = recorded.toggle_specs.borrow();
    assert!(!toggles.is_empty());
    assert!(
        toggles.iter().all(|s| s.size() == ControlSize::Medium),
        "explicit Toggle size must override ambient: {toggles:?}"
    );

    let sliders = recorded.slider_specs.borrow();
    assert!(!sliders.is_empty());
    assert!(
        sliders.iter().all(|s| s.size() == ControlSize::Medium),
        "explicit Slider size must override ambient: {sliders:?}"
    );

    let pickers = recorded.picker_specs.borrow();
    assert!(pickers.len() >= 2);
    assert!(
        pickers.iter().all(|s| s.size() == ControlSize::Medium),
        "explicit Picker/DatePicker size must override ambient: {pickers:?}"
    );

    let segmented = recorded.segmented_specs.borrow();
    assert!(!segmented.is_empty());
    assert!(
        segmented.iter().all(|s| s.size() == ControlSize::Medium),
        "explicit Segmented Picker size must override ambient: {segmented:?}"
    );

    let steppers = recorded.stepper_specs.borrow();
    assert!(!steppers.is_empty());
    assert!(
        steppers.iter().all(|s| s.size() == ControlSize::Medium),
        "explicit Stepper size must override ambient: {steppers:?}"
    );

    let fields = recorded.text_field_specs.borrow();
    assert!(fields.len() >= 2);
    assert!(
        fields.iter().all(|s| s.size() == ControlSize::Medium),
        "explicit TextField/DatePicker size must override ambient: {fields:?}"
    );
}
