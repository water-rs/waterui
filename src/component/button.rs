use crate::{
    attributed_string::AttributedString,
    reactive::{IntoRef, Ref},
    view::Frame,
};

pub struct Button {
    frame: Frame,
    pub label: Ref<AttributedString>,
}

impl Button {
    pub fn new(label: impl IntoRef<AttributedString>) -> Self {
        Self {
            label: label.into_ref(),
            frame: Default::default(),
        }
    }
}

native_implement_with_frame!(Button);

pub fn button(label: impl IntoRef<AttributedString>) -> Button {
    Button::new(label)
}
