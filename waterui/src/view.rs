use waterui_core::layout::Size;
pub use waterui_core::view::*;

use crate::component::{Button, FrameView, Image, Stack, Text};
use crate::{component::TapGesture, Event};

pub(crate) trait Visitor {
    type Value;
    fn visit_empty(self) -> Self::Value;
    fn visit_text(self, text: Text) -> Self::Value;
    fn visit_image(self, image: Image) -> Self::Value;
    fn visit_button(self, button: Button) -> Self::Value;
    fn visit_stack(self, stack: Stack) -> Self::Value;
    fn visit_frameview(self, frame: FrameView) -> Self::Value;
    fn other(self, view: BoxView) -> Self::Value
    where
        Self: Sized,
    {
        visit(view.view(), self)
    }
}

pub(crate) fn visit<V: Visitor>(mut view: BoxView, visitor: V) -> V::Value {
    match view.downcast::<()>() {
        Ok(_) => return visitor.visit_empty(),
        Err(boxed) => view = boxed,
    }
    match view.downcast() {
        Ok(text) => return visitor.visit_text(*text),
        Err(boxed) => view = boxed,
    }

    match view.downcast() {
        Ok(image) => return visitor.visit_image(*image),
        Err(boxed) => view = boxed,
    }

    match view.downcast() {
        Ok(button) => return visitor.visit_button(*button),
        Err(boxed) => view = boxed,
    }

    match view.downcast() {
        Ok(stack) => return visitor.visit_stack(*stack),
        Err(boxed) => view = boxed,
    }

    match view.downcast() {
        Ok(frameview) => return visitor.visit_frameview(*frameview),
        Err(boxed) => view = boxed,
    }

    match view.downcast::<BoxView>() {
        Ok(view) => return visit(*view, visitor),
        Err(boxed) => view = boxed,
    }

    visitor.other(view)
}

pub trait ViewExt {
    fn on_tap(self, event: impl Event) -> TapGesture;
    fn width(self, size: impl Into<Size>) -> FrameView;
    fn height(self, size: impl Into<Size>) -> FrameView;

    fn boxed(self) -> BoxView;
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

    fn boxed(self) -> BoxView {
        Box::new(self)
    }
}
