use waterui_core::view::Frame;

use crate::{component, vdom, view::Alignment};

#[repr(C)]
pub struct Text {
    text: Buf,
    alignment: Alignment,
    selectable: bool,
}

impl From<component::Text> for Text {
    fn from(value: component::Text) -> Self {
        Self {
            text: value.text.into_plain().into(),
            alignment: value.alignment,
            selectable: value.selectable,
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
    node: NodeInner,
}

impl From<vdom::Node> for Node {
    fn from(value: vdom::Node) -> Self {
        let node = match value.inner {
            vdom::NodeInner::Text(text) => NodeInner::Text(text.into()),
            _ => todo!(),
        };
        Node {
            frame: value.frame,
            node,
        }
    }
}

#[repr(C)]
pub struct Buf {
    head: *const u8,
    len: usize,
}

impl From<String> for Buf {
    fn from(value: String) -> Self {
        let head = value.as_ptr();
        Self {
            head,
            len: value.len(),
        }
    }
}
