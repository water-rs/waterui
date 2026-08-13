//! View wrapper that lets arbitrary [`Layout`] implementations
//! participate in the `WaterUI` view tree.

use core::fmt;
use fmt::Debug;

use alloc::{boxed::Box, vec::Vec};
use nami::{Computed, Signal};
use waterui_core::{
    AnyView, Native, NativeView, View,
    layout::{
        HorizontalAlignment, LayoutDirection, LayoutInvalidationCallback, PlacedSubview,
        ProposalSize, Rect, Size, SubView, VerticalAlignment,
    },
    view::TupleViews,
    views::{AnyViews, Views, ViewsExt},
};

use crate::{Layout, StretchAxis};

struct DirectionalLayout {
    inner: Box<dyn Layout>,
    direction: Computed<LayoutDirection>,
}

impl DirectionalLayout {
    fn new(inner: Box<dyn Layout>, direction: Computed<LayoutDirection>) -> Self {
        Self { inner, direction }
    }

    fn mirror(&self, bounds: Rect, frame: Rect) -> Rect {
        if self.direction.get().is_right_to_left() {
            Rect::new(
                waterui_core::layout::Point::new(
                    bounds.min_x() + bounds.max_x() - frame.max_x(),
                    frame.y(),
                ),
                *frame.size(),
            )
        } else {
            frame
        }
    }
}

impl Debug for DirectionalLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirectionalLayout")
            .field("inner", &self.inner)
            .field("direction", &self.direction.get())
            .finish()
    }
}

impl Layout for DirectionalLayout {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        self.inner.size_that_fits(proposal, children)
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        self.inner
            .place(bounds, children)
            .into_iter()
            .map(|frame| self.mirror(bounds, frame))
            .collect()
    }

    fn explicit_horizontal(
        &self,
        alignment: HorizontalAlignment,
        bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        if !self.direction.get().is_right_to_left() {
            return self.inner.explicit_horizontal(alignment, bounds, children);
        }
        let unmirrored = children
            .iter()
            .map(|child| PlacedSubview::new(child.view, self.mirror(bounds, child.frame)))
            .collect::<Vec<_>>();
        self.inner
            .explicit_horizontal(alignment, bounds, &unmirrored)
            .map(|guide| bounds.min_x() + bounds.max_x() - guide)
    }

    fn explicit_vertical(
        &self,
        alignment: VerticalAlignment,
        bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        if !self.direction.get().is_right_to_left() {
            return self.inner.explicit_vertical(alignment, bounds, children);
        }
        let unmirrored = children
            .iter()
            .map(|child| PlacedSubview::new(child.view, self.mirror(bounds, child.frame)))
            .collect::<Vec<_>>();
        self.inner.explicit_vertical(alignment, bounds, &unmirrored)
    }

    fn explicit_horizontal_alignments(&self) -> Vec<HorizontalAlignment> {
        self.inner.explicit_horizontal_alignments()
    }

    fn explicit_vertical_alignments(&self) -> Vec<VerticalAlignment> {
        self.inner.explicit_vertical_alignments()
    }

    fn stretch_axis(&self, children: &[StretchAxis]) -> StretchAxis {
        self.inner.stretch_axis(children)
    }

    fn watch_invalidation(
        &self,
        invalidate: LayoutInvalidationCallback,
    ) -> Vec<nami::watcher::BoxWatcherGuard> {
        let mut guards = self.inner.watch_invalidation(invalidate.clone());
        guards.push(self.direction.watch(move |_| invalidate()));
        guards
    }
}

/// A view wrapper that executes an arbitrary [`Layout`]
/// implementation.
pub struct FixedContainer {
    layout: Box<dyn Layout>,
    contents: Vec<AnyView>,
}

impl Debug for FixedContainer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Container")
            .field("layout", &"Box<dyn Layout>")
            .field("contents", &self.contents)
            .finish()
    }
}

impl FixedContainer {
    /// Wraps the supplied layout object and tuple of child views into a
    /// container view.
    pub fn new(layout: impl Layout + 'static, contents: impl TupleViews) -> Self {
        Self {
            layout: Box::new(layout),
            contents: contents.into_views(),
        }
    }

    /// Returns the boxed layout object together with the collected child views.
    #[must_use]
    pub fn into_inner(self) -> (Box<dyn Layout>, Vec<AnyView>) {
        (self.layout, self.contents)
    }

    /// Creates a fixed container from pre-built layout and children parts.
    #[must_use]
    pub fn from_parts(layout: Box<dyn Layout>, contents: Vec<AnyView>) -> Self {
        Self { layout, contents }
    }

    /// Returns borrowed access to the boxed layout and collected children.
    #[must_use = "this borrows the container's parts without consuming it"]
    pub fn as_parts(&self) -> (&dyn Layout, &[AnyView]) {
        (self.layout.as_ref(), &self.contents)
    }
}

impl FixedContainer {
    /// What each child says about itself, in order, for the layout to answer from.
    fn child_stretch_axes(&self) -> Vec<StretchAxis> {
        self.contents.iter().map(View::stretch_axis).collect()
    }
}

impl NativeView for FixedContainer {
    fn stretch_axis(&self) -> StretchAxis {
        self.layout.stretch_axis(&self.child_stretch_axes())
    }
}

impl View for FixedContainer {
    fn body(mut self, env: &waterui_core::Environment) -> impl View {
        self.layout = Box::new(DirectionalLayout::new(
            self.layout,
            waterui_core::layout::layout_direction(env),
        ));
        Native::new(self)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.layout.stretch_axis(&self.child_stretch_axes())
    }
}

/// A view wrapper that executes an arbitrary [`Layout`] implementation
/// with reconstructable views, which can support lazy layouting.
///
/// Unlike [`FixedContainer`], this container stores views as an [`AnyViews`]
/// collection, allowing backends to reconstruct views on-demand for efficient
/// rendering of large lists.
#[derive(Debug)]
pub struct LazyContainer {
    layout: Box<dyn Layout>,
    contents: AnyViews<AnyView>,
    direction: Computed<LayoutDirection>,
}

impl LazyContainer {
    /// Wraps the supplied layout object and views into a lazy container view.
    pub fn new<V: View>(
        layout: impl Layout + 'static,
        contents: impl Views<View = V> + 'static,
    ) -> Self {
        Self {
            layout: Box::new(layout),
            contents: AnyViews::new(contents.map(|v| AnyView::new(v))),
            direction: Computed::constant(LayoutDirection::default()),
        }
    }
    /// Returns the boxed layout object together with the collected child views.
    #[must_use]
    pub fn into_inner(self) -> (Box<dyn Layout>, AnyViews<AnyView>) {
        (self.layout, self.contents)
    }

    /// Returns borrowed access to the boxed layout and lazy child collection.
    #[must_use]
    pub fn as_parts(&self) -> (&dyn Layout, &AnyViews<AnyView>) {
        (self.layout.as_ref(), &self.contents)
    }

    /// Returns the resolved logical layout direction for lazy placement.
    #[must_use]
    pub fn direction(&self) -> Computed<LayoutDirection> {
        self.direction.clone()
    }
}

impl LazyContainer {
    /// A lazy container cannot enumerate its children without materializing the
    /// collection, which is the one thing it exists to avoid, so its layout
    /// answers with no children. Every layout that virtualizes — the two stacks —
    /// is content-sized and ignores them anyway.
    const fn child_stretch_axes() -> &'static [StretchAxis] {
        &[]
    }
}

impl NativeView for LazyContainer {
    fn stretch_axis(&self) -> StretchAxis {
        self.layout.stretch_axis(Self::child_stretch_axes())
    }
}

impl View for LazyContainer {
    fn body(mut self, env: &waterui_core::Environment) -> impl View {
        // Keep the concrete stack layout visible to backends: lazy-stack
        // virtualization identifies its axis from that type. Direction is a
        // separate signal so placement can mirror without materializing items.
        self.direction = waterui_core::layout::layout_direction(env);
        Native::new(self)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.layout.stretch_axis(Self::child_stretch_axes())
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use waterui_core::layout::Point;

    #[derive(Debug)]
    struct OffsetLayout;

    impl Layout for OffsetLayout {
        fn size_that_fits(&self, _proposal: ProposalSize, _children: &[&dyn SubView]) -> Size {
            Size::new(100.0, 40.0)
        }

        fn place(&self, _bounds: Rect, _children: &[&dyn SubView]) -> Vec<Rect> {
            vec![Rect::new(Point::new(10.0, 4.0), Size::new(20.0, 12.0))]
        }
    }

    #[test]
    fn right_to_left_direction_mirrors_horizontal_placement() {
        let layout = DirectionalLayout::new(
            Box::new(OffsetLayout),
            Computed::constant(LayoutDirection::RightToLeft),
        );
        let frames = layout.place(Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 40.0)), &[]);

        assert_eq!(
            frames,
            vec![Rect::new(Point::new(70.0, 4.0), Size::new(20.0, 12.0))]
        );
    }
}
