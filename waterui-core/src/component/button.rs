use crate::{attributed_string::AttributedString, reactive::IntoReactive, view::IntoView, BoxView};
use std::fmt::Display;

use super::Text;

pub struct Button {
    pub(crate) label: BoxView,
    pub(crate) action: Box<dyn Fn() + Send + Sync>,
}

impl Default for Button {
    fn default() -> Self {
        Self {
            label: Box::new(()),
            action: Box::new(|| {}),
        }
    }
}

impl Button {
    pub fn new(label: impl IntoReactive<AttributedString>) -> Self {
        Self::default().label(Text::new(label))
    }

    pub fn action(mut self, action: impl Fn() + Send + Sync + 'static) -> Self {
        self.action = Box::new(action);
        self
    }

    pub fn display(v: impl Display) -> Self {
        Self::new(v.to_string())
    }

    pub fn label(mut self, label: impl IntoView) -> Self {
        self.label = Box::new(label.into_view());
        self
    }
}

raw_view!(Button);
