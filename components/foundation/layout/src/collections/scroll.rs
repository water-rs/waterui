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
        self.generation.set(
            self.generation
                .get()
                .checked_add(1)
                .expect("scroll request generation overflow"),
        );
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
/// ```ignore
/// scroll(
///     vstack((
///         text("Item 1"),
///         text("Item 2"),
///         text("Item 3"),
///         // ... many more items
///     ))
/// )
/// ```
///
/// By default, `ScrollView` scrolls vertically. For horizontal scrolling:
///
/// ```ignore
/// scroll_horizontal(long_content)
/// ```
///
/// Or both directions:
///
/// ```ignore
/// scroll_both(large_image)
/// ```
#[derive(Debug)]
pub struct ScrollView {
    axis: Axis,
    content: AnyView,
    controller: Option<ScrollController<Point>>,
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
        }
    }

    /// Decomposes the `ScrollView` into its axis, content, and controller.
    pub fn into_inner(self) -> (Axis, AnyView, Option<ScrollController<Point>>) {
        (self.axis, self.content, self.controller)
    }

    /// Returns borrowed access to axis, content, and controller.
    pub const fn as_parts(&self) -> (Axis, &AnyView, Option<&ScrollController<Point>>) {
        (self.axis, &self.content, self.controller.as_ref())
    }

    /// Connects a programmatic scroll controller.
    #[must_use]
    pub fn scroll_controller(mut self, controller: &ScrollController<Point>) -> Self {
        self.controller = Some(controller.clone());
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
        assert_eq!(controller.target().get(), Point::new(0.0, 240.0));
        assert_eq!(controller.generation().get(), 1);

        controller.scroll_to(Point::new(0.0, 240.0));
        assert_eq!(controller.target().get(), Point::new(0.0, 240.0));
        assert_eq!(controller.generation().get(), 2);
    }
}
