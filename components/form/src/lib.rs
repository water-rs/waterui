extern crate alloc;

pub mod text_field;
pub mod toggle;
#[doc(inline)]
pub use text_field::{TextField, field};
#[doc(inline)]
pub use toggle::{Toggle, toggle};
pub mod slider;
pub use slider::Slider;
pub mod picker;
pub mod stepper;
#[doc(inline)]
pub use stepper::{Stepper, stepper};
use waterui_color::Color;
use waterui_core::{Binding, Str, View};

pub struct FormBuilderResult<T: 'static, V: View> {
    pub view: V,
    pub value: Binding<T>,
}

pub trait FormBuilder: Sized {
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
