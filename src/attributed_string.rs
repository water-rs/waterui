use std::{
    ops::{Add, Bound, Deref, RangeBounds},
    slice::Iter,
};

use crate::reactive::{IntoRef, Ref};

type Range = (Bound<usize>, Bound<usize>);

#[derive(Clone)]
pub struct AttributedString {
    pub text: String,
    attributes: Vec<(Range, Attribute)>,
}
impl IntoRef<AttributedString> for &str {
    fn into_ref(self) -> Ref<AttributedString> {
        Ref::new(AttributedString::new(self))
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

    pub fn attribute(mut self, range: impl RangeBounds<usize>, attribute: Attribute) -> Self {
        self.set_attribute(range, attribute);
        self
    }

    pub fn set_attribute(&mut self, range: impl RangeBounds<usize>, attribute: Attribute) {
        self.attributes.push((
            (range.start_bound().cloned(), range.end_bound().cloned()),
            attribute,
        ));
    }

    pub fn attributes(&self) -> Iter<(Range, Attribute)> {
        self.attributes.iter()
    }

    //pub fn into_html(&self) -> String {}
}

impl Add<AttributedString> for AttributedString {
    type Output = AttributedString;
    fn add(mut self, rhs: AttributedString) -> Self::Output {
        self.text.push_str(&rhs.text);
        let len = self.text.len();
        for (range, attribute) in rhs.attributes.into_iter() {
            let (mut start, mut end) = range.clone();
            match &mut start {
                Bound::Included(n) => *n += len,
                Bound::Excluded(n) => *n += len,
                Bound::Unbounded => {}
            }

            match &mut end {
                Bound::Included(n) => *n += len,
                Bound::Excluded(n) => *n += len,
                Bound::Unbounded => {}
            }

            self.set_attribute((start, end), attribute);
        }
        self
    }
}

#[derive(Clone)]
pub struct Attribute {
    pub font: Font,
}

#[derive(Clone)]
pub enum Font {
    Default,
}
