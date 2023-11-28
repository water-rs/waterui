use waterui_core::view::BoxView;

use crate::component::{Button, FrameView, Image, Stack, Text};

pub trait Visitor {
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
        renderer(view.view(), self)
    }
}

pub fn renderer<V: Visitor>(mut view: BoxView, visitor: V) -> V::Value {
    match view.downcast::<()>() {
        Ok(text) => return visitor.visit_empty(),
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
        Ok(view) => return renderer(*view, visitor),
        Err(boxed) => view = boxed,
    }

    visitor.other(view)
}
