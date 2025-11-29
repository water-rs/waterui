//! Horizontal stack layout.

use alloc::{vec, vec::Vec};
use nami::collection::Collection;
use waterui_core::{AnyView, View, id::Identifable, view::TupleViews, views::ForEach};

use crate::{
    ChildMetadata, ChildPlacement, Container, Layout, LayoutContext, Point, ProposalSize, Rect,
    SafeAreaInsets, Size, container::FixedContainer, stack::VerticalAlignment,
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
    fn propose(
        &mut self,
        parent: ProposalSize,
        children: &[ChildMetadata],
        _context: &LayoutContext,
    ) -> Vec<ProposalSize> {
        // Per LAYOUT_SPEC.md:
        // - Width: None (child decides)
        // - Height: Parent's height proposal
        //
        // This allows:
        // - Children to measure intrinsic width (for sizing)
        // - Vertically-expanding views to fill available height
        vec![ProposalSize::new(None, parent.height); children.len()]
    }

    fn size(
        &mut self,
        parent: ProposalSize,
        children: &[ChildMetadata],
        _context: &LayoutContext,
    ) -> Size {
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

        // Calculate intrinsic height as the maximum height of NON-STRETCHY children.
        // Stretchy children (like Spacer) don't contribute to HStack's height.
        // This allows center alignment to work correctly.
        let max_height = children
            .iter()
            .filter(|c| !c.stretch())
            .map(|c| c.proposal().height.unwrap_or(0.0))
            .max_by(f32::total_cmp)
            .unwrap_or(0.0);

        // Use intrinsic height, but cap at parent's proposed height if any
        let final_height = match parent.height {
            Some(proposed) => max_height.min(proposed),
            None => max_height,
        };

        Size::new(final_width, final_height)
    }

    fn place(
        &mut self,
        bound: Rect,
        _proposal: ProposalSize,
        children: &[ChildMetadata],
        context: &LayoutContext,
    ) -> Vec<ChildPlacement> {
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
        let mut remaining_safe_area = context.safe_area.clone();

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
            let rect = Rect::new(origin, size);

            // Calculate safe area for this child:
            // - First child gets leading safe area
            // - Last child gets trailing safe area
            // - All children get top/bottom safe area
            let child_safe_area = SafeAreaInsets {
                top: remaining_safe_area.top,
                bottom: remaining_safe_area.bottom,
                leading: if i == 0 { remaining_safe_area.leading } else { 0.0 },
                trailing: if i == children.len() - 1 {
                    remaining_safe_area.trailing
                } else {
                    0.0
                },
            };

            let child_context = LayoutContext {
                safe_area: child_safe_area,
                ignores_safe_area: context.ignores_safe_area,
            };

            placements.push(ChildPlacement::new(rect, child_context));
            current_x += child_width;

            // Consume leading safe area after first child
            remaining_safe_area.leading = 0.0;
        }

        placements
    }
}

impl<C> HStack<(C,)> {
    /// Creates a horizontal stack with the provided alignment, spacing, and
    /// children.
    pub const fn new(alignment: VerticalAlignment, spacing: f32, contents: C) -> Self {
        Self {
            layout: HStackLayout { alignment, spacing },
            contents: (contents,),
        }
    }
}

impl<C> HStack<C> {
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
pub const fn hstack<C>(contents: C) -> HStack<(C,)> {
    HStack::new(VerticalAlignment::Center, 10.0, contents)
}

impl<C, F, V> View for HStack<ForEach<C, F, V>>
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

impl<C: TupleViews + 'static> View for HStack<(C,)> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        FixedContainer::new(self.layout, self.contents.0)
    }
}
