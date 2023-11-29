use itertools::Itertools;
use serde::{Deserialize, Serialize};
use waterui_core::view::{Alignment, Frame};

use crate::{
    component::{stack::DisplayMode, Button, Image, Text},
    renderer::{render, Visitor},
};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Node {
    pub(crate) id: usize,
    pub(crate) inner: NodeInner,
    pub(crate) frame: Frame,
}

impl<T: Into<NodeInner>> From<T> for Node {
    fn from(value: T) -> Self {
        Self {
            id: 1,
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
    contents: Vec<Node>,
}

pub struct VDOMVisitor;

impl VDOMVisitor {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug)]
pub enum Patch {
    Update(usize, Box<Node>),
    Insert(usize, Box<Node>),
    Delete(usize),
}

impl Visitor for VDOMVisitor {
    type Value = Node;

    fn visit_empty(self) -> Self::Value {
        NodeInner::Empty.into()
    }
    fn visit_text(self, text: Text) -> Self::Value {
        text.into()
    }

    fn visit_button(self, button: Button) -> Self::Value {
        button.into()
    }

    fn visit_stack(self, stack: crate::component::Stack) -> Self::Value {
        Stack {
            alignment: stack.alignment,
            mode: stack.mode,
            contents: stack
                .contents
                .into_iter()
                .map(|content| render(content, VDOMVisitor::new()))
                .collect_vec(),
        }
        .into()
    }

    fn visit_image(self, image: Image) -> Self::Value {
        image.into()
    }

    fn visit_frameview(self, frameview: crate::component::FrameView) -> Self::Value {
        let mut node = render(frameview.content, self);
        node.frame = frameview.frame;
        node
    }
}

impl Node {
    pub fn diff(&self, new: &Node) -> Vec<Patch> {
        let mut patch = Vec::new();
        diff_inner(self, new, &mut patch);
        patch
    }
}

fn diff_inner(old: &Node, new: &Node, patch: &mut Vec<Patch>) {
    if !shallow_eq(old, new) {
        patch.push(Patch::Update(old.id, Box::new(new.clone())))
    }

    if let NodeInner::Stack(stack) = &old.inner {
        if let NodeInner::Stack(new_stack) = &new.inner {
            let stack_len = stack.contents.len();
            let new_stack_len = new_stack.contents.len();
            if stack_len >= new_stack_len {
                for i in 0..stack_len {
                    let old = &stack.contents[i];
                    let new = if let Some(new) = new_stack.contents.get(i) {
                        new
                    } else {
                        patch.push(Patch::Delete(i));
                        continue;
                    };
                    diff_inner(old, new, patch);
                }
            } else {
                for i in 0..new_stack_len {
                    let new = &new_stack.contents[i];
                    let old = if let Some(old) = stack.contents.get(i) {
                        old
                    } else {
                        println!("!");
                        patch.push(Patch::Insert(i, Box::new(new.clone())));
                        continue;
                    };
                    diff_inner(old, new, patch);
                }
            }
        }
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

#[cfg(test)]
mod test {
    use crate::{
        component::{stack::vstack, DatePicker},
        renderer::render,
        ViewExt,
    };

    use super::VDOMVisitor;

    #[test]
    fn test() {
        let view = vstack(());
        let view2 = DatePicker::now();

        let output = render(view.into_boxed(), VDOMVisitor::new());
        let output2 = render(view2.into_boxed(), VDOMVisitor::new());
        let diff = output.diff(&output2);
        println!("{diff:?}");
    }
}
