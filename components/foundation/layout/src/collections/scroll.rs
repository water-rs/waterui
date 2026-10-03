//! Scroll containers that defer behaviour to the active renderer backend.

use nami::{Binding, Computed};
use waterui_core::{AnyView, View, raw_view};

use crate::{Point, StretchAxis};

/// Explicit programmatic control for a scrollable view.
///
/// A controller owns a reactive target and a monotonically increasing request
/// generation. The generation makes repeated requests to the same target
/// observable after the user has scrolled elsewhere.
#[derive(Clone, Debug)]
pub struct ScrollController<T: Clone + 'static> {
    target: Binding<T>,
    generation: Binding<i32>,
}

impl<T: Clone + 'static> ScrollController<T> {
    /// Creates a controller whose first target is `initial_target`.
    #[must_use]
    pub fn new(initial_target: T) -> Self {
        Self {
            target: Binding::container(initial_target),
            generation: Binding::container(0),
        }
    }

    /// Requests an immediate jump to `target`.
    ///
    /// # Panics
    ///
    /// Panics if the request generation exceeds [`i32::MAX`].
    pub fn scroll_to(&self, target: T) {
        self.target.set(target);
        self.generation.with_mut(|generation| {
            *generation = generation
                .checked_add(1)
                .expect("scroll request generation overflow");
        });
    }

    /// Returns the current requested target as a read-only signal.
    #[must_use]
    pub fn target(&self) -> Computed<T> {
        self.target.clone().into()
    }

    /// Returns the request generation as a read-only signal.
    #[must_use]
    pub fn generation(&self) -> Computed<i32> {
        self.generation.clone().into()
    }
}

impl<T> Default for ScrollController<T>
where
    T: Clone + Default + 'static,
{
    fn default() -> Self {
        Self::new(T::default())
    }
}

/// A scrollable view that displays content larger than its frame.
///
/// Use a `ScrollView` when you have content that might not fit in the available space.
/// The view automatically enables scrolling in the specified direction.
///
/// ```rust
/// # use waterui::prelude::*;
/// # fn feed() -> impl View {
/// scroll(
///     vstack((
///         text("Item 1"),
///         text("Item 2"),
///         text("Item 3"),
///         // ... many more items
///     ))
/// )
/// # }
/// ```
///
/// By default, `ScrollView` scrolls vertically. For horizontal scrolling:
///
/// ```rust
/// # use waterui::layout::scroll::scroll_horizontal;
/// # use waterui::prelude::*;
/// # fn carousel(long_content: impl View) -> impl View {
/// scroll_horizontal(long_content)
/// # }
/// ```
///
/// Or both directions:
///
/// ```rust
/// # use waterui::layout::scroll::scroll_both;
/// # use waterui::prelude::*;
/// # fn canvas(large_image: impl View) -> impl View {
/// scroll_both(large_image)
/// # }
/// ```
#[derive(Debug)]
pub struct ScrollView {
    axis: Axis,
    content: AnyView,
    controller: Option<ScrollController<Point>>,
    offset: Option<Binding<Point>>,
}

/// The parts a [`ScrollView`] is made of, as [`ScrollView::into_inner`]
/// hands them to a backend.
#[derive(Debug)]
#[non_exhaustive]
pub struct ScrollViewParts {
    /// The axis or axes the view scrolls along.
    pub axis: Axis,
    /// The scrolled content.
    pub content: AnyView,
    /// The programmatic scroll controller, if one is connected.
    pub controller: Option<ScrollController<Point>>,
    /// The binding the backend reports the content offset into, if one is
    /// connected.
    pub offset: Option<Binding<Point>>,
}

/// Defines the scrolling directions supported by `ScrollView`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[non_exhaustive]
pub enum Axis {
    /// Allow horizontal scrolling only.
    Horizontal,
    /// Allow vertical scrolling only (default).
    #[default]
    Vertical,
    /// Allow scrolling in both directions.
    All,
}

impl ScrollView {
    /// Creates a new `ScrollView` with the specified scroll axis and content.
    #[must_use]
    pub const fn new(axis: Axis, content: AnyView) -> Self {
        Self {
            axis,
            content,
            controller: None,
            offset: None,
        }
    }

    /// Decomposes the `ScrollView` into its parts.
    #[must_use]
    pub fn into_inner(self) -> ScrollViewParts {
        ScrollViewParts {
            axis: self.axis,
            content: self.content,
            controller: self.controller,
            offset: self.offset,
        }
    }

    /// The axis or axes the view scrolls along.
    #[must_use]
    pub const fn axis(&self) -> Axis {
        self.axis
    }

    /// The scrolled content.
    #[must_use = "this borrows the scrolled content without consuming it"]
    pub const fn content(&self) -> &AnyView {
        &self.content
    }

    /// Connects a programmatic scroll controller.
    #[must_use]
    pub fn scroll_controller(mut self, controller: &ScrollController<Point>) -> Self {
        self.controller = Some(controller.clone());
        self
    }

    /// Reports the content offset into `offset` as the view scrolls — the
    /// distance the content has moved from its origin, in points, whether the
    /// user or a [`ScrollController`] moved it.
    ///
    /// Chrome outside the scroll view follows it through the binding: a top
    /// app bar that lifts once content passes under it reads
    /// `offset.map(|offset| offset.y > 0.0)`. The backend writes only when the
    /// offset changes. The binding is written, never read, so setting it does
    /// not scroll; use a controller for that.
    #[must_use]
    pub fn report_offset(mut self, offset: &Binding<Point>) -> Self {
        self.offset = Some(offset.clone());
        self
    }

    /// Creates a `ScrollView` with horizontal scrolling.
    pub fn horizontal(content: impl View) -> Self {
        Self::new(Axis::Horizontal, AnyView::new(content))
    }

    /// Creates a `ScrollView` with vertical scrolling.
    pub fn vertical(content: impl View) -> Self {
        Self::new(Axis::Vertical, AnyView::new(content))
    }

    /// Creates a `ScrollView` with scrolling in both directions.
    pub fn both(content: impl View) -> Self {
        Self::new(Axis::All, AnyView::new(content))
    }
}

raw_view!(ScrollView, StretchAxis::Both);

/// Creates a vertical `ScrollView` with the given content.
///
/// This is the most common scroll direction for lists and long content.
/// The actual scrolling behavior is implemented by the renderer backend.
pub fn scroll(content: impl View) -> ScrollView {
    ScrollView::vertical(content)
}

/// Creates a horizontal `ScrollView` with the given content.
///
/// Useful for wide content that needs to scroll left-right.
/// The actual scrolling behavior is implemented by the renderer backend.
pub fn scroll_horizontal(content: impl View) -> ScrollView {
    ScrollView::horizontal(content)
}

/// Creates a `ScrollView` that can scroll in both directions.
///
/// Useful for large content like images or tables that may need both horizontal and vertical scrolling.
/// The actual scrolling behavior is implemented by the renderer backend.
pub fn scroll_both(content: impl View) -> ScrollView {
    ScrollView::both(content)
}

#[cfg(test)]
mod tests {
    use super::ScrollController;
    use crate::Point;
    use nami::Signal;

    #[test]
    fn repeated_target_requests_advance_generation() {
        let controller = ScrollController::new(Point::zero());

        controller.scroll_to(Point::new(0.0, 240.0));
        assert_eq!(controller.target().snapshot(), Point::new(0.0, 240.0));
        assert_eq!(controller.generation().snapshot(), 1);

        controller.scroll_to(Point::new(0.0, 240.0));
        assert_eq!(controller.target().snapshot(), Point::new(0.0, 240.0));
        assert_eq!(controller.generation().snapshot(), 2);
    }
}
