use std::borrow::Cow;
use std::fmt::{Display, Write};
use std::{
    ops::{Add, Bound, Deref, Range, RangeBounds},
    slice::Iter,
};

use itertools::Itertools;
use serde::{Deserialize, Serialize};

use crate::view::Size;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AttributedString {
    text: String,
    attributes: Vec<(Range<usize>, Attribute)>,
}

impl Display for AttributedString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.text.as_str())
    }
}

impl<T: Into<String>> From<T> for AttributedString {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl Deref for AttributedString {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        self.text.deref()
    }
}

impl AttributedString {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            attributes: Vec::new(),
        }
    }

    pub fn attribute(
        mut self,
        range: impl RangeBounds<usize>,
        attribute: impl Into<Attribute>,
    ) -> Self {
        self.set_attribute(range, attribute);
        self
    }

    pub fn set_attribute(
        &mut self,
        range: impl RangeBounds<usize>,
        attribute: impl Into<Attribute>,
    ) {
        let right;
        let left = match range.start_bound() {
            Bound::Included(v) => *v,
            Bound::Excluded(v) => v - 1,
            Bound::Unbounded => 0,
        };
        match range.end_bound() {
            Bound::Included(v) => right = *v,
            Bound::Excluded(v) => right = v - 1,
            Bound::Unbounded => {
                if self.len() == 0 {
                    right = self.len();
                } else {
                    right = self.len() - 1;
                }
            }
        }

        self.attributes.push(((left..right), attribute.into()));
    }

    pub fn attributes(&self) -> Iter<(Range<usize>, Attribute)> {
        self.attributes.iter()
    }

    pub fn bold(self) -> Self {
        self.attribute(.., Font::bold())
    }

    pub fn into_html(&self) -> String {
        if self.attributes.is_empty() {
            return self.text.clone();
        }
        let mut ranges: Vec<usize> = self
            .attributes
            .iter()
            .flat_map(|(range, _)| [range.start, range.end])
            .unique()
            .collect();
        ranges.sort_unstable();
        let mut result = String::new();

        let mut iter = ranges.into_iter().peekable();
        while let Some(start) = iter.next() {
            if let Some(end) = iter.peek() {
                let split_range: Range<usize> = start..*end;
                let mut buf = self.text[split_range.clone()].to_string();

                let attribute_iter = self
                    .attributes
                    .iter()
                    .filter(|(range, _)| contain(range.clone(), split_range.clone()));
                for (_, attribute) in attribute_iter {
                    match attribute {
                        Attribute::Font(font) => {
                            if font.bold {
                                buf = format!("<b>{buf}</b>");
                            }
                            let style = font.html_style();
                            if !style.is_empty() {
                                buf = format!("<label style=\"{}\">{buf}</label>", style);
                            }
                        }
                    }
                }
                result.push_str(&buf);
            }
        }
        result
    }
}

fn contain(orginal: Range<usize>, range: Range<usize>) -> bool {
    orginal.start <= range.start && orginal.end >= range.end
}

impl Add<AttributedString> for AttributedString {
    type Output = AttributedString;
    fn add(mut self, rhs: AttributedString) -> Self::Output {
        self.text.push_str(&rhs.text);
        let len = self.text.len();
        for (range, _) in &mut self.attributes {
            range.end += len;
        }
        for (mut range, attribute) in rhs.attributes {
            range.start += len;
            range.end += len;
            self.set_attribute(range, attribute);
        }
        self
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum Attribute {
    Font(Font),
}

impl_from!(Attribute, Font);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Font {
    name: Cow<'static, str>,
    bold: bool,
    size: Size,
}

impl Default for Font {
    fn default() -> Self {
        Self::new()
    }
}

impl Font {
    pub const fn new() -> Self {
        Self {
            name: Cow::Borrowed(""),
            bold: false,
            size: Size::Default,
        }
    }

    pub fn bold() -> Self {
        let mut font = Self::new();
        font.bold = true;
        font
    }

    pub fn size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }

    fn html_style(&self) -> String {
        let mut buf = String::new();
        if !self.name.is_empty() {
            write!(buf, "font-family:{};", self.name).unwrap();
        }

        match self.size {
            Size::Px(px) => write!(buf, "font-size:{px}px;").unwrap(),
            Size::Percent(percent) => write!(buf, "font-size:{percent}%;").unwrap(),
            _ => {}
        }

        buf
    }
}
