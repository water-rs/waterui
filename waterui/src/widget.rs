use crate::layout::Frame;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use waterui_core::view::BoxView;

use crate::{
    component::{stack::DisplayMode, Button, Image, Text},
    view::{visit, Visitor},
    View,
};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Widget {
    pub(crate) inner: WidgetInner,
    pub(crate) frame: Frame,
}

impl Widget {
    pub fn from_view(view: impl View + 'static) -> Self {
        let view = Box::new(view);
        to_widget(view)
    }
}

impl<T: Into<WidgetInner>> From<T> for Widget {
    fn from(value: T) -> Self {
        Self {
            inner: value.into(),
            frame: Frame::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub enum WidgetInner {
    Text(Text),
    Button(Button),
    Image(Image),
    Empty,
    Stack(Stack),
}

impl_from!(WidgetInner, Text);
impl_from!(WidgetInner, Button);
impl_from!(WidgetInner, Image);
impl_from!(WidgetInner, Stack);

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Stack {
    mode: DisplayMode,
    pub(crate) contents: Vec<Widget>,
}

struct WidgetVisitor;

pub(crate) fn to_widget(view: BoxView) -> Widget {
    visit(view, WidgetVisitor)
}

impl Visitor for WidgetVisitor {
    type Value = Widget;

    fn visit_empty(self) -> Self::Value {
        WidgetInner::Empty.into()
    }
    fn visit_text(self, text: Text) -> Self::Value {
        text.into()
    }

    fn visit_button(self, button: Button) -> Self::Value {
        button.into()
    }

    fn visit_stack(self, stack: crate::component::Stack) -> Self::Value {
        Stack {
            mode: stack.mode,
            contents: stack
                .contents
                .into_iter()
                .map(|content| visit(content, WidgetVisitor))
                .collect_vec(),
        }
        .into()
    }

    fn visit_image(self, image: Image) -> Self::Value {
        image.into()
    }

    fn visit_frameview(self, frameview: crate::component::FrameView) -> Self::Value {
        let mut node = visit(frameview.content, self);
        node.frame = frameview.frame;
        node
    }
}
