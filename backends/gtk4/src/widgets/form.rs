//! GTK4 form widget implementations.

use gtk4::{
    Box, Button, Calendar, CheckButton, ColorButton, ComboBoxText, Entry, Label, ListBox,
    ListBoxRow, Orientation, Scale, ScrolledWindow, SpinButton, Widget, prelude::*,
};
use std::cell::Cell;
use std::rc::Rc;
use waterui::{
    Environment, Signal,
    component::form::{
        picker::{
            PickerConfig, color::ColorPickerConfig, date::DatePickerConfig,
            multi_date::MultiDatePickerConfig,
        },
        slider::SliderConfig,
        stepper::StepperConfig,
        text_field::{KeyboardType, TextFieldConfig},
        toggle::ToggleConfig,
    },
    core::Color,
};

/// Render a TextField as a GTK4 Entry - takes ownership.
pub fn render_text_field(field: TextFieldConfig, _env: &Environment) -> Widget {
    let entry = Entry::new();

    // Configure entry based on keyboard type
    match field.keyboard {
        KeyboardType::Secure => {
            entry.set_visibility(false);
        }
        KeyboardType::Email => {
            entry.set_input_purpose(gtk4::InputPurpose::Email);
        }
        KeyboardType::URL => {
            entry.set_input_purpose(gtk4::InputPurpose::Url);
        }
        KeyboardType::Number => {
            entry.set_input_purpose(gtk4::InputPurpose::Number);
        }
        KeyboardType::PhoneNumber => {
            entry.set_input_purpose(gtk4::InputPurpose::Phone);
        }
        _ => {
            // Default, no special configuration needed
        }
    }

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
                binding.set(text);
                updating.set(false);
            }
        }
    });

    entry.upcast()
}

/// Render a SecureField as a GTK4 Entry with password mode - takes ownership.
pub fn render_secure_field(field: TextFieldConfig, env: &Environment) -> Widget {
    // SecureField is just a TextField with secure keyboard type
    let mut secure_field = field;
    secure_field.keyboard = KeyboardType::Secure;
    render_text_field(secure_field, env)
}

/// Render a Toggle as a GTK4 CheckButton - takes ownership.
pub fn render_toggle(toggle: ToggleConfig, _env: &Environment) -> Widget {
    // Create a horizontal box to hold the checkbox and label
    let container = Box::new(Orientation::Horizontal, 6);

    let checkbox = CheckButton::new();

    // Set initial state from binding
    let initial_state = toggle.toggle.get();
    checkbox.set_active(initial_state);

    let binding = toggle.toggle;
    let updating = Rc::new(Cell::new(false));

    let guard = binding.watch({
        let checkbox = checkbox.clone();
        let updating = updating.clone();
        move |ctx| {
            if !updating.get() && ctx.value != checkbox.is_active() {
                updating.set(true);
                checkbox.set_active(ctx.value);
                updating.set(false);
            }
        }
    });

    // Set up reactive bindings for state changes
    checkbox.connect_toggled(move |button| {
        let _ = &guard; // Keep the guard alive
        if !updating.get() {
            let state = button.is_active();
            if state != binding.get() {
                updating.set(true);
                binding.set(state);
                updating.set(false);
            }
        }
    });

    container.append(&checkbox);

    // Render the label view
    // TODO: Properly render the AnyView label using the renderer
    // For now, create a placeholder label
    let label = Label::new(Some("Subscribe to Newsletter"));
    container.append(&label);

    container.upcast()
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
        *stepper.range.start() as f64, // Default min range
        *stepper.range.end() as f64,   // Default max range
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

/// Convert WaterUI Color to GTK4 RGBA
fn waterui_color_to_gdk_rgba(color: &Color) -> gtk4::gdk::RGBA {
    let (r, g, b, a) = color.rgba();
    gtk4::gdk::RGBA::new(r, g, b, a)
}

/// Convert GTK4 RGBA to WaterUI Color  
fn gdk_rgba_to_waterui_color(rgba: &gtk4::gdk::RGBA) -> Color {
    Color::from_rgba(rgba.red(), rgba.green(), rgba.blue(), rgba.alpha())
}

/// Render a color picker as a GTK4 ColorButton.
pub fn render_color_picker(color: ColorPickerConfig, _env: &Environment) -> Widget {
    let color_button = ColorButton::new();

    let binding = color.value.clone();
    let updating = Rc::new(Cell::new(false));

    // Set initial color
    let initial_color = binding.get();
    let initial_rgba = waterui_color_to_gdk_rgba(&initial_color);
    color_button.set_rgba(&initial_rgba);

    // Watch for external changes to the binding and update the color button
    let guard = binding.watch({
        let color_button = color_button.clone();
        let updating = updating.clone();
        move |ctx| {
            if !updating.get() {
                updating.set(true);
                let rgba = waterui_color_to_gdk_rgba(&ctx.value);
                color_button.set_rgba(&rgba);
                updating.set(false);
            }
        }
    });

    // Set up reactive bindings for color changes
    color_button.connect_color_set(move |color_button| {
        let _ = &guard; // Keep the guard alive
        if !updating.get() {
            updating.set(true);
            let rgba = color_button.rgba();
            let new_color = gdk_rgba_to_waterui_color(&rgba);
            binding.set(new_color);
            updating.set(false);
        }
    });

    color_button.upcast()
}

/// Render a Picker as a GTK4 ComboBoxText - takes ownership.
pub fn render_picker(picker: PickerConfig, _env: &Environment) -> Widget {
    let combo_box = ComboBoxText::new();

    // Store the mapping between indices and IDs for reverse lookup
    let items = picker.items.get();
    let mut id_mapping = Vec::new();

    // Get initial items and populate the combo box
    for (index, item) in items.iter().enumerate() {
        // Extract text from the TaggedView's Text content
        let text_content = &item.content;
        let display_text = text_content.content().get().to_string();

        combo_box.append_text(&display_text);
        id_mapping.push(item.tag);

        // Set initial selection if this item matches the current binding value
        if item.tag == picker.selection.get() {
            combo_box.set_active(Some(index as u32));
        }
    }

    let binding = picker.selection.clone();
    let updating = Rc::new(Cell::new(false));

    // Watch for external changes to the binding
    let id_mapping_clone = id_mapping.clone();
    let _guard = binding.watch({
        let combo_box = combo_box.clone();
        let updating = updating.clone();
        move |ctx| {
            if !updating.get() {
                updating.set(true);
                // Find the index of the selected ID
                if let Some(index) = id_mapping_clone.iter().position(|id| *id == ctx.value) {
                    combo_box.set_active(Some(index as u32));
                }
                updating.set(false);
            }
        }
    });

    // Set up reactive bindings for selection changes
    {
        let id_mapping_for_changed = id_mapping;
        let binding_for_changed = binding.clone();
        let updating_for_changed = updating.clone();
        combo_box.connect_changed(move |combo_box| {
            if !updating_for_changed.get() {
                updating_for_changed.set(true);
                if let Some(index) = combo_box.active()
                    && (index as usize) < id_mapping_for_changed.len()
                {
                    let selected_id = &id_mapping_for_changed[index as usize];
                    binding_for_changed.set(*selected_id);
                }
                updating_for_changed.set(false);
            }
        });
    }

    combo_box.upcast()
}

/// Convert time::Date to GTK4 DateTime
fn time_date_to_glib_datetime(date: &time::Date) -> glib::DateTime {
    glib::DateTime::from_local(
        date.year(),
        date.month() as i32,
        date.day() as i32,
        0,
        0,
        0.0,
    )
    .unwrap_or_else(|_| glib::DateTime::now_local().unwrap())
}

/// Convert GTK4 Calendar's DateTime to time::Date
fn glib_datetime_to_time_date(
    datetime: &glib::DateTime,
) -> Result<time::Date, time::error::ComponentRange> {
    time::Date::from_calendar_date(
        datetime.year(),
        time::Month::try_from(datetime.month() as u8)?,
        datetime.day_of_month() as u8,
    )
}

/// Render a DatePicker as a GTK4 Calendar - takes ownership.
pub fn render_date_picker(date_picker: DatePickerConfig, _env: &Environment) -> Widget {
    let calendar = Calendar::new();

    let binding = date_picker.value.clone();
    let updating = Rc::new(Cell::new(false));

    // Set initial date
    let initial_date = binding.get();
    let initial_datetime = time_date_to_glib_datetime(&initial_date);
    calendar.select_day(&initial_datetime);

    // Watch for external changes to the binding and update the calendar
    let guard = binding.watch({
        let calendar = calendar.clone();
        let updating = updating.clone();
        move |ctx| {
            if !updating.get() {
                updating.set(true);
                let datetime = time_date_to_glib_datetime(&ctx.value);
                calendar.select_day(&datetime);
                updating.set(false);
            }
        }
    });

    // Set up reactive bindings for date changes
    calendar.connect_day_selected(move |calendar| {
        let _ = &guard; // Keep the guard alive
        if !updating.get() {
            updating.set(true);
            let date = calendar.date();
            if let Ok(new_date) = glib_datetime_to_time_date(&date) {
                binding.set(new_date);
            }
            updating.set(false);
        }
    });

    calendar.upcast()
}

/// Render a MultiDatePicker as a complete multi-selection interface with Calendar and List View.
pub fn render_multi_date_picker(
    multi_date_picker: MultiDatePickerConfig,
    _env: &Environment,
) -> Widget {
    // Create main container with horizontal layout
    let main_container = Box::new(Orientation::Horizontal, 12);

    // Left side: Calendar for date selection
    let calendar = Calendar::new();

    // Right side: List of selected dates with scrolling
    let selected_dates_container = Box::new(Orientation::Vertical, 6);
    let selected_dates_label = Label::new(Some("Selected Dates:"));
    selected_dates_label.set_halign(gtk4::Align::Start);

    let list_box = ListBox::new();
    list_box.set_selection_mode(gtk4::SelectionMode::None);

    let scrolled_window = ScrolledWindow::new();
    scrolled_window.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scrolled_window.set_min_content_height(200);
    scrolled_window.set_min_content_width(200);
    scrolled_window.set_child(Some(&list_box));

    selected_dates_container.append(&selected_dates_label);
    selected_dates_container.append(&scrolled_window);

    // Clear all button
    let clear_button = Button::with_label("Clear All");
    selected_dates_container.append(&clear_button);

    // Add both sides to main container
    main_container.append(&calendar);
    main_container.append(&selected_dates_container);

    let binding = multi_date_picker.value.clone();
    let updating = Rc::new(Cell::new(false));

    // Function to update the list display
    let update_list = {
        let list_box = list_box.clone();
        let binding = binding.clone();
        let updating = updating.clone();

        move || {
            // Clear existing list items
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }

            let selected_dates = binding.get();
            for date in selected_dates.iter() {
                let date_row = ListBoxRow::new();
                let date_box = Box::new(Orientation::Horizontal, 6);

                // Date label
                let date_label = Label::new(Some(&format!("{}", date)));
                date_label.set_hexpand(true);
                date_label.set_halign(gtk4::Align::Start);

                // Remove button for this date
                let remove_button = Button::with_label("Remove");
                let date_to_remove = *date;
                let binding_for_remove = binding.clone();
                let updating_for_remove = updating.clone();

                remove_button.connect_clicked(move |_| {
                    if !updating_for_remove.get() {
                        updating_for_remove.set(true);
                        let mut current_dates = binding_for_remove.get();
                        current_dates.remove(&date_to_remove);
                        binding_for_remove.set(current_dates);
                        updating_for_remove.set(false);
                    }
                });

                date_box.append(&date_label);
                date_box.append(&remove_button);
                date_row.set_child(Some(&date_box));
                list_box.append(&date_row);
            }
        }
    };

    // Initialize the list display
    update_list();

    // Set up initial calendar display (show first selected date if any)
    let initial_dates = binding.get();
    if let Some(first_date) = initial_dates.iter().next() {
        let datetime = time_date_to_glib_datetime(first_date);
        calendar.select_day(&datetime);
    }

    // Watch for external changes to the binding and update both calendar and list
    let update_list_for_watch = {
        let list_box = list_box.clone();
        let binding = binding.clone();
        let updating = updating.clone();

        move || {
            // Clear existing list items
            while let Some(child) = list_box.first_child() {
                list_box.remove(&child);
            }

            let selected_dates = binding.get();
            for date in selected_dates.iter() {
                let date_row = ListBoxRow::new();
                let date_box = Box::new(Orientation::Horizontal, 6);

                // Date label
                let date_label = Label::new(Some(&format!("{}", date)));
                date_label.set_hexpand(true);
                date_label.set_halign(gtk4::Align::Start);

                // Remove button for this date
                let remove_button = Button::with_label("Remove");
                let date_to_remove = *date;
                let binding_for_remove = binding.clone();
                let updating_for_remove = updating.clone();

                remove_button.connect_clicked(move |_| {
                    if !updating_for_remove.get() {
                        updating_for_remove.set(true);
                        let mut current_dates = binding_for_remove.get();
                        current_dates.remove(&date_to_remove);
                        binding_for_remove.set(current_dates);
                        updating_for_remove.set(false);
                    }
                });

                date_box.append(&date_label);
                date_box.append(&remove_button);
                date_row.set_child(Some(&date_box));
                list_box.append(&date_row);
            }
        }
    };

    let _guard = binding.watch({
        let calendar = calendar.clone();
        let updating = updating.clone();
        move |ctx| {
            if !updating.get() {
                updating.set(true);
                // Update list display
                update_list_for_watch();
                // Update calendar to show first selected date
                if let Some(first_date) = ctx.value.iter().next() {
                    let datetime = time_date_to_glib_datetime(first_date);
                    calendar.select_day(&datetime);
                }
                updating.set(false);
            }
        }
    });

    // Calendar day selection - toggle date in/out of selection
    {
        let binding_for_calendar = binding.clone();
        let updating_for_calendar = updating.clone();
        calendar.connect_day_selected(move |calendar| {
            if !updating_for_calendar.get() {
                updating_for_calendar.set(true);
                let date = calendar.date();
                if let Ok(selected_date) = glib_datetime_to_time_date(&date) {
                    let mut current_dates = binding_for_calendar.get();

                    // Toggle the date - add if not present, remove if present
                    if current_dates.contains(&selected_date) {
                        current_dates.remove(&selected_date);
                    } else {
                        current_dates.insert(selected_date);
                    }

                    binding_for_calendar.set(current_dates);
                }
                updating_for_calendar.set(false);
            }
        });
    }

    // Clear all button functionality
    {
        let binding_for_clear = binding.clone();
        clear_button.connect_clicked(move |_| {
            let empty_set = std::collections::BTreeSet::new();
            binding_for_clear.set(empty_set);
        });
    }

    main_container.upcast()
}
