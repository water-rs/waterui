# waterui-form

A comprehensive form building system for WaterUI applications with automatic component generation, validation, and secure data handling.

## Overview

`waterui-form` provides an ergonomic, type-safe approach to building interactive forms in WaterUI applications. The crate centers around the `FormBuilder` trait, which enables automatic mapping from Rust data structures to platform-native UI components. It includes specialized pickers (color, date, multi-date), secure input handling with automatic memory zeroing, and a flexible validation system.

Key features:
- **Automatic form generation** via `#[derive(FormBuilder)]` macro
- **Type-to-component mapping** - primitives automatically map to appropriate controls
- **Field labels from field names** - `user_name` becomes "User Name"
- **Placeholder text from doc comments** - documentation becomes UI hints
- **Secure data handling** - password fields with bcrypt hashing and memory zeroing
- **Validation system** - composable validators with error messages
- **Specialized pickers** - color, date/time, and multi-date selection
- **Reactive bindings** - forms automatically update when data changes

This crate is part of the WaterUI workspace and integrates tightly with `waterui-core` for reactivity, `waterui-controls` for base components, and `waterui-layout` for composition.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui-form = "0.1.0"

# Optional: enable serde support
waterui-form = { version = "0.1.0", features = ["serde"] }
```

## Quick Start

The most common use case is deriving `FormBuilder` on a struct:

```rust
use waterui::prelude::*;
use waterui_form::{FormBuilder, form};

#[derive(Default, Clone, Debug, FormBuilder)]
struct UserProfile {
    /// Enter your full name
    name: String,
    /// Your current age
    age: i32,
    /// Account is active
    active: bool,
}

fn profile_form() -> impl View {
    let form_binding = UserProfile::binding();
    vstack((
        form(&form_binding),
        button("Save", move || {
            tracing::debug!("Profile: {:?}", form_binding.get());
        }),
    ))
}
```

This generates a vertical stack with:
- A text field labeled "Name" with placeholder "Enter your full name"
- A stepper labeled "Age" with placeholder "Your current age"
- A toggle labeled "Active" with placeholder "Account is active"

## Core Concepts

### FormBuilder Trait

The `FormBuilder` trait is the foundation of the form system. It maps Rust types to UI components:

| Rust Type | UI Component | Description |
|-----------|--------------|-------------|
| `String`, `Str` | `TextField` | Single-line text input with optional placeholder |
| `bool` | `Toggle` | Boolean switch/checkbox |
| `i32` | `Stepper` | Integer stepper with +/- buttons |
| `f32`, `f64` | `Slider` | Numeric slider (0.0-1.0 range by default) |
| `Color` | `ColorPicker` | Platform-native color selection |
| `Date` | `DatePicker` | Date/time selection (requires `time` crate types) |
| `BTreeSet<Date>` | `MultiDatePicker` | Multiple date selection |
| `Secure` | `SecureField` | Masked password input with memory zeroing |

### The #[derive(FormBuilder)] Macro

The derive macro automatically implements `FormBuilder` by:

1. **Converting field names to labels**: `user_name` → "User Name"
2. **Extracting doc comments as placeholders**: `/// Enter email` becomes placeholder text
3. **Mapping field types to components**: Uses the table above
4. **Arranging fields in a VStack**: Creates a vertical layout of all fields

The macro requires the struct to have named fields and generates an implementation compatible with the `Project` trait for field-level reactive bindings.

### Reactive Bindings and Project

Forms use `Binding<T>` from the `nami` crate for reactive state. The `Project` trait (auto-derived alongside `FormBuilder`) allows accessing individual field bindings:

```rust
let form_binding = UserProfile::binding();
let projected = form_binding.project();

// Access individual field bindings
projected.name.set("Alice".to_string());
projected.age.set(30);

// Read entire form state
let profile = form_binding.get();
```

## Examples

### Pre-filled Form Data

Initialize a form with existing data instead of defaults:

```rust
use waterui::prelude::*;
use waterui_form::{FormBuilder, form};

#[derive(Default, Clone, Debug, FormBuilder)]
struct LoginForm {
    username: String,
    password: String,
}

fn login_view() -> impl View {
    let initial = LoginForm {
        username: "alice@example.com".to_string(),
        password: String::new(),
    };
    let form_binding = Binding::new(initial);

    vstack((
        form(&form_binding),
        button("Login", move || {
            let credentials = form_binding.get();
            tracing::debug!("Logging in as: {}", credentials.username);
        }),
    ))
}
```

### Displaying Form Values Reactively

Use projected bindings to display live form data:

```rust
use waterui::prelude::*;
use waterui_form::{FormBuilder, form};

#[derive(Default, Clone, Debug, FormBuilder)]
struct ContactForm {
    /// Your full name
    name: String,
    /// Email address
    email: String,
    /// Receive newsletter
    subscribe: bool,
}

fn contact_form_view() -> impl View {
    let form_binding = ContactForm::binding();
    let projected = form_binding.project();

    vstack((
        form(&form_binding),
        text(projected.name.map(|n| format!("Hello, {}!", n))),
        text(projected.email.map(|e| format!("Email: {}", e))),
    ))
}
```

### Secure Password Input

Use `SecureField` for sensitive data with automatic memory zeroing:

```rust
use waterui::prelude::*;
use waterui_form::secure::{Secure, SecureField, secure};

fn password_form() -> impl View {
    let password = Binding::new(Secure::default());
    let confirm = Binding::new(Secure::default());

    vstack((
        SecureField::new("Password", &password),
        secure("Confirm Password", &confirm),
        button("Create Account", move || {
            let hash = password.get().hash();
            tracing::debug!("Password hash: {}", hash);
        }),
    ))
}
```

The `Secure` type:
- Implements `Zeroize` to clear memory on drop
- Has `Debug` impl that prints `Secure(****)` instead of the actual value
- Provides `hash()` method using bcrypt with default cost
- Use `expose()` to access the underlying string when needed

### Form Validation

Compose validators to enforce rules on form fields:

```rust
use waterui::prelude::*;
use waterui_form::valid::{Validator, ValidatableView};
use regex::Regex;

fn validated_form() -> impl View {
    let age_binding = Binding::new(0i32);
    let email_binding = Binding::new(String::new());

    let age_range = 18..=100;
    let email_pattern = Regex::new(r"^[^@]+@[^@]+\.[^@]+$").unwrap();

    vstack((
        ValidatableView::new(
            Stepper::new(&age_binding).label("Age"),
            age_range,
        ),
        ValidatableView::new(
            TextField::new(&email_binding).label("Email"),
            email_pattern,
        ),
    ))
}
```

Validators can be combined:
- `.and(other)` - both must succeed
- `.or(other)` - at least one must succeed

Built-in validators:
- `Range<T>` - validates value is within range
- `Regex` - validates string matches pattern
- `Required` - validates `Option<T>` is `Some` or string is non-empty

### Date Picker

Use `DatePicker` for date and time selection:

```rust
use waterui::prelude::*;
use waterui_form::picker::{DatePicker, DatePickerType};
use time::Date;

fn event_form() -> impl View {
    let event_date = Binding::new(Date::MIN);

    vstack((
        DatePicker::new(&event_date)
            .label("Event Date")
            .ty(DatePickerType::DateHourAndMinute),
        DatePicker::new(&event_date)
            .label("Date Only")
            .ty(DatePickerType::Date),
    ))
}
```

Available picker types:
- `DatePickerType::Date` - Date only
- `DatePickerType::HourAndMinute` - Time only (hour:minute)
- `DatePickerType::HourMinuteAndSecond` - Time with seconds
- `DatePickerType::DateHourAndMinute` - Date and time (default)
- `DatePickerType::DateHourMinuteAndSecond` - Date and time with seconds

### Color Picker

Use `ColorPicker` for platform-native color selection:

```rust
use waterui::prelude::*;
use waterui_form::picker::ColorPicker;
use waterui_color::Color;

fn theme_editor() -> impl View {
    let primary_color = Binding::new(Color::rgb(0.0, 0.5, 1.0));
    let background = Binding::new(Color::rgb(1.0, 1.0, 1.0));

    vstack((
        ColorPicker::new(&primary_color).label("Primary Color"),
        ColorPicker::new(&background).label("Background"),
    ))
}
```

### Multi-Date Selection

Use `MultiDatePicker` for selecting multiple dates:

```rust
use waterui::prelude::*;
use waterui_form::picker::multi_date::MultiDatePicker;
use alloc::collections::BTreeSet;
use time::Date;

fn availability_calendar() -> impl View {
    let available_dates = Binding::new(BTreeSet::<Date>::new());

    vstack((
        MultiDatePicker::new(&available_dates)
            .label("Select Available Dates"),
        text(available_dates.map(|dates| {
            format!("Selected {} dates", dates.len())
        })),
    ))
}
```

### Manual FormBuilder Implementation

For custom layouts or specialized behavior, implement `FormBuilder` manually:

```rust
use waterui::prelude::*;
use waterui_form::FormBuilder;

struct TwoColumnForm {
    left_field: String,
    right_field: String,
}

impl FormBuilder for TwoColumnForm {
    type View = HStack<(TextField, TextField)>;

    fn view(binding: &Binding<Self>, _label: AnyView, _placeholder: Str) -> Self::View {
        let projected = binding.project();
        hstack((
            TextField::new(&projected.left_field).label("Left"),
            TextField::new(&projected.right_field).label("Right"),
        ))
    }
}
```

## API Overview

### Core Types

- **`FormBuilder`** - Trait for types that can render as form UI
- **`form()`** - Function to create a form view from a `Binding<T: FormBuilder>`

### Secure Module

- **`Secure`** - Wrapper type for sensitive strings with automatic memory zeroing
- **`SecureField`** - Password input component
- **`secure()`** - Helper function to create a `SecureField`

### Picker Module

- **`Picker`** - Generic picker for selecting from a list
- **`ColorPicker`** - Platform-native color selection
- **`DatePicker`** - Date and time selection with multiple styles
- **`MultiDatePicker`** - Multiple date selection
- **`PickerItem<T>`** - Type alias for picker items (`TaggedView<T, Text>`)

### Validation Module

- **`Validatable`** - Trait for views that can be validated
- **`Validator<T>`** - Trait for value validation logic
- **`ValidatableView<V, T>`** - Wraps a view with a validator
- **`And<A, B>`** - Combines validators with logical AND
- **`Or<A, B>`** - Combines validators with logical OR
- **`Required`** - Validator for required fields
- **`OutOfRange<T>`** - Error type for range validation
- **`NotMatch`** - Error type for regex validation

## Features

### Default Features

None. The crate works out-of-the-box with no feature flags.

### Optional Features

- **`serde`** - Enables `Serialize` and `Deserialize` derives on form types
  ```toml
  waterui-form = { version = "0.1.0", features = ["serde"] }
  ```


### Workspace Dependencies

- **`waterui-core`** - Provides `View` trait, `Binding`, `Environment`, and `AnyView`
- **`waterui-controls`** - Base components (`TextField`, `Toggle`, `Stepper`, `Slider`, `Button`)
- **`waterui-layout`** - Layout primitives (`VStack`, `HStack`, `ZStack`)
- **`waterui-text`** - Text rendering components
- **`waterui-color`** - Color type used by `ColorPicker`

### External Dependencies

- **`nami`** - Fine-grained reactivity system (provides `Binding`, `Computed`)
- **`time`** - Date/time types for `DatePicker`
- **`zeroize`** - Secure memory zeroing for `Secure` type
- **`bcrypt`** - Password hashing for `Secure::hash()`
- **`regex`** - Pattern matching for validation

### Derive Macros

The `FormBuilder` derive macro is provided by `waterui-derive`, which is automatically included when using `waterui::prelude::*`.

## Notes

- This crate is `#![no_std]` compatible (uses `extern crate alloc`)
- All code examples in this README are extracted from actual source code or documentation
- The crate follows WaterUI's layout contract system - see component source for detailed layout behavior
- Forms are rendered to native platform widgets (UIKit/AppKit on Apple, Android View on Android)
