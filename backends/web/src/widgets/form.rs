//! Form widget implementations for the web backend.

use crate::element::WebElement;
use std::collections::HashMap;
use waterui::component::form::{
    Slider, Stepper, TextField, Toggle,
    picker::{ColorPicker, DatePicker, Picker},
};
use waterui::view::ConfigurableView;
use waterui::{Environment, Signal};

/// Render a text input field.
pub fn render_text_field(field: TextField, _env: &Environment) -> WebElement {
    let element = WebElement::create("input").expect("Failed to create input element");

    let _ = element.set_attribute("type", "text");

    // Get the current value from the text field
    let config = field.config();
    let value = config.value.get();
    let _ = element.set_attribute("value", value.as_ref());

    // Add basic styling
    let styles: HashMap<&str, &str> = [
        ("padding", "8px 12px"),
        ("border", "1px solid #ccc"),
        ("border-radius", "4px"),
        ("font-size", "14px"),
    ]
    .iter()
    .cloned()
    .collect();

    let _ = element.set_styles(&styles);

    element
}

/// Render a secure password field.
pub fn render_secure_field(field: TextField, env: &Environment) -> WebElement {
    let element = render_text_field(field, env);
    let _ = element.set_attribute("type", "password");
    element
}

/// Render a toggle/checkbox.
pub fn render_toggle(toggle: Toggle, _env: &Environment) -> WebElement {
    let container = WebElement::create("label").expect("Failed to create label for toggle");
    let checkbox = WebElement::create("input").expect("Failed to create checkbox input");

    let _ = checkbox.set_attribute("type", "checkbox");

    // Get the current toggle state
    let config = toggle.config();
    let is_on = config.toggle.get();
    if is_on {
        let _ = checkbox.set_attribute("checked", "true");
    }

    let _ = container.append_child(&checkbox);

    // Add label text - simplified for now
    let text_node = WebElement::create_text("Toggle").expect("Failed to create text node");
    let _ = container.append_text(&text_node);

    container
}

/// Render a slider input.
pub fn render_slider(slider: Slider, _env: &Environment) -> WebElement {
    let element = WebElement::create("input").expect("Failed to create slider input");

    let _ = element.set_attribute("type", "range");

    let config = slider.config();
    let _range_signal = config.range;
    let value = config.value.get();

    // For now, use default range values
    let min_val = 0.0;
    let max_val = 100.0;

    let _ = element.set_attribute("min", &min_val.to_string());
    let _ = element.set_attribute("max", &max_val.to_string());
    let _ = element.set_attribute("value", value.to_string().as_ref());
    let _ = element.set_attribute("step", "0.01");

    element
}

/// Render a numeric stepper.
pub fn render_stepper(stepper: Stepper, _env: &Environment) -> WebElement {
    let element = WebElement::create("input").expect("Failed to create stepper input");

    let _ = element.set_attribute("type", "number");

    let config = stepper.config();
    let value = config.value.get();
    let step = config.step.get();
    let _range_signal = config.range;

    // For now, use default range values
    let min_val = 0.0;
    let max_val = 100.0;

    let _ = element.set_attribute("min", &min_val.to_string());
    let _ = element.set_attribute("max", &max_val.to_string());
    let _ = element.set_attribute("step", &step.to_string());
    let _ = element.set_attribute("value", value.to_string().as_ref());

    element
}

/// Render a generic picker (select dropdown).
pub fn render_picker(_picker: Picker, _env: &Environment) -> WebElement {
    let select = WebElement::create("select").expect("Failed to create select element");

    // Add options (simplified - would need access to actual options from config)
    // This is a placeholder implementation
    let option = WebElement::create("option").expect("Failed to create option element");
    option.set_text_content("Select option...");
    let _ = select.append_child(&option);

    select
}

/// Render a color picker.
pub fn render_color_picker(_picker: ColorPicker, _env: &Environment) -> WebElement {
    let element = WebElement::create("input").expect("Failed to create color input");

    let _ = element.set_attribute("type", "color");
    let _ = element.set_attribute("value", "#000000");

    element
}

/// Render a date picker.
pub fn render_date_picker(_picker: DatePicker, _env: &Environment) -> WebElement {
    let element = WebElement::create("input").expect("Failed to create date input");

    let _ = element.set_attribute("type", "date");

    element
}
