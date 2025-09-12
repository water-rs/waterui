use crate::Size;
use crate::engine::{Constraint, Container, Layout, LayoutResult, MeasuredChild};
use waterui_core::{AnyView, View};
/// Represents an overlay that can be displayed on top of other content.
#[derive(Debug)]
#[must_use]
#[non_exhaustive]
pub struct Overlay {
    /// The content to display in the overlay.
    pub content: AnyView,
}

impl Overlay {
    /// Creates a new `Overlay` with the specified content.
    pub fn new(content: impl View) -> Self {
        Self {
            content: AnyView::new(content),
        }
    }
}

/// Creates a new overlay with the specified content.
pub fn overlay(content: impl View) -> Overlay {
    Overlay::new(content)
}

impl Layout for Overlay {
    fn layout(self, constraint: Constraint, measured_children: &[MeasuredChild]) -> LayoutResult {
        // Overlay typically contains a single child that fills the available space
        if measured_children.is_empty() {
            return LayoutResult {
                size: Size {
                    width: 0.0,
                    height: 0.0,
                },
                child_positions: Vec::new(),
                child: AnyView::new(self),
            };
        }

        let child = &measured_children[0];
        let mut child_positions = Vec::new();

        // The overlay content should fill the available space
        let content_width = constraint
            .max
            .width
            .min(child.max_size.width)
            .max(child.min_size.width)
            .max(constraint.min.width);

        let content_height = constraint
            .max
            .height
            .min(child.max_size.height)
            .max(child.min_size.height)
            .max(constraint.min.height);

        // Center the content within the available space
        let child_x = (constraint.max.width - content_width) / 2.0;
        let child_y = (constraint.max.height - content_height) / 2.0;

        child_positions.push(Size {
            width: child_x.max(0.0),  // Store x position in width
            height: child_y.max(0.0), // Store y position in height
        });

        LayoutResult {
            size: Size {
                width: constraint.max.width.max(constraint.min.width),
                height: constraint.max.height.max(constraint.min.height),
            },
            child_positions,
            child: AnyView::new(self),
        }
    }
}

impl View for Overlay {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        // Overlay contains a single child
        Container::new(self)
    }
}
