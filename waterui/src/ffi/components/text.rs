use waterui_reactive::Computed;

use crate::ffi::ComputedUtf8Data;

#[repr(C)]
pub struct Text {
    content: ComputedUtf8Data,
}

impl From<crate::component::Text> for Text {
    fn from(value: crate::component::Text) -> Self {
        Self {
            content: value._content.into(),
        }
    }
}

impl From<Text> for crate::component::Text {
    fn from(value: Text) -> Self {
        Self::new(Computed::from(value.content))
    }
}

impl_view!(Text, waterui_view_force_as_text, waterui_view_text_id);
