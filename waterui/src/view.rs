pub use waterui_core::view::*;

use crate::component::FrameView;
use crate::{component::TapGesture, Event};
pub trait ViewExt {
    fn on_tap(self, event: impl Event) -> TapGesture;
    fn width(self, size: impl Into<Size>) -> FrameView;
    fn height(self, size: impl Into<Size>) -> FrameView;

    fn into_boxed(self) -> BoxView;
}

impl<V: View + 'static> ViewExt for V {
    fn on_tap(self, event: impl Event) -> TapGesture {
        TapGesture::new(Box::new(self), Box::new(event))
    }

    fn width(self, size: impl Into<Size>) -> FrameView {
        FrameView::new(self).width(size)
    }

    fn height(self, size: impl Into<Size>) -> FrameView {
        FrameView::new(self).height(size)
    }

    fn into_boxed(self) -> BoxView {
        Box::new(self)
    }
}
