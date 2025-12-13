# waterui-controls

Interactive UI controls for WaterUI applications with reactive data binding.

## Overview

`waterui-controls` provides a complete set of form controls and interactive components for WaterUI applications. Each control integrates seamlessly with WaterUI's reactive system through `Binding<T>` and `Computed<T>`, enabling automatic UI updates when data changes. Controls render to native platform widgets (UIKit/UIKit/AppKit on Apple, Android Views/Jetpack Compose on Android), providing a truly native look and feel.

This crate is part of the WaterUI workspace and is re-exported through the main `waterui` crate's prelude, so most users will access these components via `use waterui::prelude::*` rather than depending on this crate directly.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui-controls = "0.1.0"
```

Or use the main `waterui` crate which re-exports all controls:

```toml
[dependencies]
waterui = "0.1.0"
```

## Quick Start

```rust
use waterui::prelude::*;
use waterui::reactive::binding;

let name = binding("");
let age = binding(25);
let enabled = binding(true);
let volume = binding(0.5);

vstack((
    // Text input with label
    field("Name", &name),

    // Numeric stepper
    stepper(&age).label("Age").range(0..=120),

    // Boolean toggle
    toggle("Enabled", &enabled),

    // Range slider
    slider(0.0..=1.0, &volume).label("Volume"),

    // Submit button
    button("Submit").action(|| {
        tracing::debug!("Form submitted!");
    }),
))
```

## Core Concepts

### Reactive Bindings

All controls accept reactive bindings that enable two-way data synchronization:

- **`Binding<T>`**: Mutable reactive state that updates both when the UI changes and when modified programmatically
- **`Computed<T>`**: Read-only derived values that automatically update when dependencies change

```rust
let count = binding(0);

// UI updates when count changes programmatically
count.set(5);

// count updates when user interacts with stepper
stepper(&count)
```

### Layout Behavior

Controls have different layout characteristics:

- **Horizontal stretch** (`TextField`, `Slider`, `Toggle`, `Stepper`): Expand to fill available width
- **Content-sized** (`Button`): Size to fit their content
- **Dynamic** (`Progress`): Linear style stretches, circular style is content-sized

## Components

### Button

Triggers actions when clicked. Supports multiple visual styles.

```rust
use waterui::prelude::*;

// Basic button
button("Click me").action(|| {
    tracing::debug!("Button clicked!");
});

// Styled button
button("Submit")
    .style(ButtonStyle::BorderedProminent)
    .action(|| {
        tracing::debug!("Form submitted!");
    });

// Link-style button
button("Learn more")
    .style(ButtonStyle::Link)
    .action(|| {
        tracing::debug!("Opening link...");
    });
```

**Available styles**: `Automatic` (default), `Plain`, `Link`, `Borderless`, `Bordered`, `BorderedProminent`

### Toggle

A switch control for boolean values.

```rust
use waterui::prelude::*;
use waterui::reactive::binding;

let wifi_enabled = binding(true);

// Toggle with label
toggle("Wi-Fi", &wifi_enabled)

// Toggle without label (just the switch)
Toggle::new(&wifi_enabled)
```

### Slider

Continuous value selection within a range.

```rust
use waterui::prelude::*;
use waterui::reactive::binding;

let brightness = binding(0.5);

// Basic slider (0.0 to 1.0)
slider(0.0..=1.0, &brightness)
    .label("Brightness")
    .min_value_label("Dark")
    .max_value_label("Bright")

// Volume control
let volume = binding(50.0);
slider(0.0..=100.0, &volume).label(text!("{:.0}%", volume))
```

### Stepper

Increment or decrement integer values.

```rust
use waterui::prelude::*;
use waterui::reactive::binding;

let quantity = binding(1);

// Basic stepper (shows current value)
stepper(&quantity)

// Stepper with label and constraints
stepper(&quantity)
    .label("Items")
    .range(1..=10)
    .step(1)

// Custom value formatting
stepper(&quantity)
    .value_formatter(|n| format!("{} items", n))
```

### TextField

Single-line or multi-line text input.

```rust
use waterui::prelude::*;
use waterui::reactive::binding;

let email = binding("");
let bio = binding("");

// Single-line text field
TextField::new(&email)
    .prompt("Enter your email")

// With label (convenience function)
field("Email", &email)

// Multi-line text field
TextField::new(&bio)
    .line_limit(5)
    .prompt("Tell us about yourself")
```

**Keyboard types**: `Text` (default), `Email`, `URL`, `Number`, `PhoneNumber`

### RichTextEditor

Multi-line text editor with styled text support.

```rust
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::text::styled::StyledStr;

let content = binding(StyledStr::default());

RichTextEditor::new(&content)
    .placeholder("Start writing...")
    .disable_line_limit()
```

## Examples

### Form with Validation

```rust
use waterui::prelude::*;
use waterui::reactive::binding;

let username = binding("");
let age = binding(18);
let terms_accepted = binding(false);

vstack((
    field("Username", &username)
        .prompt("Enter username"),

    stepper(&age)
        .label("Age")
        .range(13..=120),

    toggle("Accept Terms", &terms_accepted),

    button("Register")
        .style(ButtonStyle::BorderedProminent)
        .action(|| {
            tracing::debug!("Registration submitted");
        }),
))
.padding_with(EdgeInsets::all(16.0))
```

### Settings Panel

```rust
use waterui::prelude::*;
use waterui::reactive::binding;

let dark_mode = binding(false);
let notifications = binding(true);
let volume = binding(0.7);
let font_size = binding(16);

scroll(
    vstack((
        text("Settings").size(24.0).bold(),

        Divider,

        toggle("Dark Mode", &dark_mode),
        toggle("Notifications", &notifications),

        Divider,

        slider(0.0..=1.0, &volume)
            .label("Volume"),

        stepper(&font_size)
            .label("Font Size")
            .range(12..=24)
            .step(2),
    ))
    .padding_with(EdgeInsets::all(16.0))
)
```

### Reactive Counter

```rust
use waterui::prelude::*;
use waterui::reactive::binding;

let count = binding(0);

vstack((
    text!("Count: {}", count).size(32.0),

    hstack((
        button("Decrement").action_with(&count, |count| {
            count.update(|n| n - 1);
        }),

        stepper(&count).range(0..=100),

        button("Increment").action_with(&count, |count| {
            count.update(|n| n + 1);
        }),
    )),

    button("Reset")
        .style(ButtonStyle::Borderless)
        .action_with(&count, |count| {
            count.set(0);
        }),
))
```

### Loading States

```rust
use waterui::prelude::*;
use waterui::reactive::binding;

let progress = binding(0.0);
let is_loading = binding(false);

vstack((
    // Progress bar
    progress(progress.clone()),

    // Indeterminate loading spinner
    loading().visible(is_loading.clone()),

    // Control buttons
    hstack((
        button("Start").action_with(&is_loading, |loading| {
            loading.set(true);
        }),
        button("Stop").action_with(&is_loading, |loading| {
            loading.set(false);
        }),
    )),
))
```

## API Overview

### Controls

- **`Button`**: Clickable button with customizable styles and actions
- **`Toggle`**: Boolean switch control
- **`Slider`**: Continuous range selector (f64)
- **`Stepper`**: Discrete numeric adjuster (i32)
- **`TextField`**: Single/multi-line text input
- **`RichTextEditor`**: Styled text editor

### Convenience Functions

- **`button(label)`**: Create a button with a label
- **`toggle(label, binding)`**: Create a labeled toggle
- **`slider(range, binding)`**: Create a slider
- **`stepper(binding)`**: Create a stepper
- **`field(label, binding)`**: Create a labeled text field

### Enums

- **`ButtonStyle`**: Visual styles for buttons (Automatic, Plain, Link, Borderless, Bordered, BorderedProminent)
- **`KeyboardType`**: Keyboard types for text fields (Text, Email, URL, Number, PhoneNumber)

## Features

This crate currently has no optional features. All components are included by default.

## Platform Notes

All controls render to native platform widgets:

- **Apple platforms**: UIKit/SwiftUI components (UIButton, UITextField, UISwitch, UISlider, etc.)
- **Android**: Native Views/Jetpack Compose (Button, TextField, Switch, Slider, etc.)

This ensures controls match the platform's design language and accessibility features automatically.
