use crate::{attributed_string::AttributedString, reactive::IntoReactive, view::IntoView, BoxView};
use std::fmt::Display;

use super::Text;

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
