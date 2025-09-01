# WaterUI Form Components

Form input components and utilities for the WaterUI framework.

## Overview

`waterui-form` provides a comprehensive set of form input components including text fields, toggles, sliders, steppers, and pickers. It also includes a form builder system for creating type-safe forms with reactive data binding.

## Components

### Text Field

Text input component with validation and formatting:

```rust
use waterui_form::{TextField, field};

let text_field = TextField::new(&binding)
    .label("Enter your name")
    .placeholder("John Doe")
    .validation(|text| !text.is_empty());

// Convenience function
let field = field("Username", &username_binding);
```

### Toggle

Boolean input component (checkbox/switch):

```rust
use waterui_form::{Toggle, toggle};

let toggle = Toggle::new(&enabled_binding)
    .label("Enable notifications");

// Convenience function
let switch = toggle("Dark mode", &dark_mode_binding);
```

### Slider

Numeric input with visual slider:

```rust
use waterui_form::Slider;

let slider = Slider::new(&volume_binding)
    .range(0.0..=100.0)
    .step(1.0)
    .label("Volume");
```

### Stepper

Numeric input with increment/decrement buttons:

```rust
use waterui_form::{Stepper, stepper};

let stepper = Stepper::new(&count_binding)
    .range(0..=10)
    .step(1)
    .label("Quantity");

// Convenience function
let counter = stepper("Items", &item_count_binding);
```

### Color Picker

Color selection component:

```rust
use waterui_form::picker::ColorPicker;

let color_picker = ColorPicker::new(&color_binding)
    .label("Background Color")
    .show_alpha(true);
```

## Form Builder

Type-safe form creation with automatic UI generation:

```rust
use waterui_form::{FormBuilder, FormBuilderResult};
use waterui_core::{View};
use waterui_str::Str;
use waterui_core::Color;

// Automatically creates appropriate form controls
let name_form = Str::build("Name");          // Creates TextField
let age_form = i32::build("Age");            // Creates Stepper
let enabled_form = bool::build("Enabled");   // Creates Toggle
let color_form = Color::build("Color");      // Creates ColorPicker

// Access the generated UI and reactive bindings
let FormBuilderResult { view, value } = name_form;
```

### Form Builder Trait

Implement `FormBuilder` for custom types:

```rust
impl FormBuilder for MyType {
    fn build(label: impl View) -> FormBuilderResult<Self, impl View> {
        let value = Binding::default();
        FormBuilderResult {
            view: MyCustomInput::new(&value).label(label),
            value,
        }
    }
}
```

## Validation

Form components support validation with error display:

```rust
let email_field = TextField::new(&email_binding)
    .label("Email")
    .validation(|email| {
        if email.contains('@') {
            Ok(())
        } else {
            Err("Please enter a valid email address")
        }
    })
    .show_validation_errors(true);
```

## Reactive Data Flow

All form components work with reactive `Binding<T>` values:

```rust
use nami::binding;

let username = binding(String::new());
let age = binding(25);
let notifications = binding(true);

// Form components automatically update when bindings change
// and update bindings when user interacts with the form
let form = vstack![
    field("Username", &username),
    stepper("Age", &age),
    toggle("Notifications", &notifications),
];
```

## Styling

Form components can be styled and customized:

```rust
let styled_field = TextField::new(&binding)
    .label("Styled Input")
    .border_color(Color::BLUE)
    .focus_color(Color::GREEN)
    .error_color(Color::RED)
    .corner_radius(8.0);
```

## Dependencies

- `waterui-core`: Core framework functionality

## Example

```rust
use waterui_form::*;
use waterui_layout::vstack;
use waterui_core::{binding, View, Environment, Button};

struct UserForm {
    name: binding<String>,
    age: binding<i32>,
    notifications: binding<bool>,
}

impl View for UserForm {
    fn body(self, _env: &Environment) -> impl View {
        vstack([
            field("Name", &self.name),
            stepper("Age", &self.age),
            toggle("Enable Notifications", &self.notifications),
            Button::new("Submit")
                .on_click({
                    let form = self.clone();
                    move |_| form.submit()
                }),
        ])
    }
}

impl UserForm {
    fn submit(&self) {
        println!("Name: {}", self.name.get());
        println!("Age: {}", self.age.get());
        println!("Notifications: {}", self.notifications.get());
    }
}
```
