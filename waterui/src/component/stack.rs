use alloc::vec::Vec;
use waterui_core::raw_view;
use waterui_core::view::TupleViews;

use crate::AnyView;

#[non_exhaustive]
#[derive(Debug)]
pub struct Stack {
    pub _contents: Vec<AnyView>,
    pub _mode: StackMode,
}

#[derive(Debug)]
pub enum StackMode {
    Auto,
    Vertical,
    Horizonal,
    Layered,
}

impl Stack {
    pub fn new(contents: impl TupleViews) -> Self {
        Self {
            _contents: contents.into_views(),
            _mode: StackMode::Auto,
        }
    }

    pub fn vertical(contents: impl TupleViews) -> Self {
        let mut stack = Self::new(contents);
        stack._mode = StackMode::Vertical;
        stack
    }

    pub fn horizonal(contents: impl TupleViews) -> Self {
        let mut stack = Self::new(contents);
        stack._mode = StackMode::Horizonal;
        stack
    }

    pub fn layered(contents: impl TupleViews) -> Self {
        let mut stack = Self::new(contents);
        stack._mode = StackMode::Layered;
        stack
    }
}

raw_view!(Stack);

pub fn stack(contents: impl TupleViews) -> Stack {
    Stack::new(contents)
}

pub fn vstack(contents: impl TupleViews) -> Stack {
    Stack::vertical(contents)
}

pub fn hstack(contents: impl TupleViews) -> Stack {
    Stack::horizonal(contents)
}

pub fn zstack(contents: impl TupleViews) -> Stack {
    Stack::layered(contents)
}
