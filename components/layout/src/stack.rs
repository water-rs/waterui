use alloc::vec::Vec;
use waterui_core::AnyView;
use waterui_core::View;
use waterui_core::view::TupleViews;

use crate::Alignment;
use crate::engine::{Container, Layout};

/// A vertical stack layout that arranges views from top to bottom.
///
/// Views are positioned vertically with optional spacing between them,
/// and can be aligned horizontally within the stack.
#[derive(Debug)]
pub struct VStack {
    /// The horizontal alignment of children within the stack.
    pub alignment: Alignment,
    /// The spacing between adjacent children in the stack.
    pub spacing: f64,
    /// The child views to arrange in the stack.
    pub children: Vec<AnyView>,
}

impl Layout for VStack {
    fn layout(
        self,
        constraint: crate::engine::Constraint,
        measured_children: &[crate::engine::MeasuredChild],
    ) -> crate::engine::LayoutResult {
        let mut total_height: f64 = 0.0;
        let mut max_width: f64 = 0.0;
        let mut child_positions = Vec::new();

        let available_width = constraint.max.width;
        let mut current_y = 0.0;

        for child in measured_children {
            // Calculate actual child size within constraints
            let child_width = child
                .ideal_size
                .width
                .min(available_width)
                .min(child.max_size.width)
                .max(child.min_size.width);

            let child_height = child
                .ideal_size
                .height
                .min(child.max_size.height)
                .max(child.min_size.height);

            // Calculate x position based on alignment
            let child_x = match self.alignment {
                Alignment::Leading | Alignment::Default => 0.0,
                Alignment::Center => (available_width - child_width) / 2.0,
                Alignment::Trailing => available_width - child_width,
            };

            child_positions.push(crate::Size {
                width: child_x,    // Store x position in width
                height: current_y, // Store y position in height
            });

            current_y += child_height + self.spacing;
            max_width = max_width.max(child_width);
        }

        // Remove the last spacing
        if !child_positions.is_empty() {
            total_height = current_y - self.spacing;
        }

        crate::engine::LayoutResult {
            size: crate::Size {
                width: max_width
                    .min(constraint.max.width)
                    .max(constraint.min.width),
                height: total_height
                    .min(constraint.max.height)
                    .max(constraint.min.height),
            },
            child_positions,
            child: AnyView::new(self),
        }
    }
}

impl View for VStack {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Container::new(self)
    }
}

/// A horizontal stack layout that arranges views from left to right.
///
/// Views are positioned horizontally with optional spacing between them,
/// and can be aligned vertically within the stack.
#[derive(Debug)]
pub struct HStack {
    /// The vertical alignment of children within the stack.
    pub alignment: Alignment,
    /// The spacing between adjacent children in the stack.
    pub spacing: f64,
    /// The child views to arrange in the stack.
    pub children: Vec<AnyView>,
}

impl Layout for HStack {
    fn layout(
        self,
        constraint: crate::engine::Constraint,
        measured_children: &[crate::engine::MeasuredChild],
    ) -> crate::engine::LayoutResult {
        let mut total_width: f64 = 0.0;
        let mut max_height: f64 = 0.0;
        let mut child_positions = Vec::new();

        let available_height = constraint.max.height;
        let mut current_x = 0.0;

        for child in measured_children {
            // Calculate actual child size within constraints
            let child_width = child
                .ideal_size
                .width
                .min(child.max_size.width)
                .max(child.min_size.width);

            let child_height = child
                .ideal_size
                .height
                .min(available_height)
                .min(child.max_size.height)
                .max(child.min_size.height);

            // Calculate y position based on alignment
            let child_y = match self.alignment {
                Alignment::Leading | Alignment::Default => 0.0,
                Alignment::Center => (available_height - child_height) / 2.0,
                Alignment::Trailing => available_height - child_height,
            };

            child_positions.push(crate::Size {
                width: current_x, // Store x position in width
                height: child_y,  // Store y position in height
            });

            current_x += child_width + self.spacing;
            max_height = max_height.max(child_height);
        }

        // Remove the last spacing
        if !child_positions.is_empty() {
            total_width = current_x - self.spacing;
        }

        crate::engine::LayoutResult {
            size: crate::Size {
                width: total_width
                    .min(constraint.max.width)
                    .max(constraint.min.width),
                height: max_height
                    .min(constraint.max.height)
                    .max(constraint.min.height),
            },
            child_positions,
            child: AnyView::new(self),
        }
    }
}

impl View for HStack {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Container::new(self)
    }
}

/// A depth stack layout that layers views on top of each other.
///
/// All views are positioned at the same location, creating layered content.
/// Views are aligned according to the specified alignment option.
#[derive(Debug)]
pub struct ZStack {
    /// The alignment of children within the stack bounds.
    pub alignment: Alignment,
    /// The child views to layer in the stack.
    pub children: Vec<AnyView>,
}

impl Layout for ZStack {
    fn layout(
        self,
        constraint: crate::engine::Constraint,
        measured_children: &[crate::engine::MeasuredChild],
    ) -> crate::engine::LayoutResult {
        let mut max_width: f64 = 0.0;
        let mut max_height: f64 = 0.0;
        let mut child_positions = Vec::new();

        // Find the maximum dimensions among all children
        for child in measured_children {
            max_width = max_width.max(child.ideal_size.width);
            max_height = max_height.max(child.ideal_size.height);
        }

        let container_width = max_width
            .min(constraint.max.width)
            .max(constraint.min.width);
        let container_height = max_height
            .min(constraint.max.height)
            .max(constraint.min.height);

        // Position all children based on alignment (all layered on top of each other)
        for child in measured_children {
            let child_width = child
                .ideal_size
                .width
                .min(container_width)
                .min(child.max_size.width)
                .max(child.min_size.width);

            let child_height = child
                .ideal_size
                .height
                .min(container_height)
                .min(child.max_size.height)
                .max(child.min_size.height);

            // Calculate position based on alignment
            let child_x = match self.alignment {
                Alignment::Leading => 0.0,
                Alignment::Center | Alignment::Default => (container_width - child_width) / 2.0,
                Alignment::Trailing => container_width - child_width,
            };

            let child_y = match self.alignment {
                Alignment::Leading => 0.0,
                Alignment::Center | Alignment::Default => (container_height - child_height) / 2.0,
                Alignment::Trailing => container_height - child_height,
            };

            child_positions.push(crate::Size {
                width: child_x,  // Store x position in width
                height: child_y, // Store y position in height
            });
        }

        crate::engine::LayoutResult {
            size: crate::Size {
                width: container_width,
                height: container_height,
            },
            child_positions,
            child: AnyView::new(self),
        }
    }
}

impl View for ZStack {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Container::new(self)
    }
}

/// Creates a vertical stack with the given child views.
///
/// The stack will have center alignment and 8.0 spacing by default.
/// Use methods on the returned `VStack` to customize alignment and spacing.
pub fn vstack(contents: impl TupleViews) -> VStack {
    VStack {
        alignment: Alignment::Center,
        spacing: 8.0,
        children: contents.into_views(),
    }
}

/// Creates a horizontal stack with the given child views.
///
/// The stack will have center alignment and 8.0 spacing by default.
/// Use methods on the returned `HStack` to customize alignment and spacing.
pub fn hstack(contents: impl TupleViews) -> HStack {
    HStack {
        alignment: Alignment::Center,
        spacing: 8.0,
        children: contents.into_views(),
    }
}

/// Creates a depth stack with the given child views.
///
/// The stack will have center alignment by default.
/// Use methods on the returned `ZStack` to customize alignment.
pub fn zstack(contents: impl TupleViews) -> ZStack {
    ZStack {
        alignment: Alignment::Center,
        children: contents.into_views(),
    }
}
