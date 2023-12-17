use crate::{
    layout::{Alignment, Frame, Size},
    Modifier,
};
use waterui_core::modifier::ViewModifier;
pub use waterui_core::view::*;
pub trait ViewExt: View {
    fn modifier<T: ViewModifier>(self, modifier: T) -> Modifier<T>;
    fn width(self, size: impl Into<Size>) -> Modifier<Frame>
    where
        Self: Sized;
    fn height(self, size: impl Into<Size>) -> Modifier<Frame>
    where
        Self: Sized;

    fn leading(self) -> Modifier<Frame>;

    fn boxed(self) -> BoxView;
}

impl<V: View + 'static> ViewExt for V {
    fn modifier<T: ViewModifier>(self, modifier: T) -> Modifier<T> {
        Modifier::new(self.boxed(), modifier)
    }

    fn width(self, size: impl Into<Size>) -> Modifier<Frame> {
        Modifier::new(self.boxed(), Frame::default().width(size))
    }

    fn height(self, size: impl Into<Size>) -> Modifier<Frame> {
        Modifier::new(self.boxed(), Frame::default().height(size))
    }

    fn leading(self) -> Modifier<Frame> {
        Modifier::new(self.boxed(), Frame::default().alignment(Alignment::Leading))
    }

    fn boxed(self) -> BoxView {
        Box::new(self)
    }
}
