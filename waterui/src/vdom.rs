use serde::{Deserialize, Serialize};
use waterui_core::view::{Alignment, Frame};

use crate::{
    component::{stack::DisplayMode, Button, Image, Text},
    renderer::{renderer, Visitor},
};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Node {
    inner: NodeInner,
    frame: Frame,
}

impl<T: Into<NodeInner>> From<T> for Node {
    fn from(value: T) -> Self {
        Self {
            inner: value.into(),
            frame: Frame::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub enum NodeInner {
    Text(Text),
    Button(Button),
    Image(Image),
    Empty,
    Stack(Stack),
}

impl_from!(NodeInner, Text);
impl_from!(NodeInner, Button);
impl_from!(NodeInner, Image);
impl_from!(NodeInner, Stack);

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Stack {
    alignment: Alignment,
    mode: DisplayMode,
    contents_num: usize,
}

pub struct VDOMVisitor {
    buf: Vec<Node>,
}

impl VDOMVisitor {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }
}

#[derive(Debug)]
pub enum Patch {
    Update(usize, Box<Node>),
    Delete(usize),
}

pub fn diff(original: &[Node], new: &[Node]) -> Vec<Patch> {
    let mut patch = Vec::new();
    diff_inner(original, 0, new, 0, &mut patch);
    patch
}

fn diff_inner(
    original: &[Node],
    original_head: usize,
    new: &[Node],
    new_head: usize,
    patch: &mut Vec<Patch>,
) {
    let left = &original[original_head];
    if let Some(right) = new.get(new_head) {
        if !shallow_eq(left, right) {
            patch.push(Patch::Update(
                original_head,
                Box::new(new[new_head].clone()),
            ))
        }
        if let NodeInner::Stack(_stack) = &left.inner {
            if let NodeInner::Stack(new_stack) = &right.inner {
                for i in 1..=new_stack.contents_num {
                    diff_inner(original, original_head + i, new, new_head + i, patch);
                }
            }
        }
    } else {
        patch.push(Patch::Delete(original_head))
    }
}

fn shallow_eq(left: &Node, right: &Node) -> bool {
    if left.frame != right.frame {
        return false;
    }

    match &left.inner {
        NodeInner::Stack(stack) => {
            if let NodeInner::Stack(new_stack) = &right.inner {
                stack.alignment == new_stack.alignment && stack.mode == new_stack.mode
            } else {
                false
            }
        }
        _ => left == right,
    }
}

impl Visitor for VDOMVisitor {
    type Value = Vec<Node>;

    fn visit_empty(mut self) -> Self::Value {
        self.buf.push(NodeInner::Empty.into());
        self.buf
    }
    fn visit_text(mut self, text: Text) -> Self::Value {
        self.buf.push(text.into());
        self.buf
    }

    fn visit_button(mut self, button: Button) -> Self::Value {
        self.buf.push(button.into());
        self.buf
    }

    fn visit_stack(self, stack: crate::component::Stack) -> Self::Value {
        let mut visitor = self;
        let len = stack.contents.len();

        visitor.buf.push(
            Stack {
                alignment: stack.alignment,
                mode: stack.mode,
                contents_num: len,
            }
            .into(),
        );
        for content in stack.contents {
            visitor = Self {
                buf: renderer(content, visitor),
            };
        }

        visitor.buf
    }

    fn visit_image(mut self, image: Image) -> Self::Value {
        self.buf.push(image.into());
        self.buf
    }

    fn visit_frameview(self, frameview: crate::component::FrameView) -> Self::Value {
        let mut buf = renderer(frameview.content, self);
        buf.last_mut().unwrap().frame = frameview.frame;
        buf
    }
}

#[cfg(test)]
mod test {
    use crate::{
        component::{
            stack::{hstack, vstack},
            text, DatePicker,
        },
        renderer::renderer,
        vdom::diff,
        ViewExt,
    };

    use super::VDOMVisitor;

    #[test]
    fn test() {
        let view = vstack(());
        let view2 = vstack(vstack((vstack(()), text("233"))));

        let output = renderer(view.into_boxed(), VDOMVisitor::new());
        let output2 = renderer(view2.into_boxed(), VDOMVisitor::new());
        println!("{:?}", diff(&output, &output2));
    }
}
