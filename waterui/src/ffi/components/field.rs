use crate::component::text_field::RawTextField;
use crate::ffi::{binding::BindingStr, computed::ComputedStr};

use super::AnyView;

#[repr(C)]
pub struct TextField {
    label: AnyView,
    value: BindingStr,
    prompt: ComputedStr,
}

impl From<RawTextField> for TextField {
    fn from(value: RawTextField) -> Self {
        Self {
            label: value._label.into(),
            value: value._value.into(),
            prompt: value._prompt.into(),
        }
    }
}

impl_view!(
    RawTextField,
    TextField,
    waterui_view_force_as_field,
    waterui_view_field_id
);
