//! Render tree infrastructure shared across backends.
//!
//! Hydrolysis parses `AnyView` values into [`RenderNode`] implementations and stores them inside a
//! [`RenderTree`]. Backends consume this tree to drive layout and painting.

pub mod layout;
pub mod parser;
pub mod reactive;
pub mod render;

use std::vec::Vec;

pub use layout::{LayoutCtx, LayoutEngine, LayoutResult};
pub use parser::build_tree;
pub use reactive::NodeSignal;
pub use render::{RenderCtx, RenderNode};

/// Identifier for a render node stored inside the [`RenderTree`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(usize);

impl NodeId {
    /// Creates a new [`NodeId`] from the raw index.
    #[must_use]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    /// Returns the raw index backing this identifier.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Reason why a node requires processing before the next frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyReason {
    /// The node's layout is invalid.
    Layout,
    /// Only paint output changed; layout stays valid.
    Paint,
    /// Reactive inputs changed; node should refresh its state.
    Reactive,
}

/// Entry describing a node that needs work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirtyNode {
    /// The affected node identifier.
    pub id: NodeId,
    /// Why the node became dirty.
    pub reason: DirtyReason,
}

#[derive(Debug)]
struct NodeEntry {
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    node: Box<dyn RenderNode>,
}

impl NodeEntry {
    fn new(node: Box<dyn RenderNode>, parent: Option<NodeId>) -> Self {
        Self {
            parent,
            children: Vec::new(),
            node,
        }
    }
}

/// Arena storing the parsed render nodes.
#[derive(Debug, Default)]
pub struct RenderTree {
    nodes: Vec<NodeEntry>,
    root: Option<NodeId>,
    dirty: Vec<DirtyNode>,
}

impl RenderTree {
    /// Creates an empty render tree.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            nodes: Vec::new(),
            root: None,
            dirty: Vec::new(),
        }
    }

    /// Replaces the root node of the tree, clearing any existing nodes.
    pub fn replace_root(&mut self, node: Box<dyn RenderNode>) -> NodeId {
        self.nodes.clear();
        self.dirty.clear();

        let root_id = self.push_entry(NodeEntry::new(node, None));
        self.root = Some(root_id);
        self.mark_dirty(root_id, DirtyReason::Layout);
        root_id
    }

    /// Adds a child under the provided parent.
    ///
    /// # Panics
    ///
    /// Panics if the parent node does not exist.
    pub fn insert_child(&mut self, parent: NodeId, node: Box<dyn RenderNode>) -> NodeId {
        let parent_index = parent.index();
        assert!(
            parent_index < self.nodes.len(),
            "parent must exist before inserting children"
        );

        let id = self.push_entry(NodeEntry::new(node, Some(parent)));
        self.nodes[parent_index].children.push(id);
        id
    }

    /// Returns the root node identifier, if one exists.
    #[must_use]
    pub const fn root(&self) -> Option<NodeId> {
        self.root
    }

    /// Returns a mutable reference to the root node, if present.
    pub fn root_mut(&mut self) -> Option<&mut dyn RenderNode> {
        let root = self.root?;
        self.node_mut(root)
    }

    /// Returns the child identifiers for the provided node.
    #[must_use]
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.nodes
            .get(id.index())
            .map_or(&[], |entry| entry.children.as_slice())
    }

    /// Marks a node as dirty for the provided reason.
    pub fn mark_dirty(&mut self, id: NodeId, reason: DirtyReason) {
        if self
            .dirty
            .iter()
            .any(|entry| entry.id == id && entry.reason == reason)
        {
            return;
        }
        self.dirty.push(DirtyNode { id, reason });
    }

    /// Drains all dirty nodes discovered since the previous frame.
    pub fn drain_dirty(&mut self) -> impl Iterator<Item = DirtyNode> + '_ {
        self.dirty.drain(..)
    }

    /// Visits a node mutably.
    #[must_use]
    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut (dyn RenderNode + '_)> {
        if let Some(entry) = self.nodes.get_mut(id.index()) {
            Some(entry.node.as_mut())
        } else {
            None
        }
    }

    fn push_entry(&mut self, entry: NodeEntry) -> NodeId {
        let id = NodeId::new(self.nodes.len());
        self.nodes.push(entry);
        id
    }

    /// Returns the total number of nodes stored in this tree.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.nodes.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}
