use alloc::vec::Vec;
use waterui_core::raw_view;
use waterui_core::view::TupleViews;

use crate::AnyView;

#[derive(Debug)]
pub struct Stack {
    contents: Vec<AnyView>,
    mode: StackMode,
}

#[derive(Debug, Default)]
pub enum StackMode {
    #[default]
    Vertical,
    Horizonal,
    Layered,
}

impl Stack {
    fn new(contents: impl TupleViews, mode: StackMode) -> Self {
        Self {
            contents: contents.into_views(),
            mode,
        }
    }

    pub fn into_inner(self) -> (Vec<AnyView>, StackMode) {
        (self.contents, self.mode)
    }

    pub fn vertical(contents: impl TupleViews) -> Self {
        Self::new(contents, StackMode::Vertical)
    }

    pub fn horizonal(contents: impl TupleViews) -> Self {
        Self::new(contents, StackMode::Horizonal)
    }

    pub fn layered(contents: impl TupleViews) -> Self {
        Self::new(contents, StackMode::Layered)
    }
}

raw_view!(Stack);

pub fn vstack(contents: impl TupleViews) -> Stack {
    Stack::vertical(contents)
}

pub fn hstack(contents: impl TupleViews) -> Stack {
    Stack::horizonal(contents)
}

pub fn zstack(contents: impl TupleViews) -> Stack {
    Stack::layered(contents)
}
