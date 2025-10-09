//! Overlay stack layout.

use alloc::{vec::{Vec},vec};
use nami::collection::Collection;
use waterui_core::{id::Identifable, view::TupleViews, views::{ForEach}, AnyView, View};

use crate::{container::{FixedContainer}, stack::Alignment, Container, Layout, Point, ProposalSize, Rect, Size};

/// A layout implementation for stacking views on top of each other with specified alignment.
///
/// `ZStackLayout` positions all child views within the same bounds, overlaying them
/// according to the specified alignment. Each child is sized independently and
/// positioned based on the alignment setting.
#[derive(Debug, Clone, Default)]
pub struct ZStackLayout {
    /// The alignment used to position children within the `ZStack`
    pub alignment: Alignment,
}

impl Layout for ZStackLayout {
    fn propose(
        &mut self,
        parent: ProposalSize,
        children: &[crate::ChildMetadata],
    ) -> Vec<ProposalSize> {
        // For ZStack, we propose the parent's size to each child
        // Each child gets to size itself based on the full available space
        vec![parent; children.len()]
    }

    #[allow(clippy::cast_precision_loss)]
    fn size(&mut self, parent: ProposalSize, children: &[crate::ChildMetadata]) -> Size {
        // ZStack's size is determined by the largest child
        // We find the maximum width and height among all children
        let mut max_width: f32 = 0.0;
        let mut max_height: f32 = 0.0;

        for child in children {
            if let Some(width) = child.proposal_width() {
                max_width = max_width.max(width);
            }
            if let Some(height) = child.proposal_height() {
                max_height = max_height.max(height);
            }
        }

        // If no children have explicit sizes, use parent constraints
        if max_width == 0.0 {
            max_width = parent.width.unwrap_or(0.0);
        }
        if max_height == 0.0 {
            max_height = parent.height.unwrap_or(0.0);
        }

        // Respect parent constraints - don't exceed them
        let final_width = parent
            .width
            .map_or(max_width, |parent_width| max_width.min(parent_width));

        let final_height = parent
            .height
            .map_or(max_height, |parent_height| max_height.min(parent_height));

        Size::new(final_width, final_height)
    }

    fn place(
        &mut self,
        bound: Rect,
        _proposal: ProposalSize,
        children: &[crate::ChildMetadata],
    ) -> Vec<Rect> {
        // ZStack places all children within the same bounds, positioned according to alignment
        if children.is_empty() {
            return Vec::new();
        }

        let mut rects = Vec::with_capacity(children.len());

        for child in children {
            // Each child gets sized to its ideal size, but constrained by the ZStack bounds
            let child_width = child
                .proposal_width()
                .unwrap_or_else(|| bound.width())
                .min(bound.width())
                .max(0.0);

            let child_height = child
                .proposal_height()
                .unwrap_or_else(|| bound.height())
                .min(bound.height())
                .max(0.0);

            // Position the child within the ZStack bounds according to alignment
            let (x, y) = self.calculate_position(&bound, &Size::new(child_width, child_height));

            rects.push(Rect::new(
                Point::new(x, y),
                Size::new(child_width, child_height),
            ));
        }

        rects
    }
}

impl ZStackLayout {
    /// Calculate the position of a child within the `ZStack` bounds based on alignment
    fn calculate_position(&self, bound: &Rect, child_size: &Size) -> (f32, f32) {
        let available_width = bound.width();
        let available_height = bound.height();

        match self.alignment {
            Alignment::TopLeading => (bound.x(), bound.y()),
            Alignment::Top => (
                bound.x() + (available_width - child_size.width) / 2.0,
                bound.y(),
            ),
            Alignment::TopTrailing => (bound.max_x() - child_size.width, bound.y()),
            Alignment::Leading => (
                bound.x(),
                bound.y() + (available_height - child_size.height) / 2.0,
            ),
            Alignment::Center => (
                bound.x() + (available_width - child_size.width) / 2.0,
                bound.y() + (available_height - child_size.height) / 2.0,
            ),
            Alignment::Trailing => (
                bound.max_x() - child_size.width,
                bound.y() + (available_height - child_size.height) / 2.0,
            ),
            Alignment::BottomLeading => (bound.x(), bound.max_y() - child_size.height),
            Alignment::Bottom => (
                bound.x() + (available_width - child_size.width) / 2.0,
                bound.max_y() - child_size.height,
            ),
            Alignment::BottomTrailing => (
                bound.max_x() - child_size.width,
                bound.max_y() - child_size.height,
            ),
        }
    }
}

/// A view that overlays its children, aligning them according to the specified alignment.
#[derive(Debug, Clone)]
pub struct ZStack<C> {
    layout: ZStackLayout,
    contents: C,
}

impl <C>ZStack<C> {
    /// Sets the alignment for the `ZStack`.
    #[must_use]
    pub const fn alignment(mut self, alignment: Alignment) -> Self {
        self.layout.alignment = alignment;
        self
    }
}

impl <C,F,V>ZStack<ForEach<C,F,V>>
where
        C: Collection,
        C::Item: Identifable,
        F: 'static + Fn(C::Item) -> V,
        V:View{
    /// Creates a new `ZStack` with views generated from a collection using `ForEach`.
    ///
    /// # Arguments
    /// * `collection` - The collection of items to iterate over
    /// * `generator` - A function that generates a view for each item in the collection
    pub fn for_each(collection: C, generator: F) -> Self
    {
        Self {
            layout: ZStackLayout::default(),
            contents: ForEach::new(collection, generator),
        }
    }
}

impl <C:TupleViews> ZStack<(C,)> {
    /// Creates a new `ZStack` with the specified alignment and contents.
    ///
    /// # Arguments
    /// * `alignment` - The alignment to use for positioning children within the stack
    /// * `contents` - A collection of views to be stacked
    pub const fn new(alignment: Alignment, contents: C) -> Self {
        Self {
            layout: ZStackLayout { alignment },
            contents: (contents,),
        }
    }
}

impl<V> FromIterator<V> for ZStack<(Vec<AnyView>,)>
where
    V: View,
{
    fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
        let contents = iter.into_iter().map(AnyView::new).collect::<Vec<_>>();
        Self::new(Alignment::default(), contents)
    }
}

/// Creates a new `ZStack` with center alignment and the specified contents.
///
/// This is a convenience function that creates a `ZStack` with `Alignment::Center`.
pub const fn zstack<C: TupleViews>(contents: C) -> ZStack<(C,)> {
    ZStack::new(Alignment::Center, contents)
}


impl<C> View for ZStack<(C,)>
where
    C: TupleViews + 'static,
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        FixedContainer::new(self.layout, self.contents.0)
    }
}

impl <C,F,V>View for ZStack<ForEach<C,F,V>>
where
        C: Collection,
        C::Item: Identifable,
        F: 'static + Fn(C::Item) -> V,
        V:View{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Container::new(self.layout, self.contents)
    }
}

