use crate::{
    component::{Button, HStack, Image, Text, VStack},
    modifier::{background::BackgroundModifier, FrameModifier},
    view::{visit, Visitor},
};

use super::{Widget, WidgetInner};

#[derive(Debug)]
pub struct FFIVisitor;

impl Visitor for FFIVisitor {
    type Value = Widget;
    fn visit_text(self, text: Text) -> Self::Value {
        WidgetInner::Text(text.into()).into()
    }

    fn visit_image(self, image: Image) -> Self::Value {
        WidgetInner::Image(image.into()).into()
    }
    fn visit_frame_modifier(self, modifier: FrameModifier) -> Self::Value {
        let mut widget = visit(modifier.content, self);
        widget.frame = modifier.frame;
        widget
    }

    fn visit_background_modifier(self, modifier: BackgroundModifier) -> Self::Value {
        let mut widget = visit(modifier.content, self);
        widget.background = modifier.background.into();
        widget
    }

    fn visit_empty(self) -> Self::Value {
        WidgetInner::Empty.into()
    }
    fn visit_button(self, button: Button) -> Self::Value {
        WidgetInner::Button(button.into()).into()
    }
    fn visit_vstack(self, stack: VStack) -> Self::Value {
        WidgetInner::Stack(stack.into()).into()
    }

    fn visit_hstack(self, stack: HStack) -> Self::Value {
        WidgetInner::Stack(stack.into()).into()
    }
}
