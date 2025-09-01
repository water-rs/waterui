//! GTK4 form widget implementations.

use gtk4::{ColorButton, Entry, Scale, SpinButton, Switch, Widget, prelude::*};
use std::cell::Cell;
use std::rc::Rc;
use waterui::{
    Environment, Signal,
    component::form::{
        picker::color::ColorPickerConfig, slider::SliderConfig, stepper::StepperConfig,
        text_field::TextFieldConfig, toggle::ToggleConfig,
    },
    core::Color,
};

/// Render a TextField as a GTK4 Entry - takes ownership.
pub fn render_text_field(field: TextFieldConfig, _env: &Environment) -> Widget {
    let entry = Entry::new();

    // Set initial text from binding
    let initial_text = field.value.get();
    entry.set_text(&initial_text);

    // Set placeholder text if available
    let prompt_text = field.prompt.content().get().to_string();
    if !prompt_text.is_empty() {
        entry.set_placeholder_text(Some(&prompt_text));
    }

    let binding = field.value.clone();
    let updating = Rc::new(Cell::new(false));

    // Watch for external changes to the binding and update the entry
    let guard = binding.watch({
        let entry = entry.clone();
        let updating = updating.clone();
        move |ctx| {
            if !updating.get() && ctx.value != entry.text().as_str() {
                updating.set(true);
                entry.set_text(&ctx.value);
                updating.set(false);
            }
        }
    });

    // Set up reactive bindings for text changes
    entry.connect_changed(move |entry| {
        let _ = &guard; // Keep the guard alive
        if !updating.get() {
            let text = entry.text().to_string();
            if text != binding.get() {
                updating.set(true);
                binding.set(text.into());
                updating.set(false);
            }
        }
    });

    // TODO: Handle keyboard type configuration

    entry.upcast()
}

/// Render a Toggle as a GTK4 Switch - takes ownership.
pub fn render_toggle(toggle: ToggleConfig, _env: &Environment) -> Widget {
    let widget = Switch::new();

    // Set initial state from binding
    let initial_state = toggle.toggle.get();
    widget.set_active(initial_state);

    let binding = toggle.toggle;
    let updating = Rc::new(Cell::new(false));

    let guard = binding.watch({
        let widget = widget.clone();
        let updating = updating.clone();
        move |ctx| {
            if !updating.get() && ctx.value != widget.is_active() {
                updating.set(true);
                widget.set_active(ctx.value);
                updating.set(false);
            }
        }
    });

    // Set up reactive bindings for state changes
    widget.connect_state_notify(move |switch| {
        let _ = &guard; // Keep the guard alive
        if !updating.get() {
            let state = switch.is_active();
            if state != binding.get() {
                updating.set(true);
                binding.set(state);
                updating.set(false);
            }
        }
    });

    widget.upcast()
}

/// Render a Slider as a GTK4 Scale - takes ownership.
pub fn render_slider(slider: SliderConfig, _env: &Environment) -> Widget {
    // Create scale with proper range from config
    let range = slider.range;
    let widget = Scale::with_range(
        gtk4::Orientation::Horizontal,
        *range.start(),
        *range.end(),
        1.0, // Default step size for GTK4 Scale
    );

    // Set number of digits for display
    widget.set_digits(2);

    // Set initial value from binding
    let initial_value = slider.value.get();
    widget.set_value(initial_value);

    let binding = slider.value.clone();
    let updating = Rc::new(Cell::new(false));

    // Watch for external changes to the binding and update the scale
    let guard = binding.watch({
        let widget = widget.clone();
        let updating = updating.clone();
        move |ctx| {
            if !updating.get() && (ctx.value - widget.value()).abs() > f64::EPSILON {
                updating.set(true);
                widget.set_value(ctx.value);
                updating.set(false);
            }
        }
    });

    // Set up reactive bindings for value changes
    widget.connect_value_changed(move |scale| {
        let _ = &guard; // Keep the guard alive
        if !updating.get() {
            let value = scale.value();
            if (value - binding.get()).abs() > f64::EPSILON {
                updating.set(true);
                binding.set(value);
                updating.set(false);
            }
        }
    });

    widget.upcast()
}

/// Render a Stepper as a GTK4 SpinButton - takes ownership.
pub fn render_stepper(stepper: StepperConfig, _env: &Environment) -> Widget {
    // Create spin button with step size from config
    let step_value = stepper.step.get() as f64;
    let widget = SpinButton::with_range(
        0.0,    // Default min range
        1000.0, // Default max range
        step_value,
    );

    // Set to integer mode
    widget.set_digits(0);

    // Set initial value from binding
    let initial_value = stepper.value.get() as f64;
    widget.set_value(initial_value);

    let binding = stepper.value.clone();
    let updating = Rc::new(Cell::new(false));

    // Watch for external changes to the binding and update the spin button
    let guard = binding.watch({
        let spin_button = widget.clone();
        let updating = updating.clone();
        move |ctx| {
            let new_value = ctx.value as f64;
            if !updating.get() && (new_value - spin_button.value()).abs() > f64::EPSILON {
                updating.set(true);
                spin_button.set_value(new_value);
                updating.set(false);
            }
        }
    });

    // Set up reactive bindings for value changes
    widget.connect_value_changed(move |spin_button| {
        let _ = &guard; // Keep the guard alive
        if !updating.get() {
            let value = spin_button.value() as i32;
            if value != binding.get() {
                updating.set(true);
                binding.set(value);
                updating.set(false);
            }
        }
    });

    widget.upcast()
}

/// Render a color picker as a GTK4 ColorButton.
pub fn render_color_picker(color: ColorPickerConfig, _env: &Environment) -> Widget {
    let color_button = ColorButton::new();

    let binding = color.value.clone();

    // Watch for external changes to the binding and update the color button
    let guard = binding.watch({
        let _color_button = color_button.clone();
        move |_ctx| {
            // TODO: Convert WaterUI Color to GDK RGBA when Color struct is implemented
            // For now, just trigger the callback without changing the color
        }
    });

    // Set up reactive bindings for color changes
    color_button.connect_color_set(move |color_button| {
        let _ = &guard; // Keep the guard alive
        let _rgba = color_button.rgba();
        // TODO: Convert GDK RGBA to WaterUI Color when Color struct is implemented
        // For now, create a default color
        let new_color = Color::default();
        binding.set(new_color);
    });

    color_button.upcast()
}
