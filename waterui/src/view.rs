use crate::layout::Size;
pub use waterui_core::view::*;

use crate::component::{Button, HStack, Image, Text, VStack};
use crate::modifier::background::BackgroundModifier;
use crate::modifier::FrameModifier;
use crate::{component::TapGesture, Event};

pub(crate) trait Visitor {
    type Value;
    fn visit_empty(self) -> Self::Value;
    fn visit_text(self, text: Text) -> Self::Value;
    fn visit_image(self, image: Image) -> Self::Value;
    fn visit_button(self, button: Button) -> Self::Value;
    fn visit_vstack(self, stack: VStack) -> Self::Value;
    fn visit_hstack(self, stack: HStack) -> Self::Value;

    fn visit_frame_modifier(self, modifier: FrameModifier) -> Self::Value;
    fn visit_background_modifier(self, modifier: BackgroundModifier) -> Self::Value;
    fn other(self, view: BoxView) -> Self::Value
    where
        Self: Sized,
    {
        visit(view.view(), self)
    }
}

macro_rules! impl_visit {
    ($($method:tt),*) => {
        pub(crate) fn visit<V: Visitor>(mut view: BoxView, visitor: V) -> V::Value {
            match view.downcast::<()>() {
                Ok(_) => return visitor.visit_empty(),
                Err(boxed) => view = boxed,
            }

            $(
                match view.downcast() {
                    Ok(text) => return visitor.$method(*text),
                    Err(boxed) => view = boxed,
                }
            )*

            match view.downcast::<BoxView>() {
                Ok(boxed) => return visit(*boxed,visitor),
                Err(boxed) => view = boxed,
            }

            visitor.other(view)
        }
    };
}

impl_visit!(
    visit_text,
    visit_image,
    visit_button,
    visit_vstack,
    visit_hstack,
    visit_frame_modifier,
    visit_background_modifier
);

pub trait ViewExt: View {
    fn on_tap(self, event: impl Event) -> TapGesture;
    fn width(self, size: impl Into<Size>) -> FrameModifier
    where
        Self: Sized;
    fn height(self, size: impl Into<Size>) -> FrameModifier
    where
        Self: Sized;

    fn leading(self) -> FrameModifier;

    fn boxed(self) -> BoxView;
}

impl<V: View + 'static> ViewExt for V {
    fn on_tap(self, event: impl Event) -> TapGesture {
        TapGesture::new(Box::new(self), Box::new(event))
    }

    fn width(self, size: impl Into<Size>) -> FrameModifier {
        FrameModifier::new(self.boxed()).width(size)
    }

    fn height(self, size: impl Into<Size>) -> FrameModifier {
        FrameModifier::new(self.boxed()).height(size)
    }

    fn leading(self) -> FrameModifier {
        FrameModifier::new(self.boxed()).leading()
    }

    fn boxed(self) -> BoxView {
        Box::new(self)
    }
}
