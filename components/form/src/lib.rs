//! Form components for `WaterUI`.
//!
//! This crate provides reusable form elements and builder traits for `WaterUI` applications.

extern crate alloc;

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
use waterui_color::Color;
use waterui_core::{Binding, Str, View};

/// The result of building a form field, containing the view and its bound value.
#[derive(Debug, Clone)]
pub struct FormBuilderResult<T: 'static, V: View> {
    /// The view representing the form field.
    pub view: V,
    /// The binding to the value of the form field.
    pub value: Binding<T>,
}

/// Trait for types that can be constructed as form fields.
pub trait FormBuilder: Sized {
    /// Builds a form field with the given label, returning the view and value binding.
    fn build(label: impl View) -> FormBuilderResult<Self, impl View>;
}

macro_rules! impl_form_builder {
    ($ty:ty,$view:ty) => {
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

impl_form_builder!(Str, TextField);
impl_form_builder!(i32, Stepper);
impl_form_builder!(bool, Toggle);
impl_form_builder!(Color, picker::ColorPicker);
