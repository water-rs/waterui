use crate::ffi::computed::{ComputedBool, ComputedStr};

#[repr(C)]
pub struct Text {
    content: ComputedStr,
    selection: ComputedBool,
}

impl From<crate::component::Text> for Text {
    fn from(value: crate::component::Text) -> Self {
        Self {
            content: value._content.into(),
            selection: value._selection.into(),
        }
    }
}

impl_view!(
    crate::component::Text,
    Text,
    waterui_view_force_as_text,
    waterui_view_text_id
);
