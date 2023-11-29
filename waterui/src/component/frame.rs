use waterui_core::view::{BoxView, Edge, Size, View};
use waterui_derive::widget;

use crate::view::{Frame, ViewExt};

#[widget(use_core)]
pub struct FrameView {
    pub frame: Frame,
    pub content: BoxView,
}

#[widget(use_core)]
impl View for FrameView {
    fn view(&self) {}
}

impl FrameView {
    pub fn new(content: impl View + 'static) -> Self {
        Self {
            frame: Frame::default(),
            content: content.into_boxed(),
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
