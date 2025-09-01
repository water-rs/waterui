//! Core rendering engine for web backend.

use crate::{
    element::WebElement,
    widgets::{
        form::{
            render_color_picker, render_date_picker, render_picker,
            render_slider, render_stepper, render_text_field, render_toggle,
        },
        layout::{
            render_scroll_view, render_with_padding,
        },
        render_label, render_text,
    },
};
use waterui::{
    Environment, View,
    component::{
        Dynamic, Metadata, Text,
        form::{
            Slider, Stepper, Toggle, TextField,
            picker::{ColorPicker, DatePicker, Picker},
        },
        layout::{Edge, stack::Stack, scroll::ScrollView},
    },
};
use waterui_render_utils::ViewDispatcher;

pub fn dispatcher() -> ViewDispatcher<(), (), WebElement> {
    let mut dispatcher = ViewDispatcher::default();
    
    // Text components
    dispatcher.register(|_, _, text: Text, env: &Environment| render_text(text, env));
    dispatcher.register(|_, _, label: waterui::Str, _| render_label(label));
    
    // Form components
    dispatcher.register(|_, _, field: TextField, env: &Environment| {
        render_text_field(field, env)
    });
    dispatcher.register(|_, _, toggle: Toggle, env: &Environment| {
        render_toggle(toggle, env)
    });
    dispatcher.register(|_, _, slider: Slider, env: &Environment| {
        render_slider(slider, env)
    });
    dispatcher.register(|_, _, stepper: Stepper, env: &Environment| {
        render_stepper(stepper, env)
    });
    dispatcher.register(|_, _, picker: Picker, env: &Environment| {
        render_picker(picker, env)
    });
    dispatcher.register(|_, _, color_picker: ColorPicker, env: &Environment| {
        render_color_picker(color_picker, env)
    });
    dispatcher.register(|_, _, date_picker: DatePicker, env: &Environment| {
        render_date_picker(date_picker, env)
    });
    
    // Layout components
    dispatcher.register(|_, _, stack: Stack, env: &Environment| render_stack(stack, env));
    
    // Layout components
    dispatcher.register(|_, _, scroll: ScrollView, env: &Environment| render_scroll_view(scroll, env));
    
    // Dynamic components
    dispatcher.register(|_, _, dynamic: Dynamic, env: &Environment| render_dynamic(dynamic, env));
    
    // Metadata components
    dispatcher.register(|_, _, padding: Metadata<Edge>, env: &Environment| render_with_padding(padding, env));
    
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

/// Render a stack layout for web
pub fn render_stack(_stack: Stack, _env: &Environment) -> WebElement {
    let container = WebElement::create("div").expect("Failed to create stack container");
    
    // Apply stack-specific styling (simplified implementation)
    let _ = container.set_styles(&[
        ("display", "flex"),
        ("flex-direction", "column"),
        ("gap", "8px"),
    ].iter().cloned().collect());
    
    // TODO: Implement proper stack rendering when Stack API is available
    
    container
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