# waterui-form-derive

Derive macro for the `FormBuilder` trait in WaterUI forms.

This crate provides a `#[derive(FormBuilder)]` macro that automatically generates form implementations for structs.

## Usage

Add the derive macro to any struct with supported field types:

```rust
use waterui_form::FormBuilder;

#[derive(FormBuilder)]
pub struct UserForm {
    username: String,
    age: i32,
    remember_me: bool,
}
```

## Supported Types

- `String`, `&str` → `TextField`
- `bool` → `Toggle`
- Integer types (`i32`, `u32`, etc.) → `Stepper`
- Float types (`f32`, `f64`) → `Slider`
- `Color` → `ColorPicker`