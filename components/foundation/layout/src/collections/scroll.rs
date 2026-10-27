//! Scroll containers that defer behaviour to the active renderer backend.

use nami::{Binding, Computed};
use waterui_core::{AnyView, Environment, Native, NativeView, View, animation::Animation};

use crate::{Point, Size, StretchAxis};

/// One programmatic scroll request: where the scroll view goes and how it
/// gets there.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ScrollRequest<T> {
    /// The position the scroll view moves to.
    pub target: T,
    /// How the scroll view moves there. `None` jumps in one frame.
    /// [`Animation::Default`] uses the backend's default scroll motion: the
    /// native smooth scroll on a backend that drives a platform scroll view,
    /// the backend's default animation curve on a self-drawn one. Any other
    /// animation drives the offset along its own curve and duration.
    ///
    /// The C FFI does not carry the animation yet, so the Kotlin Android
    /// runtime, which consumes it, jumps for every request
    /// (water-rs/waterui#2084).
    pub animation: Option<Animation>,
}

/// The scroll viewport's last reported actual position.
///
/// A backend writes one of these into a [`ScrollController`] after every
/// layout or scroll that moves the viewport: user drags and wheel ticks,
/// programmatic requests, resizes and content-size changes alike. `offset`
/// is the content offset the viewport's origin sits at, `viewport_size` the
/// visible scroll area, `content_size` the scrollable extent — all in points.
///
/// `at_start`/`at_end` report edge contact across every axis the view
/// scrolls along: a view smaller than its viewport is both at start and at
/// end. Follow-tail UIs read them through [`Computed::map`] — for example
/// `controller.position().map(|p| p.is_some_and(|p| p.at_end))`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ScrollPosition {
    /// The content offset the viewport's origin sits at, in points.
    pub offset: Point,
    /// The size of the visible scroll area, in points.
    pub viewport_size: Size,
    /// The scrollable extent, in points.
    pub content_size: Size,
    /// Whether the viewport touches the start of every scrollable axis.
    pub at_start: bool,
    /// Whether the viewport touches the end of every scrollable axis.
    pub at_end: bool,
}

impl ScrollPosition {
    /// Computes a position snapshot from the viewport's offset and extents.
    ///
    /// Edge flags consider only the axes `axis` scrolls along: an offset of
    /// `0` or less is at start, an offset at or past
    /// `max(content - viewport, 0)` is at end — no tolerance is applied.
    /// Content that does not exceed its viewport reports both flags on that
    /// axis.
    #[must_use]
    pub fn new(axis: Axis, offset: Point, viewport_size: Size, content_size: Size) -> Self {
        let (horizontal, vertical) = match axis {
            Axis::Horizontal => (true, false),
            Axis::Vertical => (false, true),
            Axis::All => (true, true),
        };
        let max_x = (content_size.width - viewport_size.width).max(0.0);
        let max_y = (content_size.height - viewport_size.height).max(0.0);
        Self {
            offset,
            viewport_size,
            content_size,
            at_start: (!horizontal || offset.x <= 0.0) && (!vertical || offset.y <= 0.0),
            at_end: (!horizontal || offset.x >= max_x) && (!vertical || offset.y >= max_y),
        }
    }
}

/// Explicit programmatic control for a scrollable view.
///
/// A controller owns the latest [`ScrollRequest`] and a monotonically
/// increasing request generation. The generation makes repeated requests to
/// the same target observable after the user has scrolled elsewhere.
///
/// ```rust
/// use waterui::animation::Animation;
/// use waterui::layout::{Point, scroll::ScrollController};
///
/// let controller = ScrollController::new(Point::zero());
/// // Jump straight to the target.
/// controller.scroll_to(Point::new(0.0, 2_400.0));
/// // Glide back with the backend's default scroll motion.
/// controller.animate_to(Point::zero(), Animation::default());
/// ```
#[derive(Clone, Debug)]
pub struct ScrollController<T: Clone + 'static> {
    request: Binding<ScrollRequest<T>>,
    generation: Binding<i32>,
    /// The viewport's last reported actual [`ScrollPosition`]: `None` until
    /// the backend lays the view out once, then the latest measured
    /// position, kept across clones of this controller and across the view
    /// unmounting — it is never reset.
    position: Binding<Option<ScrollPosition>>,
}

impl<T: Clone + 'static> ScrollController<T> {
    /// Creates a controller whose first target is `initial_target`.
    #[must_use]
    pub fn new(initial_target: T) -> Self {
        Self {
            request: Binding::container(ScrollRequest {
                target: initial_target,
                animation: None,
            }),
            generation: Binding::container(0),
            position: Binding::container(None),
        }
    }

    /// Requests an immediate jump to `target`.
    ///
    /// # Panics
    ///
    /// Panics if the request generation exceeds [`i32::MAX`].
    pub fn scroll_to(&self, target: T) {
        self.submit(target, None);
    }

    /// Requests an animated scroll to `target`, moving as
    /// [`ScrollRequest::animation`] describes. A later request, or the user
    /// scrolling, takes over from an animation still in flight.
    ///
    /// # Panics
    ///
    /// Panics if the request generation exceeds [`i32::MAX`].
    pub fn animate_to(&self, target: T, animation: Animation) {
        self.submit(target, Some(animation));
    }

    fn submit(&self, target: T, animation: Option<Animation>) {
        self.request.set(ScrollRequest { target, animation });
        self.generation.with_mut(|generation| {
            *generation = generation
                .checked_add(1)
                .expect("scroll request generation overflow");
        });
    }

    /// Returns the latest request as a read-only signal.
    #[must_use]
    pub fn request(&self) -> Computed<ScrollRequest<T>> {
        self.request.clone().into()
    }

    /// Returns the request generation as a read-only signal.
    #[must_use]
    pub fn generation(&self) -> Computed<i32> {
        self.generation.clone().into()
    }

    /// Returns the viewport's last reported actual position as a read-only
    /// signal.
    ///
    /// `None` until the backend lays the bound view out once; afterwards the
    /// latest [`ScrollPosition`] the backend measured — user scrolls,
    /// requests this controller issued, resizes and content-size changes
    /// alike. It is the last actual position, never a pending request, and
    /// it is retained: cloning the controller shares the same reported
    /// position, and unmounting the view does not clear it. The signal is
    /// read-only for the app — writing it does not scroll; use
    /// [`Self::scroll_to`] for that.
    ///
    /// Read edge contact through [`Computed::map`], e.g.
    /// `controller.position().map(|p| p.is_some_and(|p| p.at_end))` for a
    /// follow-tail chat view.
    #[must_use]
    pub fn position(&self) -> Computed<Option<ScrollPosition>> {
        self.position.clone().into()
    }

    /// The sink a backend writes the measured [`ScrollPosition`] into.
    ///
    /// Backend realization crates attach this binding and write every actual
    /// position change; it is not part of the app's API.
    #[doc(hidden)]
    #[must_use]
    pub fn position_report(&self) -> Binding<Option<ScrollPosition>> {
        self.position.clone()
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

/// The axis a scroll stretches on: always its scrolling axis — the scroll
/// fills the offer along it — and the non-scrolling axis only when the
/// content itself stretches there, so a scroll hugs non-filling content
/// instead of claiming the whole cross-axis slot (water-rs/waterui#1208).
///
/// Content `MainAxis`/`CrossAxis` are claims a containing *stack* resolves
/// (stack.rs); a scroll view is not a stack, so they claim nothing here —
/// only a concrete `Horizontal`/`Vertical`/`Both` content answer stretches
/// the scroll on its non-scrolling axis.
#[doc(hidden)]
#[must_use]
pub const fn scroll_stretch_axis(axis: Axis, content: StretchAxis) -> StretchAxis {
    let cross_stretches = match axis {
        Axis::Vertical => content.stretches_horizontal(),
        Axis::Horizontal => content.stretches_vertical(),
        Axis::All => return StretchAxis::Both,
    };
    match (axis, cross_stretches) {
        (Axis::Vertical, false) => StretchAxis::Vertical,
        (Axis::Horizontal, false) => StretchAxis::Horizontal,
        _ => StretchAxis::Both,
    }
}

impl NativeView for ScrollView {
    fn stretch_axis(&self) -> StretchAxis {
        scroll_stretch_axis(self.axis, self.content.stretch_axis())
    }
}

impl View for ScrollView {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self)
    }

    fn stretch_axis(&self) -> StretchAxis {
        scroll_stretch_axis(self.axis, self.content.stretch_axis())
    }
}

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
    use super::{Axis, ScrollController, ScrollPosition, ScrollRequest, scroll_stretch_axis};
    use crate::{Point, Size, StretchAxis};
    use core::time::Duration;
    use nami::Signal;
    use waterui_core::animation::Animation;

    #[test]
    fn repeated_target_requests_advance_generation() {
        let controller = ScrollController::new(Point::zero());

        controller.scroll_to(Point::new(0.0, 240.0));
        assert_eq!(
            controller.request().snapshot(),
            ScrollRequest {
                target: Point::new(0.0, 240.0),
                animation: None,
            }
        );
        assert_eq!(controller.generation().snapshot(), 1);

        let animation = Animation::ease_out(Duration::from_millis(300));
        controller.animate_to(Point::new(0.0, 240.0), animation.clone());
        assert_eq!(
            controller.request().snapshot(),
            ScrollRequest {
                target: Point::new(0.0, 240.0),
                animation: Some(animation),
            }
        );
        assert_eq!(controller.generation().snapshot(), 2);
    }

    #[test]
    fn scroll_stretch_axis_always_stretches_on_the_scrolling_axis() {
        for content in [
            StretchAxis::None,
            StretchAxis::Horizontal,
            StretchAxis::Vertical,
            StretchAxis::Both,
            StretchAxis::MainAxis,
            StretchAxis::CrossAxis,
        ] {
            assert!(
                scroll_stretch_axis(Axis::Vertical, content).stretches_vertical(),
                "vertical scroll stretches vertically under {content:?}"
            );
            assert!(
                scroll_stretch_axis(Axis::Horizontal, content).stretches_horizontal(),
                "horizontal scroll stretches horizontally under {content:?}"
            );
        }
    }

    #[test]
    fn scroll_stretch_axis_claims_the_cross_axis_only_with_filling_content() {
        // A horizontal scroll hugs its content's height: only content that
        // itself stretches vertically lifts the scroll to `Both`.
        assert_eq!(
            scroll_stretch_axis(Axis::Horizontal, StretchAxis::Vertical),
            StretchAxis::Both
        );
        assert_eq!(
            scroll_stretch_axis(Axis::Horizontal, StretchAxis::Both),
            StretchAxis::Both
        );
        for content in [
            StretchAxis::None,
            StretchAxis::Horizontal,
            StretchAxis::MainAxis,
            StretchAxis::CrossAxis,
        ] {
            assert_eq!(
                scroll_stretch_axis(Axis::Horizontal, content),
                StretchAxis::Horizontal,
                "horizontal scroll under {content:?}"
            );
        }
        // Symmetric on the vertical scroll's width.
        assert_eq!(
            scroll_stretch_axis(Axis::Vertical, StretchAxis::Horizontal),
            StretchAxis::Both
        );
        assert_eq!(
            scroll_stretch_axis(Axis::Vertical, StretchAxis::Both),
            StretchAxis::Both
        );
        for content in [
            StretchAxis::None,
            StretchAxis::Vertical,
            StretchAxis::MainAxis,
            StretchAxis::CrossAxis,
        ] {
            assert_eq!(
                scroll_stretch_axis(Axis::Vertical, content),
                StretchAxis::Vertical,
                "vertical scroll under {content:?}"
            );
        }
    }

    #[test]
    fn scroll_stretch_axis_all_stretches_on_both_axes() {
        for content in [
            StretchAxis::None,
            StretchAxis::Horizontal,
            StretchAxis::Vertical,
            StretchAxis::Both,
            StretchAxis::MainAxis,
            StretchAxis::CrossAxis,
        ] {
            assert_eq!(
                scroll_stretch_axis(Axis::All, content),
                StretchAxis::Both,
                "omnidirectional scroll under {content:?}"
            );
        }
    }

    #[test]
    fn scroll_position_edges_follow_the_scrolling_axis() {
        let viewport = Size::new(100.0, 100.0);
        let content = Size::new(400.0, 300.0);

        // Vertical: only y decides the edges.
        let top = ScrollPosition::new(Axis::Vertical, Point::new(0.0, 0.0), viewport, content);
        assert!(top.at_start && !top.at_end);
        let middle =
            ScrollPosition::new(Axis::Vertical, Point::new(390.0, 150.0), viewport, content);
        assert!(!middle.at_start && !middle.at_end);
        let bottom = ScrollPosition::new(Axis::Vertical, Point::new(0.0, 200.0), viewport, content);
        assert!(!bottom.at_start && bottom.at_end);

        // Horizontal: only x decides the edges.
        let left = ScrollPosition::new(Axis::Horizontal, Point::new(0.0, 0.0), viewport, content);
        assert!(left.at_start && !left.at_end);
        let right =
            ScrollPosition::new(Axis::Horizontal, Point::new(300.0, 0.0), viewport, content);
        assert!(!right.at_start && right.at_end);

        // Both axes must reach an edge together.
        let corner = ScrollPosition::new(Axis::All, Point::new(300.0, 200.0), viewport, content);
        assert!(!corner.at_start && corner.at_end);
        let half = ScrollPosition::new(Axis::All, Point::new(300.0, 0.0), viewport, content);
        assert!(!half.at_start && !half.at_end);
        let origin = ScrollPosition::new(Axis::All, Point::new(0.0, 0.0), viewport, content);
        assert!(origin.at_start && !origin.at_end);
    }

    #[test]
    fn scroll_position_shorter_content_is_at_both_edges() {
        let position = ScrollPosition::new(
            Axis::Vertical,
            Point::new(0.0, 0.0),
            Size::new(100.0, 200.0),
            Size::new(100.0, 50.0),
        );
        assert!(position.at_start && position.at_end);
    }

    #[test]
    fn scroll_position_is_shared_and_unmoved_by_requests() {
        let controller = ScrollController::new(Point::zero());
        assert_eq!(controller.position().snapshot(), None);

        let reported = ScrollPosition::new(
            Axis::Vertical,
            Point::new(0.0, 20.0),
            Size::new(100.0, 100.0),
            Size::new(100.0, 300.0),
        );
        controller.position_report().set(Some(reported));

        // Clones share the same reported position …
        let clone = controller.clone();
        assert_eq!(clone.position().snapshot(), Some(reported));

        // … and submitting a request does not move the actual position.
        controller.scroll_to(Point::new(0.0, 240.0));
        assert_eq!(controller.position().snapshot(), Some(reported));
    }
}
