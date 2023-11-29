use waterui_core::view::{BoxView, Frame};

use crate::{
    component,
    renderer::render,
    vdom::{self, VDOMVisitor},
    window::WindowManager,
};

#[repr(C)]
pub struct Text {
    buf: Buf,
}

#[repr(C)]
pub struct Buf {
    head: *const u8,
    len: usize,
}

impl From<String> for Buf {
    fn from(value: String) -> Self {
        Self {
            head: value.as_ptr(),
            len: value.len(),
        }
    }
}

impl From<component::Text> for Text {
    fn from(value: component::Text) -> Self {
        Self {
            buf: value.text.into_plain().into(),
        }
    }
}

#[repr(C)]
pub enum NodeInner {
    Text(Text),
}

#[repr(C)]
pub struct Node {
    frame: Frame,
    inner: NodeInner,
}

impl From<vdom::NodeInner> for NodeInner {
    fn from(value: vdom::NodeInner) -> Self {
        match value {
            vdom::NodeInner::Text(text) => NodeInner::Text(text.into()),
            _ => todo!(),
        }
    }
}

impl From<vdom::Node> for Node {
    fn from(value: vdom::Node) -> Self {
        Self {
            frame: value.frame,
            inner: value.inner.into(),
        }
    }
}

extern "C" {
    pub fn create_window(view: Node) -> usize;
    pub fn close_window(id: usize);
}

pub struct FFIWindowManager;

impl WindowManager for FFIWindowManager {
    fn create(view: BoxView) -> usize {
        let node = render(view, VDOMVisitor);
        unsafe { create_window(Node::from(node)) }
    }
    fn close(id: usize) {
        unsafe {
            close_window(id);
        }
    }
}
