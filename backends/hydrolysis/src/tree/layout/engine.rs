//! Drives layout passes over the render tree.

use waterui_core::Environment;

use crate::{LayoutCtx, NodeId, RenderTree};

/// Layout executor that drives `WaterUI` layout trait objects and writes results back to nodes.
#[derive(Debug)]
pub struct LayoutEngine<'a> {
    tree: &'a mut RenderTree,
    env: &'a Environment,
}

impl<'a> LayoutEngine<'a> {
    /// Creates a new engine bound to the provided render tree.
    pub const fn new(tree: &'a mut RenderTree, env: &'a Environment) -> Self {
        Self { tree, env }
    }

    /// Runs layout for the entire tree.
    ///
    pub fn run(&mut self) {
        if let Some(root) = self.tree.root() {
            self.layout_node(root);
        }
    }

    fn layout_node(&mut self, id: NodeId) {
        if let Some(node) = self.tree.node_mut(id) {
            let ctx = LayoutCtx::new(self.env);
            node.layout(ctx);
        }
        let children = self.tree.children(id).to_vec();
        for child in children {
            self.layout_node(child);
        }
    }
}
