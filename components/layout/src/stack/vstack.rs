//! Vertical stack layout.

use alloc::{vec, vec::Vec};
use waterui_core::{view::TupleViews, AnyView, View};

use crate::{
    ChildMetadata, Layout, Point, ProposalSize, Rect, Size, container, stack::HorizontalAlignment,
};

/// Layout engine shared by the public [`VStack`] view.
#[derive(Debug, Clone)]
pub struct VStackLayout {
    /// The horizontal alignment of children within the stack.
    pub alignment: HorizontalAlignment,
    /// The spacing between children in the stack.
    pub spacing: f64,
}

#[allow(clippy::cast_precision_loss)]
impl Layout for VStackLayout {
    fn propose(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Vec<ProposalSize> {
        vec![ProposalSize::new(parent.width, None); children.len()]
    }

    fn size(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Size {
        if children.is_empty() {
            return Size::new(0.0, 0.0);
        }

        let has_stretchy_children = children.iter().any(ChildMetadata::stretch);

        let non_stretchy_height: f64 = children
            .iter()
            .filter(|c| !c.stretch())
            .map(|c| c.proposal().height.unwrap_or(0.0))
            .sum();

        let total_spacing = if children.len() > 1 {
            (children.len() - 1) as f64 * self.spacing
        } else {
            0.0
        };

        let intrinsic_height = non_stretchy_height + total_spacing;

        let final_height = if has_stretchy_children {
            parent.height.unwrap_or(intrinsic_height)
        } else {
            intrinsic_height
        };

        let max_width = children
            .iter()
            .map(|c| c.proposal().width.unwrap_or(0.0))
            .max_by(f64::total_cmp)
            .unwrap_or(0.0);

        let final_width = parent.width.unwrap_or(max_width);

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
        let non_stretchy_height: f64 = children
            .iter()
            .filter(|c| !c.stretch())
            .map(|c| c.proposal().height.unwrap_or(0.0))
            .sum();

        let total_spacing = if children.len() > 1 {
            (children.len() - 1) as f64 * self.spacing
        } else {
            0.0
        };

        let remaining_height = bound.height() - non_stretchy_height - total_spacing;
        let stretchy_child_height = if stretchy_children_count > 0 {
            (remaining_height / stretchy_children_count as f64).max(0.0)
        } else {
            0.0
        };

        let mut placements = Vec::with_capacity(children.len());
        let mut current_y = bound.origin().y;

        for (i, child) in children.iter().enumerate() {
            if i > 0 {
                current_y += self.spacing;
            }

            let child_proposal = child.proposal();
            let child_width = child_proposal.width.unwrap_or(0.0);
            let child_height = if child.stretch() {
                stretchy_child_height
            } else {
                child_proposal.height.unwrap_or(0.0)
            };

            let x = match self.alignment {
                HorizontalAlignment::Leading => bound.origin().x,
                HorizontalAlignment::Center => {
                    bound.origin().x + (bound.width() - child_width) / 2.0
                }
                HorizontalAlignment::Trailing => bound.origin().x + bound.width() - child_width,
            };

            let origin = Point::new(x, current_y);
            let size = Size::new(child_width, child_height);
            placements.push(Rect::new(origin, size));

            current_y += child_height;
        }

        placements
    }
}
container!(
    VStack,
    VStackLayout,
    "A vertical stack layout that arranges its children in a vertical line with specified alignment and spacing."
);

impl VStack {
    /// Creates a vertical stack with the provided alignment, spacing, and
    /// children.
    pub fn new(alignment: HorizontalAlignment, spacing: f64, contents: impl TupleViews) -> Self {
        Self {
            layout: VStackLayout { alignment, spacing },
            contents: contents.into_views(),
        }
    }

    

    /// Sets the horizontal alignment of children within the stack.
    #[must_use] 
    pub const fn alignment(mut self, alignment: HorizontalAlignment) -> Self {
        self.layout.alignment = alignment;
        self
    }

    #[must_use] 
    pub const fn spacing(mut self, spacing: f64) -> Self {
        self.layout.spacing = spacing;
        self
    }
}


impl <V>FromIterator<V> for VStack where V: View {
    fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
        let contents = iter.into_iter().map(AnyView::new).collect::<Vec<_>>();
        Self::new(HorizontalAlignment::default(), 10.0, contents)
    }
}


/// Convenience constructor that centres children and uses the default spacing.
pub fn vstack(contents: impl TupleViews) -> VStack {
    VStack::new(HorizontalAlignment::Center, 10.0, contents)
}
