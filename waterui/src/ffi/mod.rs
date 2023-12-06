pub mod buf;
mod visitor;

use crate::{
    component,
    layout::Frame,
    utils::{self, Color},
    view::{visit, BoxView},
};

use buf::Buf;
use itertools::Itertools;

#[derive(Debug)]
#[repr(C)]
pub struct Text {
    buf: Buf,
}

#[derive(Debug)]
#[repr(C)]
pub struct Widgets {
    head: *const Widget,
    len: usize,
}

impl From<Vec<BoxView>> for Widgets {
    fn from(value: Vec<BoxView>) -> Self {
        let len = value.len();
        let widgets = value.into_iter().map(Widget::from_boxed_view).collect_vec();

        let boxed = widgets.into_boxed_slice();
        Self {
            head: Box::into_raw(boxed) as *const Widget,
            len,
        }
    }
}

#[derive(Debug)]
#[repr(C)]
pub struct Stack {
    mode: StackMode,
    contents: Widgets,
}

#[derive(Debug)]
#[repr(u8)]
pub enum StackMode {
    Vertical,
    Horizonal,
}

#[derive(Debug)]
#[repr(C)]
pub struct Button {
    label: Buf,
}

#[derive(Debug)]
#[repr(C)]
pub struct Image {
    data: Buf,
}

impl From<component::Image> for Image {
    fn from(value: component::Image) -> Self {
        Self {
            data: value.data.into(),
        }
    }
}

impl From<component::Text> for Text {
    fn from(value: component::Text) -> Self {
        let text: String = value.text.into_plain();
        Self { buf: text.into() }
    }
}

impl From<component::Button> for Button {
    fn from(value: component::Button) -> Self {
        let label: String = value.label.into_plain();
        Self {
            label: label.into(),
        }
    }
}
#[derive(Debug)]
#[repr(C)]
pub enum WidgetInner {
    Empty,
    Text(Text),
    Button(Button),
    Image(Image),
    Stack(Stack),
}

impl From<component::VStack> for Stack {
    fn from(value: component::VStack) -> Self {
        Self {
            mode: StackMode::Vertical,
            contents: value.contents.into(),
        }
    }
}

impl From<component::HStack> for Stack {
    fn from(value: component::HStack) -> Self {
        Self {
            mode: StackMode::Horizonal,
            contents: value.contents.into(),
        }
    }
}
#[derive(Debug)]
#[repr(C)]
pub enum Background {
    Default,
    Color(Color),
}

impl From<utils::Background> for Background {
    fn from(value: utils::Background) -> Self {
        match value {
            utils::Background::Default => Background::Default,
            utils::Background::Color(color) => Background::Color(color),
        }
    }
}

impl_from!(WidgetInner, Text, Text);
impl_from!(WidgetInner, Stack, Stack);

#[derive(Debug)]
#[repr(C)]
pub struct Widget {
    frame: Frame,
    background: Background,
    inner: WidgetInner,
}

impl Widget {
    pub fn from_boxed_view(view: BoxView) -> Self {
        visit(view, visitor::FFIVisitor)
    }
}

impl<T: Into<WidgetInner>> From<T> for Widget {
    fn from(value: T) -> Self {
        Self {
            frame: Frame::default(),
            background: Background::Default,
            inner: value.into(),
        }
    }
}

extern "C" {
    pub fn waterui_create_window(title: Buf, widget: Widget) -> usize;
    pub fn waterui_window_closeable(id: usize, is: bool);
    pub fn waterui_close_window(id: usize);
    pub fn waterui_main() -> Widget;
}
