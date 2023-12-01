use crate::layout::{Edge, Frame, Size};
use crate::view;
use crate::view::{BoxView, View};

use crate::view::ViewExt;

#[view(use_core)]
pub struct FrameView {
    pub frame: Frame,
    pub content: BoxView,
}

#[view(use_core)]
impl View for FrameView {
    fn view(&self) {}
}

impl FrameView {
    pub fn new(content: impl View + 'static) -> Self {
        Self {
            frame: Frame::default(),
            content: content.boxed(),
        }
    }
    pub fn width(mut self, size: impl Into<Size>) -> Self {
        self.frame.width = size.into();
        self
    }

    pub fn height(mut self, size: impl Into<Size>) -> Self {
        self.frame.height = size.into();
        self
    }

    pub fn margin(mut self, edge: impl Into<Edge>) -> Self {
        self.frame.margin = edge.into();
        self
    }
}
