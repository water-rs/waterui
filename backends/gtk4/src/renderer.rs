//! Core rendering engine for GTK4 backend.

use crate::{
    layout::{render_scroll_view, render_stack},
    widgets::{
        dynamic::render_dynamic,
        form::{
            render_color_picker, render_date_picker, render_multi_date_picker, render_picker,
            render_secure_field, render_slider, render_stepper, render_text_field, render_toggle,
        },
        general::{
            render_divider, render_horizontal_divider, render_loading_progress, render_progress,
            render_vertical_divider, render_with_padding,
        },
        render_label, render_text,
    },
};
use gtk4::{Box as GtkBox, Orientation, Widget, prelude::*};

use waterui::{
    Environment, View,
    component::{
        Dynamic, Metadata, Native, Text,
        form::{
            Slider, Stepper, TextField, Toggle,
            picker::{ColorPicker, DatePicker, Picker},
            slider::SliderConfig,
            stepper::StepperConfig,
            text_field::TextFieldConfig,
            toggle::ToggleConfig,
        },
        layout::{Edge, stack::Stack},
        text::TextConfig,
    },
};
use waterui::{component::layout::scroll::ScrollView, view::ConfigurableView};
use waterui_render_utils::ViewDispatcher;

pub fn dispatcher() -> ViewDispatcher<(), (), Widget> {
    let mut dispatcher = ViewDispatcher::default();

    // Text components
    dispatcher.register(|_, _, text: Text, env: &Environment| render_text(text.config(), env));
    dispatcher.register(|_, _, label: waterui::Str, _| render_label(label));

    // Form components
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
    dispatcher
        .register(|_, _, picker: Picker, env: &Environment| render_picker(picker.config(), env));
    dispatcher.register(|_, _, color_picker: ColorPicker, env: &Environment| {
        render_color_picker(color_picker.config(), env)
    });
    dispatcher.register(|_, _, date_picker: DatePicker, env: &Environment| {
        render_date_picker(date_picker.config(), env)
    });

    dispatcher
        .register(|_, _, scroll: ScrollView, env: &Environment| render_scroll_view(scroll, env));
    // Layout components
    dispatcher.register(|_, _, stack: Stack, env: &Environment| render_stack(stack, env));

    // Dynamic components
    dispatcher.register(|_, _, dynamic: Dynamic, env| render_dynamic(dynamic, env));

    // Empty/unit type
    dispatcher.register(|_, _, _: (), _| render_empty());

    dispatcher.register(|_, _, padding: Metadata<Edge>, env| render_with_padding(padding, env));

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
