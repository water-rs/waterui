use crate::AttributedString;
pub use waterui_core::component::*;
mod async_view;
pub use async_view::AsyncView;
use waterui_core::view::IntoViews;
mod date_picker;
pub use date_picker::DatePicker;
mod image;

pub fn text(text: impl Into<AttributedString>) -> Text {
    Text::new(text)
}

pub fn button(label: impl Into<AttributedString>) -> Button {
    Button::new(label)
}

pub fn vstack(contents: impl IntoViews) -> VStack {
    VStack::new(contents)
}

pub fn gstack(contents: impl IntoViews) -> HStack {
    HStack::new(contents)
}
