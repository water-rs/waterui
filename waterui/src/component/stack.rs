use serde::{Deserialize, Serialize};

use crate::view::IntoViews;

use crate::{
    view,
    view::{BoxView, ViewExt},
    View,
};

#[view(use_core)]
pub struct Stack {
    pub mode: DisplayMode,
    pub contents: Vec<BoxView>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub enum DisplayMode {
    Vertical,
    Horizontal,
}

impl From<Vec<BoxView>> for Stack {
    fn from(value: Vec<BoxView>) -> Self {
        Self {
            contents: value,
            mode: DisplayMode::Vertical,
        }
    }
}

impl<V: View + 'static> FromIterator<V> for Stack {
    fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
        let content: Vec<BoxView> = iter.into_iter().map(|v| v.boxed()).collect();
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
