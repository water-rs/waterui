use crate::layout::{Alignment, Edge, Frame, Size};
use crate::view::BoxView;

pub struct FrameModifier {
    pub(crate) frame: Frame,
    pub(crate) content: BoxView,
}

native_implement!(FrameModifier);

impl FrameModifier {
    pub fn new(content: BoxView) -> Self {
        Self {
            frame: Frame::default(),
            content,
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

    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.frame.alignment = alignment;
        self
    }

    pub fn leading(self) -> Self {
        self.alignment(Alignment::Leading)
    }
}
