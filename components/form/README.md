# WaterUI Form Components

Text fields, toggles, sliders, steppers, and color picker, with a derive-based form builder.

## Components

### TextField

```rust,ignore
use nami::binding;
use waterui_form::{TextField, field};
use waterui_text::text;
use waterui_str::Str;

let name = binding(Str::from(""));
let input = TextField::new(&name)
    .label(text("Name"))
    .prompt("John Doe");

// Helper
let same = field(text("Name"), &name);
```

### Toggle

```rust,ignore
use nami::binding;
use waterui_form::{Toggle, toggle};
use waterui_text::text;

let enabled = binding(true);
let t = Toggle::new(&enabled).label(text("Enable"));
let t2 = toggle(text("Enable"), &enabled);
```

### Slider

```rust,ignore
use nami::binding;
use waterui_form::Slider;
use waterui_text::text;

let volume = binding(0.5);
let s = Slider::new(0.0..=1.0, &volume).label(text("Volume"));
```

### Stepper

```rust,ignore
use nami::binding;
use waterui_form::{Stepper, stepper};
use waterui_text::text;

let count = binding(0);
let a = Stepper::new(&count).step(1).range(0..=10).label(text("Count"));
let b = stepper(&count).label(text("Count"));
```

### Color Picker

```rust,ignore
use nami::binding;
use waterui_core::Color;
use waterui_form::picker::ColorPicker;
use waterui_text::text;

let color = binding(Color::default());
let picker = ColorPicker::new(&color).label(text("Color"));
```

## Derive: `FormBuilder`

Generate a form view from a struct:

```rust,ignore
use waterui_form::{FormBuilder, form};
use waterui_layout::stack::vstack;
use waterui_text::text;
use waterui_core::{View, Environment};

#[derive(Default, Clone, Debug, FormBuilder)]
struct Profile { name: String, age: i32, active: bool }

fn view() -> impl View {
    let binding = Profile::binding();
    vstack((
        form(&binding),
        text!("Hello, {}!", binding.project().name),
    ))
}
```

The derive maps common types to components: `String`/`Str` → `TextField`, `bool` → `Toggle`, numeric → `Stepper`, `f64` → `Slider`, `Color` → `ColorPicker`.
