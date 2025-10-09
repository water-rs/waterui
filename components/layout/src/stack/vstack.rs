//! Vertical stack layout.

use alloc::{vec, vec::Vec};
use nami::collection::Collection;
use waterui_core::{
    AnyView, View,
    id::Identifable,
    view::TupleViews,
    views::ForEach,
};

use crate::{
    ChildMetadata, Container, Layout, Point, ProposalSize, Rect, Size, container::FixedContainer,
    stack::HorizontalAlignment,
};

/// Layout engine shared by the public [`VStack`] view.
#[derive(Debug, Default, Clone)]
pub struct VStackLayout {
    /// The horizontal alignment of children within the stack.
    pub alignment: HorizontalAlignment,
    /// The spacing between children in the stack.
    pub spacing: f32,
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

        let non_stretchy_height: f32 = children
            .iter()
            .filter(|c| !c.stretch())
            .map(|c| c.proposal().height.unwrap_or(0.0))
            .sum();

        let total_spacing = if children.len() > 1 {
            (children.len() - 1) as f32 * self.spacing
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
            .max_by(f32::total_cmp)
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
        let non_stretchy_height: f32 = children
            .iter()
            .filter(|c| !c.stretch())
            .map(|c| c.proposal().height.unwrap_or(0.0))
            .sum();

        let total_spacing = if children.len() > 1 {
            (children.len() - 1) as f32 * self.spacing
        } else {
            0.0
        };

        let remaining_height = bound.height() - non_stretchy_height - total_spacing;
        let stretchy_child_height = if stretchy_children_count > 0 {
            (remaining_height / stretchy_children_count as f32).max(0.0)
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

/// A vertical stack view that arranges its children in a column.
#[derive(Debug, Clone)]
pub struct VStack<C> {
    layout: VStackLayout,
    contents: C,
}

impl<C: TupleViews> VStack<(C,)> {
    /// Creates a vertical stack with the provided alignment, spacing, and
    /// children.
    pub const fn new(alignment: HorizontalAlignment, spacing: f32, contents: C) -> Self {
        Self {
            layout: VStackLayout { alignment, spacing },
            contents: (contents,),
        }
    }
}
impl<C, F, V> VStack<ForEach<C, F, V>>
where
    C: Collection,
    C::Item: Identifable,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
    /// Creates a vertical stack by iterating over a collection and generating views.
    pub fn for_each(collection: C, generator: F) -> Self {
        Self {
            layout: VStackLayout::default(),
            contents: ForEach::new(collection, generator),
        }
    }
}

impl<C> VStack<C> {
    /// Sets the horizontal alignment for children in the stack.
    #[must_use]
    pub const fn alignment(mut self, alignment: HorizontalAlignment) -> Self {
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

impl<V> FromIterator<V> for VStack<(Vec<AnyView>,)>
where
    V: View,
{
    fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
        let contents = iter.into_iter().map(AnyView::new).collect::<Vec<_>>();
        Self::new(HorizontalAlignment::default(), 10.0, contents)
    }
}

/// Convenience constructor that centres children and uses the default spacing.
pub const fn vstack<C: TupleViews>(contents: C) -> VStack<(C,)> {
    VStack::new(HorizontalAlignment::Center, 10.0, contents)
}

impl<C, F, V> View for VStack<ForEach<C, F, V>>
where
    C: Collection,
    C::Item: Identifable,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Container::new(self.layout, self.contents)
    }
}

impl<C: TupleViews + 'static> View for VStack<(C,)> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        FixedContainer::new(self.layout, self.contents.0)
    }
}
