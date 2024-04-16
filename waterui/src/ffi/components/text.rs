use crate::ffi::computed::ComputedStr;

#[repr(C)]
pub struct Text {
    content: ComputedStr,
}

impl From<crate::component::Text> for Text {
    fn from(value: crate::component::Text) -> Self {
        Self {
            content: value._content.into(),
        }
    }
}

impl_view!(
    crate::component::Text,
    Text,
    waterui_view_force_as_text,
    waterui_view_text_id
);
