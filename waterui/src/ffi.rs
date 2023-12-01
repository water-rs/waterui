use std::ffi::c_char;

use crate::component::Text;
use crate::widget::WidgetInner;
use crate::widget::{Stack, Widget};
use itertools::Itertools;

pub type WaterUISize = crate::layout::Size;
pub type WaterUIEdge = crate::layout::Edge;
pub type WaterUIFrame = crate::layout::Frame;

#[repr(C)]
pub struct WaterUIBuf {
    head: *const c_char,
    len: usize,
}

impl From<String> for WaterUIBuf {
    fn from(value: String) -> Self {
        let value = value.into_bytes();
        let len = value.len();
        let boxed = value.into_boxed_slice();
        Self {
            head: Box::into_raw(boxed) as *const i8,
            len,
        }
    }
}

#[repr(C)]
pub struct WaterUIText {
    buf: WaterUIBuf,
}

#[repr(C)]
pub struct WaterUIWidgets {
    head: *const WaterUIWidget,
    len: usize,
}

impl From<Vec<Widget>> for WaterUIWidgets {
    fn from(value: Vec<Widget>) -> Self {
        let value: Vec<WaterUIWidget> = value.into_iter().map(|v| v.into()).collect_vec();
        let len = value.len();
        let boxed = value.into_boxed_slice();
        Self {
            head: Box::into_raw(boxed) as *const WaterUIWidget,
            len,
        }
    }
}

#[repr(C)]
pub struct WaterUIStack {
    contents: WaterUIWidgets,
}

#[repr(C)]
pub struct WaterUIButton {
    label: WaterUIBuf,
}

impl From<Stack> for WaterUIStack {
    fn from(value: Stack) -> Self {
        Self {
            contents: value.contents.into(),
        }
    }
}

impl From<Text> for WaterUIText {
    fn from(value: Text) -> Self {
        let text: String = value.text.into_plain();
        Self { buf: text.into() }
    }
}

#[repr(C)]
pub enum WaterUIWidgetInner {
    Empty,
    Text(WaterUIText),
    Stack(WaterUIStack),
}

impl_from!(WaterUIWidgetInner, WaterUIText, Text);
impl_from!(WaterUIWidgetInner, WaterUIStack, Stack);

#[repr(C)]
pub struct WaterUIWidget {
    frame: WaterUIFrame,
    inner: WaterUIWidgetInner,
}

impl<T: Into<WaterUIWidgetInner>> From<T> for WaterUIWidget {
    fn from(value: T) -> Self {
        Self {
            frame: WaterUIFrame::default(),
            inner: value.into(),
        }
    }
}

impl From<WidgetInner> for WaterUIWidgetInner {
    fn from(value: WidgetInner) -> Self {
        match value {
            WidgetInner::Text(text) => WaterUIText::from(text).into(),
            WidgetInner::Stack(stack) => WaterUIWidgetInner::Stack(stack.into()),
            WidgetInner::Empty => WaterUIWidgetInner::Empty,
            _ => todo!(),
        }
    }
}

impl From<Widget> for WaterUIWidget {
    fn from(value: Widget) -> Self {
        Self {
            frame: value.frame,
            inner: value.inner.into(),
        }
    }
}

extern "C" {
    pub fn waterui_create_window(title: WaterUIBuf, widget: WaterUIWidget) -> usize;
    pub fn waterui_window_closeable(id: usize, is: bool);
    pub fn waterui_close_window(id: usize);
    pub fn waterui_main() -> WaterUIWidget;
}
