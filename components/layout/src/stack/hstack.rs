//! Horizontal stack layout.

use alloc::{vec, vec::Vec};
use nami::collection::Collection;
use waterui_core::{id::Identifable, view::TupleViews, views::{ForEach}, AnyView, View};

use crate::{
    container::{FixedContainer}, stack::VerticalAlignment, ChildMetadata, Container, Layout, Point, ProposalSize, Rect, Size
};

/// A horizontal stack that arranges its children in a horizontal line.
#[derive(Debug, Clone)]
pub struct HStack<C> {
    layout: HStackLayout,
    contents: C,
}

/// Layout engine shared by the public [`HStack`] view.
#[derive(Debug, Clone)]
pub struct HStackLayout {
    /// The vertical alignment of children within the stack.
    pub alignment: VerticalAlignment,
    /// The spacing between children in the stack.
    pub spacing: f32,
}

#[allow(clippy::cast_precision_loss)]
impl Layout for HStackLayout {
    fn propose(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Vec<ProposalSize> {
        vec![ProposalSize::new(None, parent.height); children.len()]
    }

    fn size(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Size {
        if children.is_empty() {
            return Size::new(0.0, 0.0);
        }

        let has_stretchy_children = children.iter().any(ChildMetadata::stretch);

        let non_stretchy_width: f32 = children
            .iter()
            .filter(|c| !c.stretch())
            .map(|c| c.proposal().width.unwrap_or(0.0))
            .sum();

        let total_spacing = if children.len() > 1 {
            (children.len() - 1) as f32 * self.spacing
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
            .max_by(f32::total_cmp)
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
        let non_stretchy_width: f32 = children
            .iter()
            .filter(|c| !c.stretch())
            .map(|c| c.proposal().width.unwrap_or(0.0))
            .sum();

        let total_spacing = if children.len() > 1 {
            (children.len() - 1) as f32 * self.spacing
        } else {
            0.0
        };

        let remaining_width = bound.width() - non_stretchy_width - total_spacing;
        let stretchy_child_width = if stretchy_children_count > 0 {
            (remaining_width / stretchy_children_count as f32).max(0.0)
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
                VerticalAlignment::Center => {
                    bound.origin().y + (bound.height() - child_height) / 2.0
                }
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

impl <C>HStack<(C,)> {
    /// Creates a horizontal stack with the provided alignment, spacing, and
    /// children.
    pub const fn new(alignment: VerticalAlignment, spacing: f32, contents: C) -> Self {
        Self {
            layout: HStackLayout { alignment, spacing },
            contents: (contents,),
        }
    }
}

impl<C>HStack<C>{

    /// Sets the vertical alignment for children in the stack.
    #[must_use]
    pub const fn alignment(mut self, alignment: VerticalAlignment) -> Self {
        self.layout.alignment = alignment;
        self
    }

    /// Sets the spacing between children in the stack.
    #[must_use]
    pub const fn spacing(mut self, spacing: f32) -> Self {
        self.layout.spacing = spacing;
        self
    }
}

impl<V> FromIterator<V> for HStack<(Vec<AnyView>,)>
where
    V: View,
{
    fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
        let contents = iter.into_iter().map(AnyView::new).collect();
        Self::new(VerticalAlignment::default(), 10.0, contents)
    }
}


/// Convenience constructor that centres children and uses the default spacing.
pub const fn hstack<C>(contents: C) -> HStack<(C,)>{
    HStack::new(VerticalAlignment::Center, 10.0, contents)
}


impl<C,F,V>View for HStack<ForEach<C,F,V>>
where
        C: Collection,
        C::Item: Identifable,
        F: 'static + Fn(C::Item) -> V,
        V:View{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Container::new(self.layout, self.contents)
    }
}

impl<C:TupleViews+'static>View for HStack<(C,)>
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        FixedContainer::new(self.layout, self.contents.0)
    }
}