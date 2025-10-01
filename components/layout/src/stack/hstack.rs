use alloc::{vec, vec::Vec};
use waterui_core::view::TupleViews;

use crate::{container, stack::VerticalAlignment, ChildMetadata, Layout, Point, ProposalSize, Rect, Size};

#[derive(Debug, Clone)]
pub struct HStackLayout {
    pub alignment: VerticalAlignment,
    pub spacing: f64,
}

impl Layout for HStackLayout {
    fn propose(
        &mut self,
        parent: ProposalSize,
        children: &[ChildMetadata],
    ) -> Vec<ProposalSize> {
        vec![ProposalSize::new(None, parent.height); children.len()]
    }

    fn size(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Size {
        if children.is_empty() {
            return Size::new(0.0, 0.0);
        }

        let has_stretchy_children = children.iter().any(|c| c.stretch());

        let non_stretchy_width: f64 = children
            .iter()
            .filter(|c| !c.stretch())
            .map(|c| c.proposal().width.unwrap_or(0.0))
            .sum();

        let total_spacing = if children.len() > 1 {
            (children.len() - 1) as f64 * self.spacing
        } else {
            0.0
        };

        let intrinsic_width = non_stretchy_width + total_spacing;


        let final_width = if has_stretchy_children {
            parent.width.unwrap_or(intrinsic_width)
        } else {
            intrinsic_width
        };

        let max_height = children
            .iter()
            .map(|c| c.proposal().height.unwrap_or(0.0))
            .max_by(f64::total_cmp)
            .unwrap_or(0.0);
        
        let final_height = parent.height.unwrap_or(max_height);

        Size::new(final_width, final_height)
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

        let stretchy_children_count = children.iter().filter(|c| c.stretch()).count();
        let non_stretchy_width: f64 = children
            .iter()
            .filter(|c| !c.stretch())
            .map(|c| c.proposal().width.unwrap_or(0.0))
            .sum();

        let total_spacing = if children.len() > 1 {
            (children.len() - 1) as f64 * self.spacing
        } else {
            0.0
        };
        
        let remaining_width = bound.width() - non_stretchy_width - total_spacing;
        let stretchy_child_width = if stretchy_children_count > 0 {
            (remaining_width / stretchy_children_count as f64).max(0.0)
        } else {
            0.0
        };

        let mut placements = Vec::with_capacity(children.len());
        let mut current_x = bound.origin().x;

        for (i, child) in children.iter().enumerate() {
            if i > 0 {
                current_x += self.spacing;
            }

            let child_proposal = child.proposal();
            let child_height = child_proposal.height.unwrap_or(0.0);
            let child_width = if child.stretch() {
                stretchy_child_width
            } else {
                child_proposal.width.unwrap_or(0.0)
            };

            let y = match self.alignment {
                VerticalAlignment::Top => bound.origin().y,
                VerticalAlignment::Center => bound.origin().y + (bound.height() - child_height) / 2.0,
                VerticalAlignment::Bottom => bound.origin().y + bound.height() - child_height,
            };

            let origin = Point::new(current_x, y);
            let size = Size::new(child_width, child_height);
            placements.push(Rect::new(origin, size));

            current_x += child_width;
        }

        placements
    }
}

container!(HStack, HStackLayout);

impl HStack {
    pub fn new(alignment: VerticalAlignment, spacing: f64, contents: impl TupleViews) -> Self {
        Self {
            layout: HStackLayout { alignment, spacing },
            contents: contents.into_views(),
        }
    }
}

pub fn hstack(contents: impl TupleViews) -> HStack {
    HStack::new(VerticalAlignment::Center, 10.0, contents)
}