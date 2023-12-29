use crate::{view::IntoView, BoxView};
use std::fmt::Display;

pub struct Button {
    pub(crate) label: BoxView,
    pub(crate) action: Box<dyn Fn() + Send + Sync>,
}

impl Button {
    pub fn new(label: impl IntoView, action: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            label: label.into_boxed_view(),
            action: Box::new(action),
        }
    }

    pub fn display(v: impl Display, action: impl Fn() + Send + Sync + 'static) -> Self {
        Self::new(v.to_string(), action)
    }
}

raw_view!(Button);

pub fn button(label: impl IntoView, action: impl Fn() + Send + Sync + 'static) -> Button {
    Button::new(label, action)
}
