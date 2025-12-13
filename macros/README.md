# waterui-derive

Procedural macros for the WaterUI framework, providing automatic derive implementations and code generation for forms, reactive projections, string formatting, and hot reload functionality.

## Overview

This crate is the macro engine behind WaterUI's ergonomic APIs. It provides four main categories of macros:

1. **Form Generation** - Automatically generate UI forms from Rust structs with `#[derive(FormBuilder)]` and `#[form]`
2. **Reactive Projections** - Decompose struct bindings into per-field bindings with `#[derive(Project)]`
3. **Formatted Strings** - Create reactive formatted strings with the `s!` macro
4. **Hot Reload** - Enable per-function hot reloading with `#[hot_reload]`

This crate is typically accessed through the main `waterui` crate via `use waterui::prelude::*;` rather than being used directly.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui = "0.1"
```

The derive macros are automatically available when you import the prelude:

```rust
use waterui::prelude::*;
```

## Core Macros

### Form Builder System

#### `#[derive(FormBuilder)]`

Automatically implements the `FormBuilder` trait for structs, generating form UI components based on field types.

**Type-to-Component Mapping:**

| Rust Type | UI Component | Description |
|-----------|--------------|-------------|
| `String`, `Str` | `TextField` | Text input with optional placeholder |
| `bool` | `Toggle` | Boolean switch/checkbox |
| `i8`..`isize`, `u8`..`usize` | `Stepper` | Numeric stepper with increment/decrement |
| `f32`, `f64` | `Slider` | Slider (0.0-1.0 range) |
| `Color` | `ColorPicker` | Color selection widget |

**Example from `/Users/lexoliu/Coding/waterui/examples/form/src/lib.rs`:**

```rust
use waterui::prelude::*;
use waterui::reactive::binding;

#[form]
struct RegistrationForm {
    /// Full name of the user
    full_name: String,
    /// Email address for account
    email: String,
    /// Age in years
    age: i32,
    /// Whether to receive marketing emails
    newsletter: bool,
    /// Preferred volume level (0.0 - 1.0)
    volume: f64,
}

fn registration_view() -> impl View {
    let registration = RegistrationForm::binding();

    vstack((
        form(&registration),
        // Live preview using projected bindings
        hstack(("Name: ", text!("{}", registration.project().full_name))),
        hstack(("Email: ", text!("{}", registration.project().email))),
        hstack(("Age: ", text!("{}", registration.project().age))),
    ))
}
```

**How it works:**

- Converts `snake_case` field names to "Title Case" labels (e.g., `full_name` → "Full Name")
- Uses doc comments as placeholder text for text fields
- Automatically calls `Project::project()` on the binding to access individual fields
- Returns a `VStack` containing all generated form controls

#### `#[form]`

Convenience attribute macro that combines multiple common derives for form structs.

**Equivalent to:**

```rust
#[derive(Default, Clone, Debug, FormBuilder, Project)]
```

**Example from `/Users/lexoliu/Coding/waterui/examples/form/src/lib.rs`:**

```rust
#[form]
struct AppSettings {
    /// Application theme brightness
    brightness: f64,
    /// Enable dark mode
    dark_mode: bool,
    /// Font size multiplier
    font_scale: f32,
    /// Auto-save interval (minutes)
    auto_save_minutes: i32,
    /// Enable notifications
    notifications_enabled: bool,
}
```

### Reactive Projection

#### `#[derive(Project)]`

Implements the `Project` trait, enabling decomposition of struct bindings into separate bindings for each field. This is essential for reactive form handling where each field needs independent mutation.

**Supports:**
- Named structs
- Tuple structs
- Unit structs

**Example from `/Users/lexoliu/Coding/waterui/derive/src/lib.rs` documentation:**

```rust
use waterui::reactive::{Binding, binding, project::Project};
use waterui_macros::Project;

#[derive(Project, Clone)]
struct Person {
    name: String,
    age: u32,
}

let person_binding: Binding<Person> = binding(Person {
    name: "Alice".to_string(),
    age: 30,
});

let projected = person_binding.project();
projected.name.set("Bob".to_string());
projected.age.set(25u32);

let person = person_binding.get();
assert_eq!(person.name, "Bob");
assert_eq!(person.age, 25);
```

**Generated code:**

For a struct `MyForm`, the macro generates:
1. A `MyFormProjected` struct with each field wrapped in `Binding<T>`
2. A `Project` implementation that creates mapped bindings for each field
3. Proper lifetime bounds (`'static`) on generic parameters

### String Formatting

#### `s!` - Reactive String Formatting

Function-like procedural macro for creating formatted string signals with automatic variable capture. Powers the `text!` macro in WaterUI.

**Features:**
- Automatic variable capture from format string placeholders
- Positional and named argument support
- Reactive updates when dependencies change
- Supports up to 4 variables/arguments

**Usage patterns:**

```rust
use waterui::reactive::{binding, constant};
use waterui::s;

let name = binding("Alice");
let age = binding(25);

// Named variable capture (automatic)
let msg = s!("Hello {name}, you are {age} years old");

// Positional arguments
let msg2 = s!("Hello {}, you are {}", name, age);

// Static strings (returns constant signal)
let static_msg = s!("No variables here");
```

**Used internally by `text!` macro:**

```rust
// From /Users/lexoliu/Coding/waterui/src/lib.rs
#[macro_export]
macro_rules! text {
    ($($arg:tt)*) => {
        {
            $crate::text::Text::new($crate::s!($($arg)*))
        }
    };
}
```

**Implementation details:**
- Uses `zip` combinator to merge multiple reactive signals
- Delegates to `__format!` macro for actual formatting
- Returns `Computed<Str>` for reactive values, `Constant<Str>` for static strings
- Validates format string at compile time (detects mismatched argument counts)

### Hot Reload

#### `#[hot_reload]`

Attribute macro that enables per-function hot reloading during development. When the library is rebuilt via `water run`, only the annotated function is updated without restarting the app.

**Example from `/Users/lexoliu/Coding/waterui/src/debug/hot_reload.rs` documentation:**

```rust
use waterui::prelude::*;

#[hot_reload]
fn sidebar() -> impl View {
    vstack((
        text("Sidebar"),
        text("Content"),
    ))
}

fn main() -> impl View {
    hstack((
        sidebar(),  // This view will hot reload when modified
        content_panel(),
    ))
}
```

**How it works:**

1. Wraps the function body in a `HotReloadView` that registers with the hot reload system
2. Generates a C-exported symbol (when built with `--cfg waterui_hot_reload_lib`):
   ```rust
   #[unsafe(no_mangle)]
   pub unsafe extern "C" fn waterui_hot_reload_sidebar() -> *mut ()
   ```
3. The CLI loads this symbol from the rebuilt dylib and updates all registered handlers
4. Uses `module_path!()` + function name as unique identifier (e.g., `"my_crate::sidebar"`)

**Requirements:**
- Function must return `impl View`
- Enable hot reload with `WATERUI_HOT_RELOAD_HOST` and `WATERUI_HOT_RELOAD_PORT` environment variables (set by `water run`)
- Build hot reload library with `RUSTFLAGS="--cfg waterui_hot_reload_lib" cargo build`

## API Overview

### Derive Macros

- **`FormBuilder`** - Generate form UI from struct definition
- **`Project`** - Enable field-level reactive bindings

### Attribute Macros

- **`#[form]`** - Convenience macro for form structs (combines multiple derives)
- **`#[hot_reload]`** - Enable per-function hot reloading

### Function-like Macros

- **`s!(...)`** - Create reactive formatted strings with automatic variable capture

## Features

This is a proc-macro crate with no optional features. All macros are always available.

## Implementation Notes

- **Rust Edition**: 2024
- **Dependencies**: `syn ^2.0`, `quote ^1.0`, `proc-macro2 ^1.0`
- **Workspace Integration**: Part of the WaterUI workspace, follows workspace lints (clippy pedantic + nursery)

### Compile-Time Validation

The macros perform extensive compile-time validation:

- `FormBuilder`: Requires named struct fields
- `s!`: Detects mismatched placeholder/argument counts, mixed positional/named usage
- `Project`: Rejects enums and unions (only works with structs)
- `form`: Requires structs with named fields

### Code Generation Strategy

**Form Builder:**
- Generates type-safe view types using the builder pattern
- Uses `Quote::quote!` for generating trait implementations
- Automatically handles `Project` trait bounds for field access

**Project:**
- Creates a parallel struct with `Projected` suffix
- Uses `Binding::mapping` for bidirectional field synchronization
- Adds `'static` bounds to generic parameters

**String Formatting:**
- Analyzes format strings with custom parser
- Generates optimized code paths for 0-4 variables
- Uses nested `zip` calls for multiple signal combination

## Development

To test changes to the macros:

```bash
# Run tests (uses waterui as dev-dependency)
cargo test -p waterui-derive

# Check macro expansion
cargo expand --package waterui-derive

# Test in a real project
cargo install --path cli
water create --playground --name macro-test
# Add #[form] to a struct and run
water run --platform ios
```
