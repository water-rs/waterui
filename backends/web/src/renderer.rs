//! Core rendering engine for web backend.

use crate::{
    element::WebElement,
    widgets::{
        form::{
            render_color_picker, render_date_picker, render_picker, render_slider, render_stepper,
            render_text_field, render_toggle,
        },
        layout::{render_scroll_view, render_with_padding},
        render_label, render_text,
    },
};
use waterui::{
    Environment, View,
    component::{
        Dynamic, Metadata, Text,
        form::{
            Slider, Stepper, TextField, Toggle,
            picker::{ColorPicker, DatePicker, Picker},
        },
        layout::{Edge, scroll::ScrollView},
    },
};
use waterui_render_utils::ViewDispatcher;

pub fn dispatcher() -> ViewDispatcher<(), (), WebElement> {
    let mut dispatcher = ViewDispatcher::default();

    // Text components
    dispatcher.register(|_, _, text: Text, env: &Environment| render_text(text, env));
    dispatcher.register(|_, _, label: waterui::Str, _| render_label(label));

    // Form components
    dispatcher.register(|_, _, field: TextField, env: &Environment| render_text_field(field, env));
    dispatcher.register(|_, _, toggle: Toggle, env: &Environment| render_toggle(toggle, env));
    dispatcher.register(|_, _, slider: Slider, env: &Environment| render_slider(slider, env));
    dispatcher.register(|_, _, stepper: Stepper, env: &Environment| render_stepper(stepper, env));
    dispatcher.register(|_, _, picker: Picker, env: &Environment| render_picker(picker, env));
    dispatcher.register(|_, _, color_picker: ColorPicker, env: &Environment| {
        render_color_picker(color_picker, env)
    });
    dispatcher.register(|_, _, date_picker: DatePicker, env: &Environment| {
        render_date_picker(date_picker, env)
    });

    // Layout components
    // TODO: Implement layout rendering

    // Layout components
    dispatcher
        .register(|_, _, scroll: ScrollView, env: &Environment| render_scroll_view(scroll, env));

    // Dynamic components
    dispatcher.register(|_, _, dynamic: Dynamic, env: &Environment| render_dynamic(dynamic, env));

    // Metadata components
    dispatcher.register(|_, _, padding: Metadata<Edge>, env: &Environment| {
        render_with_padding(padding, env)
    });

    // Empty/unit type
    dispatcher.register(|_, _, _: (), _| render_empty());

    dispatcher
}

/// Render an empty element (for unit type)
pub fn render_empty() -> WebElement {
    WebElement::create("div").unwrap_or_else(|_| {
        // Fallback if div creation fails
        WebElement::create("span").expect("Failed to create fallback span element")
    })
}

/// Render a dynamic view for web
pub fn render_dynamic(_dynamic: Dynamic, _env: &Environment) -> WebElement {
    // TODO: Implement proper dynamic rendering when Dynamic API is available
    let container = WebElement::create("div").expect("Failed to create dynamic container");
    container.set_text_content("Dynamic content placeholder");
    container
}

pub fn render<V: View>(view: V, env: &Environment) -> WebElement {
    let mut dispatcher = dispatcher();
    dispatcher.dispatch(view, env, ())
}
