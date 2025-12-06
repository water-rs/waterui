//! Utilities for converting `AnyView` trees into Hydrolysis render nodes.

use waterui::component::progress::ProgressConfig;
use waterui::prelude::Divider;
use waterui_controls::{
    slider::SliderConfig, stepper::StepperConfig, text_field::TextFieldConfig, toggle::ToggleConfig,
};
use waterui_core::{AnyView, Environment, Native, View};
use waterui_layout::{
    ScrollView, container::FixedContainer as LayoutFixedContainer, spacer::Spacer,
};
use waterui_text::TextConfig;

use crate::{
    components::text::TextNode,
    tree::{NodeId, RenderTree},
};

/// Builds a [`RenderTree`] from a root [`AnyView`].
#[must_use]
pub fn build_tree(env: &Environment, view: AnyView) -> RenderTree {
    let mut builder = TreeBuilder::new(env);
    builder.build_root(view);
    builder.finish()
}

struct TreeBuilder<'env> {
    env: &'env Environment,
    tree: RenderTree,
}

impl<'env> TreeBuilder<'env> {
    const fn new(env: &'env Environment) -> Self {
        Self {
            env,
            tree: RenderTree::new(),
        }
    }

    fn finish(self) -> RenderTree {
        self.tree
    }

    fn build_root(&mut self, view: AnyView) -> Option<NodeId> {
        self.build_any(view, None)
    }

    fn build_any(&mut self, view: AnyView, parent: Option<NodeId>) -> Option<NodeId> {
        // Handle primitive cases.
        let view = match view.downcast::<()>() {
            Ok(_) => return None,
            Err(view) => view,
        };

        // Text nodes (Native<TextConfig>).
        let view = match view.downcast::<Native<TextConfig>>() {
            Ok(native) => {
                let node = TextNode::new(native.0, self.env);
                return Some(self.insert_node(parent, Box::new(node)));
            }
            Err(view) => view,
        };

        // Divider.
        let view = match view.downcast::<Divider>() {
            Ok(divider) => {
                let node = crate::components::divider::DividerNode::new(*divider);
                return Some(self.insert_node(parent, Box::new(node)));
            }
            Err(view) => view,
        };

        // Fixed containers.
        let view = match view.downcast::<LayoutFixedContainer>() {
            Ok(container) => {
                let id = self.build_fixed_container(*container, parent);
                return Some(id);
            }
            Err(view) => view,
        };

        // Scroll views (pass-through until ScrollNode exists).
        let view = match view.downcast::<ScrollView>() {
            Ok(scroll) => {
                let (_axis, content) = scroll.into_inner();
                return self.build_any(content, parent);
            }
            Err(view) => view,
        };

        let view = match view.downcast::<Spacer>() {
            Ok(spacer) => {
                let id = self.insert_node(
                    parent,
                    Box::new(crate::components::layout::SpacerNode::new(*spacer)),
                );
                return Some(id);
            }
            Err(view) => view,
        };

        let view = match view.downcast::<Native<SliderConfig>>() {
            Ok(native) => {
                let node = crate::components::controls::SliderNode::new(native.0);
                return Some(self.insert_node(parent, Box::new(node)));
            }
            Err(view) => view,
        };

        let view = match view.downcast::<Native<StepperConfig>>() {
            Ok(native) => {
                let node = crate::components::controls::StepperNode::new(native.0);
                return Some(self.insert_node(parent, Box::new(node)));
            }
            Err(view) => view,
        };

        let view = match view.downcast::<Native<ToggleConfig>>() {
            Ok(native) => {
                let node = crate::components::controls::ToggleNode::new(native.0);
                return Some(self.insert_node(parent, Box::new(node)));
            }
            Err(view) => view,
        };

        let view = match view.downcast::<Native<TextFieldConfig>>() {
            Ok(native) => {
                let node = crate::components::controls::TextFieldNode::new(native.0);
                return Some(self.insert_node(parent, Box::new(node)));
            }
            Err(view) => view,
        };

        let view = match view.downcast::<Native<ProgressConfig>>() {
            Ok(native) => {
                let node = crate::components::progress::ProgressNode::new(native.0);
                return Some(self.insert_node(parent, Box::new(node)));
            }
            Err(view) => view,
        };

        // TODO(parser): handle layout containers, images, controls, and metadata.

        // Default fallback: expand body and keep parsing.
        let next = view.body(self.env);
        self.build_any(AnyView::new(next), parent)
    }

    fn insert_node(
        &mut self,
        parent: Option<NodeId>,
        node: Box<dyn crate::tree::RenderNode>,
    ) -> NodeId {
        if let Some(parent) = parent {
            self.tree.insert_child(parent, node)
        } else {
            self.tree.replace_root(node)
        }
    }
    fn build_fixed_container(
        &mut self,
        container: LayoutFixedContainer,
        parent: Option<NodeId>,
    ) -> NodeId {
        let (layout, children) = container.into_inner();
        let id = self.insert_node(
            parent,
            Box::new(crate::components::layout::FixedContainerNode::new(layout)),
        );
        for child in children {
            self.build_any(child, Some(id));
        }
        id
    }
}
