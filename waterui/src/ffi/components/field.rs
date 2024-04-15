use crate::component;
use crate::ffi::{binding::BindingStr, computed::ComputedStr};

#[repr(C)]
pub struct TextField {
    label: ComputedStr,
    value: BindingStr,
    prompt: ComputedStr,
}

impl From<component::TextField> for TextField {
    fn from(value: component::TextField) -> Self {
        Self {
            label: value._label.into(),
            value: value._value.into(),
            prompt: value._prompt.into(),
        }
    }
}

impl_view!(
    TextField,
    waterui_view_force_as_field,
    waterui_view_field_id
);
