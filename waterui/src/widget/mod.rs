use crate::AttributedString;
pub use waterui_core::component::*;
mod async_view;
pub use async_view::AsyncView;
use waterui_core::{
    reactive::IntoReactive,
    view::{IntoView, IntoViews},
};
mod condition;
pub use condition::Condition;
mod date_picker;

pub use date_picker::DatePicker;
mod image;

pub fn when<ContentBuilder, Content>(
    condition: impl IntoReactive<bool>,
    content: ContentBuilder,
) -> Condition<ContentBuilder, ()>
where
    ContentBuilder: Fn() -> Content,
    Content: IntoView,
{
    Condition::new(condition.into_reactive(), content)
}

pub fn text(text: impl IntoReactive<AttributedString>) -> Text {
    Text::new(text)
}

pub fn button(label: impl IntoView, action: impl Fn() + Send + Sync + 'static) -> Button {
    Button::new(label, action)
}

pub fn vstack(contents: impl IntoViews) -> VStack {
    VStack::new(contents)
}

pub fn gstack(contents: impl IntoViews) -> HStack {
    HStack::new(contents)
}
