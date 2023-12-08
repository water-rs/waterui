use crate::{attributed_string::AttributedString, layout::Edge};
use std::fmt::Display;

pub struct Button {
    padding: Edge,
    pub(crate) label: AttributedString,
    pub(crate) action: Box<dyn Fn()>,
}

impl Button {
    pub fn new(label: impl Into<AttributedString>) -> Self {
        Self {
            label: label.into(),
            padding: Edge::default(),
            action: Box::new(|| {}),
        }
    }

    pub fn action(mut self, f: impl Fn() + 'static) -> Self {
        self.action = Box::new(f);
        self
    }

    pub fn display(v: impl Display) -> Self {
        Self::new(v.to_string())
    }

    pub fn padding(mut self, padding: Edge) -> Self {
        self.padding = padding;
        self
    }

    pub fn label(mut self, label: AttributedString) -> Self {
        self.label = label;
        self
    }
}

native_implement!(Button);

pub fn button(label: impl Into<AttributedString>) -> Button {
    Button::new(label)
}
