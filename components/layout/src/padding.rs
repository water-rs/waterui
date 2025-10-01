use waterui_core::{AnyView, View};
use alloc::{vec::{Vec},vec};
use crate::{container, ChildMetadata, Layout, Point, ProposalSize, Rect, Size};

#[derive(Debug, Clone)]
pub struct PaddingLayout{
    edges: EdgeInsets,
}

impl Layout for PaddingLayout{
    fn propose(&mut self, parent: ProposalSize, children: &[crate::ChildMetadata]) -> alloc::vec::Vec<ProposalSize> {
        if children.is_empty() {
            return vec![];
        }

        // The horizontal and vertical space consumed by padding.
        let horizontal_padding = self.edges.leading + self.edges.trailing;
        let vertical_padding = self.edges.top + self.edges.bottom;

        // Reduce the proposed size for the child by the padding amount.
        // If the parent proposal is unconstrained (None), it remains None for the child.
        // Ensure the proposed dimension is not negative.
        let child_proposal = ProposalSize {
            width: parent.width.map(|w| (w - horizontal_padding).max(0.0)),
            height: parent.height.map(|h| (h - vertical_padding).max(0.0)),
        };

        vec![child_proposal]
    }

    fn size(&mut self, _parent: ProposalSize, children: &[crate::ChildMetadata]) -> Size {
        // The child's size is the base for our size calculation.
        let child_size = match children.get(0) {
            Some(child) => child.proposal(),
            None => return Size::new(0.0, 0.0), // Should not happen with Padding
        };

        // The final size is the child's size plus the padding.
        let total_width = child_size.width.unwrap_or(0.0) + self.edges.leading + self.edges.trailing;
        let total_height = child_size.height.unwrap_or(0.0) + self.edges.top + self.edges.bottom;

        Size::new(total_width, total_height)
    }

    fn place(
        &mut self,
        bound: Rect,
        _proposal: ProposalSize,
        children: &[ChildMetadata],
    ) -> Vec<Rect> {
        if children.is_empty() {
            return vec![];
        }

        // Create the child's frame by insetting the parent's bound by the padding amount.
        let child_origin = Point::new(bound.x() + self.edges.leading, bound.y() + self.edges.top);

        let horizontal_padding = self.edges.leading + self.edges.trailing;
        let vertical_padding = self.edges.top + self.edges.bottom;

        let child_size = Size::new(
            (bound.width() - horizontal_padding).max(0.0),
            (bound.height() - vertical_padding).max(0.0),
        );

        vec![Rect::new(child_origin, child_size)]
    }
}

#[derive(Debug,Clone,PartialEq)]
pub struct EdgeInsets{
    top:f64,
    bottom:f64,
    leading:f64,
    trailing:f64,
}

// Added convenient constructors for EdgeInsets
impl EdgeInsets {
    pub fn new(top: f64, bottom: f64, leading: f64, trailing: f64) -> Self {
        Self { top, bottom, leading, trailing }
    }

    pub fn all(value: f64) -> Self {
        Self {
            top: value,
            bottom: value,
            leading: value,
            trailing: value,
        }
    }

    pub fn default() -> Self {
        Self::all(10.0) // Default padding of 10.0 on all sides
    }

    pub fn symmetric(vertical: f64, horizontal: f64) -> Self {
        Self {
            top: vertical,
            bottom: vertical,
            leading: horizontal,
            trailing: horizontal,
        }
    }
}


pub struct Padding{
    layout: PaddingLayout,
    content: AnyView,
}

// Added a convenient constructor for Padding
impl Padding {
    pub fn new(edges: EdgeInsets, content: impl View + 'static) -> Self {
        Self {
            layout: PaddingLayout { edges },
            content: AnyView::new(content),
        }
    }
}

impl View for Padding{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        container::Container::new(self.layout, vec![self.content])
    }
}