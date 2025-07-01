# WaterUI Form Components

**Version**: 0.1.0  
**Location**: `components/form/`

## Overview

`waterui-form` provides comprehensive form input components for building interactive user interfaces. It includes text fields, toggles, sliders, steppers, pickers, and a powerful form builder system that automatically generates forms from data types.

## Core Components

### TextField
Text input component for single and multi-line text:

```rust
use waterui_form::{TextField, field};

// Direct usage
let text_field = TextField::new(&binding)
    .placeholder("Enter your name")
    .label("Name");

// Convenience function
let field = field(&binding, "Name");
```

### Toggle
Boolean input component with switch-like behavior:

```rust
use waterui_form::{Toggle, toggle};

let toggle = Toggle::new(&binding)
    .label("Enable notifications");

// Convenience function  
let switch = toggle(&binding, "Notifications");
```

### Slider
Continuous value input with draggable slider:

```rust
use waterui_form::Slider;

let slider = Slider::new(&binding)
    .range(0.0..=100.0)
    .step(1.0)
    .label("Volume");
```

### Stepper
Discrete numeric input with increment/decrement buttons:

```rust
use waterui_form::{Stepper, stepper};

let stepper = Stepper::new(&binding)
    .range(1..=10)
    .step(1)
    .label("Quantity");

// Convenience function
let qty_stepper = stepper(&binding, "Quantity");
```

### Picker
Selection components for choosing from predefined options:

#### ColorPicker
```rust
use waterui_form::picker::ColorPicker;

let color_picker = ColorPicker::new(&binding)
    .label("Theme Color");
```

## Form Builder System

The `FormBuilder` trait provides automatic form generation from data types:

### FormBuilder Trait

```rust
pub trait FormBuilder: Sized {
    fn build(label: impl View) -> FormBuilderResult<Self, impl View>;
}

pub struct FormBuilderResult<T: 'static, V: View> {
    pub view: V,
    pub value: Binding<T>,
}
```

### Built-in Implementations

The crate provides `FormBuilder` implementations for common types:

- `Str` → `TextField`
- `i32` → `Stepper` 
- `bool` → `Toggle`
- `Color` → `ColorPicker`

### Usage Example

```rust
use waterui_form::FormBuilder;

// Automatically generate appropriate form controls
let (name_view, name_binding) = String::build("Name");
let (age_view, age_binding) = i32::build("Age"); 
let (enabled_view, enabled_binding) = bool::build("Enabled");

// Combine into a form
let form = VStack::new()
    .child(name_view)
    .child(age_view)
    .child(enabled_view);
```

## Implementation Macro

The `impl_form_builder!` macro simplifies adding `FormBuilder` support for custom types:

```rust
macro_rules! impl_form_builder {
    ($ty:ty, $view:ty) => {
        impl FormBuilder for $ty {
            fn build(label: impl View) -> FormBuilderResult<Self, impl View> {
                let value = Binding::default();
                FormBuilderResult {
                    view: <$view>::new(&value).label(label),
                    value,
                }
            }
        }
    };
}

// Usage
impl_form_builder!(Str, TextField);
impl_form_builder!(i32, Stepper);
```

## Component Features

### Reactive Data Binding
All form components are built with reactive data binding:

```rust
let data = binding("initial value");
let field = TextField::new(&data);

// Changes to the field automatically update the binding
// Changes to the binding automatically update the field
data.set("new value"); // Field will show "new value"
```

### Validation Support
Components support validation through reactive computed values:

```rust
let email = binding(String::new());
let is_valid = email.map(|email| email.contains('@'));

let field = TextField::new(&email)
    .validation(is_valid)
    .error_message("Please enter a valid email");
```

### Accessibility
All components include built-in accessibility features:
- Screen reader support
- Keyboard navigation
- Focus management
- ARIA attributes

### Styling & Theming
Components respect the global theme and can be customized:

```rust
let field = TextField::new(&binding)
    .style(TextFieldStyle {
        background_color: Color::WHITE,
        border_color: Color::GRAY,
        focus_color: Color::BLUE,
    });
```

## Dependencies

- `waterui-core`: Core framework functionality
- `waterui-reactive`: Reactive data binding
- `waterui-color`: Color handling
- `alloc`: Memory allocation (no-std compatible)

## Form Patterns

### Dynamic Forms
Create forms that adapt based on user input:

```rust
let form_type = binding(FormType::Basic);
let form = Dynamic::watch(form_type.clone(), move |form_type| {
    match form_type {
        FormType::Basic => basic_form(),
        FormType::Advanced => advanced_form(),
    }
});
```

### Multi-step Forms
Build wizard-style forms with multiple steps:

```rust
let current_step = binding(0);
let form = Dynamic::watch(current_step.clone(), move |step| {
    match step {
        0 => personal_info_step(),
        1 => contact_info_step(), 
        2 => preferences_step(),
        _ => summary_step(),
    }
});
```

### Form Validation
Implement comprehensive form validation:

```rust
let form_data = binding(FormData::default());
let validation = form_data.map(|data| data.validate());

let submit_button = Button::new("Submit")
    .enabled(validation.map(|v| v.is_valid()))
    .on_click(move || {
        if validation.get().is_valid() {
            submit_form(form_data.get());
        }
    });
```

## Best Practices

1. **Use convenience functions**: Prefer `field()`, `toggle()`, `stepper()` for simple cases
2. **Leverage FormBuilder**: Use the form builder system for automatic form generation
3. **Validate reactively**: Use reactive computations for real-time validation
4. **Group related fields**: Use layout components to organize form sections
5. **Provide feedback**: Show validation states and error messages clearly
6. **Consider accessibility**: Ensure forms work well with assistive technologies
