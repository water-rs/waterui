use crate::{attributed_string::AttributedString, layout::Edge, view::IntoView, BoxView};
use std::fmt::Display;

use super::Text;

pub struct Button {
    padding: Edge,
    pub(crate) label: BoxView,
    pub(crate) action: Box<dyn Fn()>,
}

impl Default for Button {
    fn default() -> Self {
        Self {
            label: Box::new(()),
            padding: Edge::default(),
            action: Box::new(|| {}),
        }
    }
}

impl Button {
    pub fn new(label: impl Into<AttributedString>) -> Self {
        Self::default().label(Text::new(label))
    }

    pub fn action(mut self, action: impl Fn() + 'static) -> Self {
        self.action = Box::new(action);
        self
    }

    pub fn display(v: impl Display) -> Self {
        Self::new(v.to_string())
    }

    pub fn padding(mut self, padding: Edge) -> Self {
        self.padding = padding;
        self
    }

    pub fn label(mut self, label: impl IntoView) -> Self {
        self.label = Box::new(label.into_view());
        self
    }
}

raw_view!(Button);
