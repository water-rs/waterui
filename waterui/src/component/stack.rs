use crate::view::IntoViews;

use crate::{
    view::{Alignment, BoxView, ViewExt},
    widget, View,
};

#[widget]
pub struct Stack {
    pub alignment: Alignment,
    pub mode: DisplayMode,
    pub content: Vec<BoxView>,
}

#[repr(C)]
#[derive(Debug, Clone)]
pub enum DisplayMode {
    Vertical,
    Horizontal,
}

impl From<Vec<BoxView>> for Stack {
    fn from(value: Vec<BoxView>) -> Self {
        Self {
            content: value,
            alignment: Alignment::Default,
            mode: DisplayMode::Vertical,
        }
    }
}

impl<V: View + 'static> FromIterator<V> for Stack {
    fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
        let content: Vec<BoxView> = iter.into_iter().map(|v| v.into_boxed()).collect();
        Self::from(content)
    }
}

impl Stack {
    pub fn new(views: impl IntoViews) -> Self {
        Self::from(views.into_views())
    }

    pub fn vertical(mut self) -> Self {
        self.mode = DisplayMode::Vertical;
        self
    }

    pub fn horizontal(mut self) -> Self {
        self.mode = DisplayMode::Horizontal;
        self
    }

    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }

    pub fn mode(mut self, mode: DisplayMode) -> Self {
        self.mode = mode;
        self
    }
}

pub fn vstack(views: impl IntoViews) -> Stack {
    Stack::new(views).vertical()
}

pub fn hstack(views: impl IntoViews) -> Stack {
    Stack::new(views).horizontal()
}

native_implement!(Stack);
