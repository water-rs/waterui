//! Core rendering engine for GTK4 backend.

use crate::{
    layout::render_stack,
    widgets::{
        dynamic::render_dynamic,
        form::{render_slider, render_stepper, render_text_field, render_toggle},
        render_label, render_text,
    },
};
use gtk4::{Box as GtkBox, Orientation, Widget, prelude::*};

use waterui::{
    Environment, View,
    component::{
        Dynamic, Native, Text,
        form::{
            Slider, Stepper, Toggle, slider::SliderConfig, stepper::StepperConfig,
            text_field::TextFieldConfig, toggle::ToggleConfig,
        },
        layout::stack::Stack,
        text::TextConfig,
    },
};
use waterui::{component::form::TextField, view::ConfigurableView};
use waterui_render_utils::ViewDispatcher;

pub fn dispatcher() -> ViewDispatcher<(), (), Widget> {
    let mut dispatcher = ViewDispatcher::default();
    dispatcher.register(|_, _, text: Text, env: &Environment| render_text(text.config(), env));
    dispatcher.register(|_, _, label: waterui::Str, _| render_label(label));
    dispatcher.register(|_, _, field: TextField, env: &Environment| {
        render_text_field(field.config(), env)
    });

    dispatcher
        .register(|_, _, toggle: Toggle, env: &Environment| render_toggle(toggle.config(), env));

    dispatcher
        .register(|_, _, slider: Slider, env: &Environment| render_slider(slider.config(), env));

    dispatcher.register(|_, _, stepper: Stepper, env: &Environment| {
        render_stepper(stepper.config(), env)
    });
    dispatcher.register(|_, _, stack: Stack, env: &Environment| render_stack(stack, env));

    dispatcher.register(|_, _, dynamic: Dynamic, env: &Environment| render_dynamic(dynamic, env));

    dispatcher.register(|_, _, _: (), _| render_empty());
    dispatcher
}

/// Render an empty widget (for unit type)
pub fn render_empty() -> Widget {
    let empty_box = GtkBox::new(Orientation::Horizontal, 0);
    empty_box.upcast()
}

pub fn render<V: View>(view: V, env: &Environment) -> Widget {
    let mut dispatcher = dispatcher();
    dispatcher.dispatch(view, env, ())
}
