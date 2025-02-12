use core::ops::{Add, AddAssign};

use alloc::vec::Vec;
use waterui_str::Str;

use crate::component::text::Font;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AttributedString {
    nodes: Vec<AttributedStringNode>,
}

impl AttributedString {
    pub const fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    pub fn push(&mut self, node: AttributedStringNode) {
        self.nodes.push(node);
    }

    pub fn push_text(&mut self, content: Str, font: Font) {
        self.push(AttributedStringNode::text(content, font));
    }
}

impl Extend<AttributedStringNode> for AttributedString {
    fn extend<T: IntoIterator<Item = AttributedStringNode>>(&mut self, iter: T) {
        self.nodes.extend(iter);
    }
}

impl AddAssign<AttributedStringNode> for AttributedString {
    fn add_assign(&mut self, rhs: AttributedStringNode) {
        self.nodes.push(rhs);
    }
}

impl Add<AttributedStringNode> for AttributedString {
    type Output = Self;
    fn add(mut self, rhs: AttributedStringNode) -> Self::Output {
        self += rhs;
        self
    }
}

impl From<AttributedStringNode> for AttributedString {
    fn from(value: AttributedStringNode) -> Self {
        let mut result = AttributedString::new();
        result.push(value);
        result
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AttributedStringNode {
    Text { content: Str, font: Font },
    Link { content: Str, url: Str },
}

impl AttributedStringNode {
    pub fn text(content: Str, font: Font) -> Self {
        Self::Text { content, font }
    }

    pub fn link(content: Str, url: Str) -> Self {
        Self::Link { content, url }
    }
}

impl From<Str> for AttributedStringNode {
    fn from(value: Str) -> Self {
        Self::text(value, Font::default())
    }
}

impl Add for AttributedStringNode {
    type Output = AttributedString;
    fn add(self, rhs: Self) -> Self::Output {
        let mut result = AttributedString::new();
        result.extend([self, rhs]);
        result
    }
}

impl AddAssign for AttributedString {
    fn add_assign(&mut self, mut rhs: Self) {
        self.nodes.append(&mut rhs.nodes);
    }
}

impl AddAssign<Str> for AttributedString {
    fn add_assign(&mut self, rhs: Str) {
        self.nodes.push(rhs.into());
    }
}

impl Add for AttributedString {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        let mut output = self.clone();
        output += rhs;
        output
    }
}
