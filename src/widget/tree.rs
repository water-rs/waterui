//! A view for displaying hierarchical data in a tree structure.

use crate::{ViewExt, ext::SignalExt, prelude::*, widget::condition::when};
use alloc::vec::Vec;
use core::hash::Hash;
use std::collections::HashSet;
use waterui_core::id::Identifiable;
use waterui_layout::stack::{VStack, hstack, vstack};

/// Represents a node in the tree. It contains the data for the node,
/// a unique ID, and a vector of child nodes.
#[derive(Debug, Clone)]
pub struct TreeNode<T, ID> {
    /// The unique identifier for this node.
    pub id: ID,
    /// The data associated with this node.
    pub data: T,
    /// A vector of child nodes.
    pub children: Vec<TreeNode<T, ID>>,
}

impl<T, ID: Clone + Hash + Ord> Identifiable for TreeNode<T, ID> {
    type Id = ID;
    fn id(&self) -> Self::Id {
        self.id.clone()
    }
}

/// A helper view that recursively renders a single node and its children.
#[derive(Debug)]
struct NodeView<T, ID, F>
where
    T: Clone + 'static,
    ID: Clone + Hash + Eq + Ord + 'static,
    F: Fn(T) -> AnyView + Clone + 'static,
{
    node: TreeNode<T, ID>,
    expanded: Binding<HashSet<ID>>,
    render_leaf: F,
}

impl<T, ID, F> View for NodeView<T, ID, F>
where
    T: Clone + 'static,
    ID: Clone + Hash + Eq + Ord + 'static,
    F: Fn(T) -> AnyView + Clone + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let expanded = self.expanded.clone();

        let is_expanded = expanded.clone().map({
            let id = self.node.id();
            move |expanded_set| expanded_set.contains(&id)
        });

        let has_children = !self.node.children.is_empty();

        let toggle_button = {
            let expanded_for_action = expanded.clone();
            let id = self.node.id();

            let icon = is_expanded
                .clone()
                .map(|expanded| if expanded { "▼ " } else { "▶ " });

            button(hstack((
                when(has_children, move || text(icon.clone())),
                (self.render_leaf)(self.node.data.clone()),
            )))
            .action(move || {
                if has_children {
                    let mut set = expanded_for_action.get();
                    if set.contains(&id) {
                        set.remove(&id);
                    } else {
                        set.insert(id.clone());
                    }
                    expanded_for_action.set(set);
                }
            })
        };

        let children_view = {
            let children = self.node.children.clone();
            let render_leaf = self.render_leaf;

            when(is_expanded, move || {
                children
                    .clone()
                    .into_iter()
                    .map(|child_node| {
                        Self {
                            node: child_node,
                            expanded: expanded.clone(),
                            render_leaf: render_leaf.clone(),
                        }
                        .anyview()
                    })
                    .collect::<VStack<_>>()
            })
        };

        vstack((toggle_button, children_view))
    }
}

/// A view that renders hierarchical data in a collapsible tree structure.
#[derive(Debug)]
pub struct TreeView<T, ID, F>
where
    T: Clone + 'static,
    ID: Clone + Hash + Eq + Ord + 'static,
    F: Fn(T) -> AnyView + Clone + 'static,
{
    nodes: Vec<TreeNode<T, ID>>,
    expanded: Binding<HashSet<ID>>,
    render_leaf: F,
}

impl<T, ID, F> TreeView<T, ID, F>
where
    T: Clone + 'static,
    ID: Clone + Hash + Eq + Ord + 'static,
    F: Fn(T) -> AnyView + Clone + 'static,
{
    /// Creates a new `TreeView`.
    pub const fn new(
        nodes: Vec<TreeNode<T, ID>>,
        expanded: Binding<HashSet<ID>>,
        render_leaf: F,
    ) -> Self {
        Self {
            nodes,
            expanded,
            render_leaf,
        }
    }
}

impl<T, ID, F> View for TreeView<T, ID, F>
where
    T: Clone + 'static,
    ID: Clone + Hash + Eq + Ord + 'static,
    F: Fn(T) -> AnyView + Clone + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        self.nodes
            .into_iter()
            .map(move |node| NodeView {
                node,
                expanded: self.expanded.clone(),
                render_leaf: self.render_leaf.clone(),
            })
            .collect::<VStack<_>>()
    }
}

#[allow(clippy::implicit_hasher)]
/// Convenience function to create a new `TreeView`.
pub const fn tree_view<T, ID, F>(
    nodes: Vec<TreeNode<T, ID>>,
    expanded: Binding<HashSet<ID>>,
    render_leaf: F,
) -> TreeView<T, ID, F>
where
    T: Clone + 'static,
    ID: Clone + Hash + Eq + Ord + 'static,
    F: Fn(T) -> AnyView + Clone + 'static,
{
    TreeView::new(nodes, expanded, render_leaf)
}
