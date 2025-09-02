//! Form components for `WaterUI`.
//!
//! This crate provides reusable form elements and builder traits for `WaterUI` applications.

#![no_std]
extern crate alloc;

use alloc::string::String;

#[doc(inline)]
pub use waterui_form_derive::FormBuilder;

/// Text field form component module.
pub mod text_field;
/// Toggle (switch) form component module.
pub mod toggle;
#[doc(inline)]
pub use text_field::{TextField, field};
#[doc(inline)]
pub use toggle::{Toggle, toggle};
/// Slider form component module.
pub mod slider;
pub use slider::Slider;
/// Picker form component module.
pub mod picker;
/// Stepper form component module.
pub mod stepper;
#[doc(inline)]
pub use stepper::{Stepper, stepper};
use waterui_core::{Binding, Color, Str, View};

/// Trait for types that can be automatically converted to form UI components.
///
/// This trait is implemented for types that can be rendered as form fields,
/// either manually or through the `#[derive(FormBuilder)]` macro.
pub trait FormBuilder: Sized {
    /// The view type that represents this form field.
    type View: View;

    /// Creates a view representation of this form field bound to the given binding.
    fn view(binding: &Binding<Self>) -> Self::View;

    /// Creates a new binding with the default value for this form.
    #[must_use]
    fn binding() -> Binding<Self>
    where
        Self: Default + Clone,
    {
        Binding::default()
    }
}

macro_rules! impl_form_builder {
    ($ty:ty,$view:ty) => {
        impl FormBuilder for $ty {
            type View = $view;
            fn view(binding: &Binding<Self>) -> Self::View {
                <$view>::new(binding)
            }
        }
    };
}

impl_form_builder!(Str, TextField);
impl_form_builder!(i32, Stepper);
impl_form_builder!(bool, Toggle);
impl_form_builder!(Color, picker::ColorPicker);

/// Creates a form view from a binding to a type that implements `FormBuilder`.
///
/// This is the main entry point for creating forms from structs that derive `FormBuilder`.
#[must_use]
pub fn form<T: FormBuilder>(binding: &Binding<T>) -> T::View {
    T::view(binding)
}

/// Example form struct demonstrating the `FormBuilder` derive macro.
///
/// This struct shows how the `FormBuilder` derive macro can automatically
/// generate form implementations for structs with supported field types.
#[derive(Default, Clone, Debug)]
pub struct Form {
    /// The user's username
    pub username: String, // will be a text field
    /// The user's password
    pub password: String, // will be a text field with password mode
    /// Whether to remember the user
    pub remember_me: bool, // will be a toggle
    /// The user's age
    pub age: i32, // will be a stepper
}

/// Secure form components for handling sensitive data like passwords.
pub mod secure;
pub use secure::{SecureField, secure};
