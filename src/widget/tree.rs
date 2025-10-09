//! A view for displaying hierarchical data in a tree structure.

use crate::{ViewExt, ext::SignalExt, prelude::*, widget::condition::when};
use alloc::vec::Vec;
use core::hash::Hash;
use std::collections::HashSet;
use waterui_layout::stack::{VStack, hstack, vstack};

/// Represents a node in the tree. It contains the data for the node,
/// a unique ID, and a vector of child nodes.
#[derive(Debug, Clone)]
pub struct TreeNode<T, ID: Clone + Hash + Eq> {
    /// The unique identifier for this node.
    pub id: ID,
    /// The data associated with this node.
    pub data: T,
    /// A vector of child nodes.
    pub children: Vec<TreeNode<T, ID>>,
}

/// A helper view that recursively renders a single node and its children.
/// This is not intended to be used directly by end-users.
#[derive(Debug)]
struct NodeView<T, ID, F>
where
    T: Clone + 'static,
    ID: Clone + Hash + Eq + 'static,
    F: Fn(T) -> AnyView + Clone + 'static,
{
    node: TreeNode<T, ID>,
    expanded: Binding<HashSet<ID>>,
    render_leaf: F,
}

impl<T, ID, F> View for NodeView<T, ID, F>
where
    T: Clone + 'static,
    ID: Clone + Hash + Eq + 'static,
    F: Fn(T) -> AnyView + Clone + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        // Clone the binding at the start to prevent moving from `self`.
        let expanded = self.expanded.clone();

        // Use a clone for the `.map` call, so `expanded` is not moved.
        let is_expanded = expanded.clone().map({
            let id = self.node.id.clone();
            move |expanded_set| expanded_set.contains(&id)
        });

        let has_children = !self.node.children.is_empty();

        // The button to toggle the expanded state of the node.
        let toggle_button = {
            let expanded_for_action = expanded.clone();
            let id = self.node.id.clone();

            // Clone `is_expanded` before it's moved by `.map` to create the icon.
            let icon = is_expanded
                .clone()
                .map(|expanded| if expanded { "▼ " } else { "▶ " });

            button(hstack((
                // Only show the icon if the node has children.
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

        // The view for the children, rendered recursively.
        let children_view = {
            let children = self.node.children.clone();
            let render_leaf = self.render_leaf.clone();

            // Use the original `is_expanded` signal here.
            when(is_expanded, move || {
                VStack::from_iter(children.clone().into_iter().map(|child_node| {
                    NodeView {
                        node: child_node,
                        expanded: expanded.clone(),
                        render_leaf: render_leaf.clone(),
                    }
                    .anyview() // Use .anyview() to break the recursive type definition.
                }))
            })
        };

        vstack((toggle_button, children_view))
    }
}

/// A view that renders hierarchical data in a collapsible tree structure.
///
/// The `TreeView` takes a vector of root `TreeNode`s, a binding to control
/// the expanded state of nodes, and a closure to render the data of each node.
#[derive(Debug)]
pub struct TreeView<T, ID, F>
where
    T: Clone + 'static,
    ID: Clone + Hash + Eq + 'static,
    F: Fn(T) -> AnyView + Clone + 'static,
{
    nodes: Vec<TreeNode<T, ID>>,
    expanded: Binding<HashSet<ID>>,
    render_leaf: F,
}

impl<T, ID, F> TreeView<T, ID, F>
where
    T: Clone + 'static,
    ID: Clone + Hash + Eq + 'static,
    F: Fn(T) -> AnyView + Clone + 'static,
{
    /// Creates a new `TreeView`.
    ///
    /// # Arguments
    ///
    /// * `nodes` - The root nodes of the tree.
    /// * `expanded` - A `Binding` to a `HashSet` containing the IDs of all expanded nodes.
    /// * `render_leaf` - A closure that takes the node's data and returns a `View`.
    pub fn new(
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
    ID: Clone + Hash + Eq + 'static,
    F: Fn(T) -> AnyView + Clone + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        VStack::from_iter(self.nodes.into_iter().map(move |node| {
            NodeView {
                node,
                expanded: self.expanded.clone(),
                render_leaf: self.render_leaf.clone(),
            }
            .anyview()
        }))
    }
}

/// Convenience function to create a new `TreeView`.
pub fn tree_view<T, ID, F>(
    nodes: Vec<TreeNode<T, ID>>,
    expanded: Binding<HashSet<ID>>,
    render_leaf: F,
) -> TreeView<T, ID, F>
where
    T: Clone + 'static,
    ID: Clone + Hash + Eq + 'static,
    F: Fn(T) -> AnyView + Clone + 'static,
{
    TreeView::new(nodes, expanded, render_leaf)
}
