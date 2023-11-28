use serde::{Deserialize, Serialize};

use crate::widget;
use crate::{attributed_string::AttributedString, utils::Background, view::Edge};
use std::fmt::Display;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[widget]
pub struct Button {
    pub background: Background,
    pub padding: Edge,
    pub label: AttributedString,
}

impl Button {
    pub fn new(label: impl Into<AttributedString>) -> Self {
        Self {
            label: label.into(),
            background: Background::default(),
            padding: Edge::default(),
        }
    }

    pub fn display(v: impl Display) -> Self {
        Self::new(v.to_string())
    }

    pub fn padding(mut self, padding: Edge) -> Self {
        self.padding = padding;
        self
    }

    pub fn background(mut self, background: impl Into<Background>) -> Self {
        self.background = background.into();
        self
    }
}

native_implement!(Button);

pub fn button(label: impl Into<AttributedString>) -> Button {
    Button::new(label)
}
