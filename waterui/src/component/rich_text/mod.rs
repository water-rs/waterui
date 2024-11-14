pub mod attributed;

use core::ops::Add;

use waterui_core::{view::ConfigurableView, View};
use waterui_reactive::{zip::FlattenMap, ComputeExt, Computed};
use waterui_str::Str;

use crate::component::{
    rich_text::attributed::{AttributedString, AttributedStringNode},
    text::{Font, TextConfig},
    Text,
};

#[derive(Debug)]
pub struct RichText {
    content: Computed<AttributedString>,
}

impl RichText {
    pub fn from_markdown() {}
}

impl View for RichText {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        todo!()
    }
}

impl From<Text> for RichText {
    fn from(value: Text) -> Self {
        let config = value.config();
        Self {
            content: config
                .content
                .zip(config.font)
                .flatten_map(|content, font| AttributedStringNode::text(content, font).into())
                .computed(),
        }
    }
}

impl Add<Text> for Text {
    type Output = RichText;
    fn add(self, rhs: Text) -> Self::Output {
        let left = self.config();
        let right = rhs.config();
        let left = left.content.zip(left.font);
        let right = right.content.zip(right.font);

        let content = left
            .zip(right)
            .flatten_map(move |left: (Str, Font), right: (Str, Font)| {
                AttributedStringNode::text(left.0, left.1)
                    + AttributedStringNode::text(right.0, right.1)
            })
            .computed();
        RichText { content }
    }
}

impl Add<Text> for RichText {
    type Output = RichText;
    fn add(self, rhs: Text) -> Self::Output {
        let TextConfig { content, font, .. } = rhs.config();
        let computed = self
            .content
            .zip(content)
            .zip(font)
            .flatten_map(|rich, content, font| rich + AttributedStringNode::text(content, font))
            .computed();
        RichText { content: computed }
    }
}

impl Add<RichText> for RichText {
    type Output = RichText;
    fn add(self, rhs: RichText) -> Self::Output {
        Self {
            content: self
                .content
                .zip(rhs.content)
                .flatten_map(|left, right| left + right)
                .computed(),
        }
    }
}
