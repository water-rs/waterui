pub use waterui_core::view::*;

use crate::{component::TapGesture, Event};

pub trait ViewExt {
    fn on_tap(self, event: impl Event) -> TapGesture;
    fn width(self, size: impl Into<Size>) -> Self;
    fn height(self, size: impl Into<Size>) -> Self;
    fn margin(self, size: impl Into<Edge>) -> Self;
    fn into_boxed(self) -> BoxView;
}

impl<V: View> ViewExt for V {
    fn on_tap(self, event: impl Event) -> TapGesture {
        TapGesture::new(Box::new(self), Box::new(event))
    }

    fn width(mut self, size: impl Into<Size>) -> Self {
        let mut frame = self.frame();
        frame.width = size.into();
        self.set_frame(frame);
        self
    }

    fn height(mut self, size: impl Into<Size>) -> Self {
        let mut frame = self.frame();
        frame.height = size.into();
        self.set_frame(frame);
        self
    }

    fn margin(mut self, size: impl Into<Edge>) -> Self {
        let mut frame = self.frame();
        frame.margin = size.into();
        self.set_frame(frame);
        self
    }

    fn into_boxed(self) -> BoxView {
        Box::new(self)
    }
}
