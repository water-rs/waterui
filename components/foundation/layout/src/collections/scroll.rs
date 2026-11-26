//! Scroll containers that defer behaviour to the active renderer backend.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::fmt;

use nami::{Binding, Computed, SignalExt};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::id::{Id, Mapping};
use waterui_core::{
    AnyView, Environment, Metadata, Native, NativeView, View, animation::Animation,
};

use crate::{Point, Size, StretchAxis};

/// One programmatic scroll request: where the scroll view goes and how it
/// gets there.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ScrollRequest<T> {
    /// The position the scroll view moves to.
    target: T,
    /// How the scroll view moves there. `None` jumps in one frame.
    /// [`Animation::DEFAULT`] uses the backend's default scroll motion: the
    /// native smooth scroll on a backend that drives a platform scroll view,
    /// the backend's default animation curve on a self-drawn one. Any other
    /// animation drives the offset along its own curve and duration.
    ///
    /// The C FFI does not carry the animation yet, so the Kotlin Android
    /// runtime, which consumes it, jumps for every request
    /// (water-rs/waterui#2084).
    animation: Option<Animation>,
}

impl<T> ScrollRequest<T> {
    /// The position the scroll view moves to.
    #[must_use]
    pub const fn target(&self) -> &T {
        &self.target
    }

    /// How the scroll view moves there. `None` jumps in one frame.
    /// [`Animation::DEFAULT`] uses the backend's default scroll motion: the
    /// native smooth scroll on a backend that drives a platform scroll view,
    /// the backend's default animation curve on a self-drawn one. Any other
    /// animation drives the offset along its own curve and duration.
    ///
    /// The C FFI does not carry the animation yet, so the Kotlin Android
    /// runtime, which consumes it, jumps for every request
    /// (water-rs/waterui#2084).
    #[must_use]
    pub const fn animation(&self) -> Option<&Animation> {
        self.animation.as_ref()
    }

    pub(crate) fn map<U>(self, f: impl FnOnce(T) -> U) -> ScrollRequest<U> {
        ScrollRequest {
            target: f(self.target),
            animation: self.animation,
        }
    }
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
/// end. Follow-tail UIs read them through [`SignalExt::map`](nami::SignalExt::map) — for example
/// `controller.position().map(|p| p.is_some_and(|p| p.at_end))`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ScrollPosition {
    /// The content offset the viewport's origin sits at, in points.
    offset: Point,
    /// The size of the visible scroll area, in points.
    viewport_size: Size,
    /// The scrollable extent, in points.
    content_size: Size,
    /// Whether the viewport touches the start of every scrollable axis.
    at_start: bool,
    /// Whether the viewport touches the end of every scrollable axis.
    at_end: bool,
}

impl ScrollPosition {
    /// The content offset the viewport's origin sits at, in points.
    #[must_use]
    pub const fn offset(&self) -> Point {
        self.offset
    }

    /// The size of the visible scroll area, in points.
    #[must_use]
    pub const fn viewport_size(&self) -> Size {
        self.viewport_size
    }

    /// The scrollable extent, in points.
    #[must_use]
    pub const fn content_size(&self) -> Size {
        self.content_size
    }

    /// Whether the viewport touches the start of every scrollable axis.
    #[must_use]
    pub const fn at_start(&self) -> bool {
        self.at_start
    }

    /// Whether the viewport touches the end of every scrollable axis.
    #[must_use]
    pub const fn at_end(&self) -> bool {
        self.at_end
    }
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
/// A controller owns the latest [`ScrollRequest`] and a request generation
/// that advances with every request, wrapping on overflow. The generation makes repeated requests to
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
            request: Binding::new(ScrollRequest {
                target: initial_target,
                animation: None,
            }),
            generation: Binding::new(0),
            position: Binding::new(None),
        }
    }

    /// Requests an immediate jump to `target`.
    pub fn scroll_to(&self, target: T) {
        self.submit(target, None);
    }

    /// Requests an animated scroll to `target`, moving as
    /// [`ScrollRequest::animation`] describes. A later request, or the user
    /// scrolling, takes over from an animation still in flight.
    pub fn animate_to(&self, target: T, animation: Animation) {
        self.submit(target, Some(animation));
    }

    fn submit(&self, target: T, animation: Option<Animation>) {
        self.request.set(ScrollRequest { target, animation });
        // The generation is only compared for change, so wrapping is
        // harmless.
        self.generation
            .with_mut(|generation| *generation = generation.wrapping_add(1));
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
    /// Read edge contact through [`SignalExt::map`](nami::SignalExt::map), e.g.
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

    /// Returns a backend-facing [`ScrollDriver`] reading this controller's
    /// requests and generation and reporting into its position sink.
    #[doc(hidden)]
    #[must_use]
    pub fn driver(&self) -> ScrollDriver<T> {
        ScrollDriver {
            request: self.request(),
            generation: self.generation(),
            position: self.position.clone(),
        }
    }

    /// Returns a backend-facing [`ScrollDriver`] whose target is mapped by `map`.
    #[doc(hidden)]
    #[must_use]
    pub fn map_driver<U: Clone + 'static>(
        &self,
        map: impl Fn(T) -> U + 'static,
    ) -> ScrollDriver<U> {
        self.driver().map_target(map)
    }
}

/// Identifies a [`LazyContainer`](crate::LazyContainer) bound to an enclosing
/// [`ScrollView`]'s item-identity [`ScrollController`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScrollCollectionId(usize);

/// Environment entry installed by an identity-controlled [`ScrollView`] so
/// descendant [`LazyContainer`](crate::LazyContainer) instances with matching
/// item identity type `T` register their [`Mapping<T>`] into it.
pub(crate) struct ScrollItemBinder<T>(Rc<RefCell<Vec<Mapping<T>>>>);

impl<T> Clone for ScrollItemBinder<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T: Ord + Clone + 'static> ScrollItemBinder<T> {
    pub(crate) fn new() -> Self {
        Self(Rc::new(RefCell::new(Vec::new())))
    }

    pub(crate) fn bind(&self, mapping: Mapping<T>) -> ScrollCollectionId {
        let mut mappings = self.0.borrow_mut();
        let id = ScrollCollectionId(mappings.len());
        mappings.push(mapping);
        id
    }

    fn id_in(&self, collection: ScrollCollectionId, target: &T) -> Option<Id> {
        self.0
            .borrow()
            .get(collection.0)
            .map(|mapping| mapping.to_id(target.clone()))
    }
}

type ResolveItemId = dyn Fn(ScrollCollectionId) -> Option<Id>;

/// An erased item identity carried inside a [`ScrollTarget::Item`] request.
///
/// Backends query [`Self::id_in`] against each descendant
/// [`LazyContainer`](crate::LazyContainer)'s [`ScrollCollectionId`] to obtain
/// the erased [`Id`] matching `AnyViewsSnapshot::get_id`.
#[derive(Clone)]
pub struct ScrollItemTarget {
    resolve: Rc<ResolveItemId>,
}

impl fmt::Debug for ScrollItemTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScrollItemTarget").finish_non_exhaustive()
    }
}

impl ScrollItemTarget {
    fn new<T: Ord + Clone + 'static>(target: T, binder: ScrollItemBinder<T>) -> Self {
        Self {
            resolve: Rc::new(move |collection| binder.id_in(collection, &target)),
        }
    }

    /// Resolves this request's item identity to the erased [`Id`] used by
    /// `collection`, or `None` if `collection` is not bound to this controller.
    #[must_use]
    pub fn id_in(&self, collection: ScrollCollectionId) -> Option<Id> {
        (self.resolve)(collection)
    }
}

/// What a [`ScrollView`] programmatic scroll request targets.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ScrollTarget {
    /// An explicit content offset in points.
    Point(Point),
    /// An item inside a scrollable collection, identified by its erased identity.
    Item(ScrollItemTarget),
}

/// The backend-facing view of a connected [`ScrollController`].
///
/// Carries the reactive request signal (with any typed item identity erased),
/// the request generation signal, and the position report binding.
#[derive(Clone, Debug)]
pub struct ScrollDriver<T: Clone + 'static = ScrollTarget> {
    request: Computed<ScrollRequest<T>>,
    generation: Computed<i32>,
    position: Binding<Option<ScrollPosition>>,
}

impl<T: Clone + 'static> ScrollDriver<T> {
    /// Returns the latest request as a read-only signal.
    #[must_use]
    pub fn request(&self) -> Computed<ScrollRequest<T>> {
        self.request.clone()
    }

    /// Returns the request generation as a read-only signal.
    #[must_use]
    pub fn generation(&self) -> Computed<i32> {
        self.generation.clone()
    }

    /// The sink a backend writes the measured [`ScrollPosition`] into.
    #[must_use]
    pub fn position_report(&self) -> Binding<Option<ScrollPosition>> {
        self.position.clone()
    }

    /// Maps the request target through `map`, preserving generation and position reporting.
    #[must_use]
    pub fn map_target<U: Clone + 'static>(
        &self,
        map: impl Fn(T) -> U + 'static,
    ) -> ScrollDriver<U> {
        let map = Rc::new(map);
        ScrollDriver {
            request: self
                .request
                .clone()
                .map(move |request| request.map(map.as_ref()))
                .computed(),
            generation: self.generation.clone(),
            position: self.position.clone(),
        }
    }
}

type EnvInstallFn = Rc<dyn Fn(&mut Environment)>;

mod sealed {
    #[derive(Debug)]
    pub struct PointTarget;
    #[derive(Debug)]
    pub struct ItemTarget;
    pub trait Sealed<M> {}
}

/// A [`ScrollController`] accepted by [`ScrollView::scroll_controller`]: either
/// `&ScrollController<Point>` for explicit pixel offsets or `&ScrollController<T>`
/// (`T: Ord + Clone + 'static`) for item-identity scrolling.
pub trait IntoScrollDriver<M>: sealed::Sealed<M> {
    #[doc(hidden)]
    fn into_scroll_driver(self) -> (ScrollDriver<ScrollTarget>, Option<EnvInstallFn>);
}

impl sealed::Sealed<sealed::PointTarget> for &ScrollController<Point> {}

impl IntoScrollDriver<sealed::PointTarget> for &ScrollController<Point> {
    fn into_scroll_driver(self) -> (ScrollDriver<ScrollTarget>, Option<EnvInstallFn>) {
        (self.map_driver(ScrollTarget::Point), None)
    }
}

impl<T: Ord + Clone + 'static> sealed::Sealed<sealed::ItemTarget> for &ScrollController<T> {}

impl<T: Ord + Clone + 'static> IntoScrollDriver<sealed::ItemTarget> for &ScrollController<T> {
    fn into_scroll_driver(self) -> (ScrollDriver<ScrollTarget>, Option<EnvInstallFn>) {
        let binder = ScrollItemBinder::<T>::new();
        let driver = self.map_driver({
            let binder = binder.clone();
            move |target| ScrollTarget::Item(ScrollItemTarget::new(target, binder.clone()))
        });
        let install: EnvInstallFn = Rc::new(move |env: &mut Environment| {
            env.insert(binder.clone());
        });
        (driver, Some(install))
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
///
/// The content keeps its own type, `C`, until the view reaches a backend,
/// which receives it as its child slot and realizes it in place.
pub struct ScrollView<C> {
    axis: Axis,
    content: C,
    controller: Option<ScrollDriver<ScrollTarget>>,
    install_mapping: Option<EnvInstallFn>,
    offset: Option<Binding<Point>>,
}

impl<C: fmt::Debug> fmt::Debug for ScrollView<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScrollView")
            .field("axis", &self.axis)
            .field("content", &self.content)
            .field("controller", &self.controller)
            .field("offset", &self.offset)
            .finish_non_exhaustive()
    }
}

/// The parts a [`ScrollView`] is made of, as [`ScrollView::into_inner`]
/// hands them to a backend.
#[derive(Debug)]
#[non_exhaustive]
pub struct ScrollViewParts<C> {
    /// The axis or axes the view scrolls along.
    axis: Axis,
    /// The scrolled content.
    content: C,
    /// The programmatic scroll controller driver, if one is connected.
    controller: Option<ScrollDriver<ScrollTarget>>,
    /// The binding the backend reports the content offset into, if one is
    /// connected.
    offset: Option<Binding<Point>>,
}

impl<C> ScrollViewParts<C> {
    /// Takes the value apart into its fields, in declaration order.
    pub fn into_parts(
        self,
    ) -> (
        Axis,
        C,
        Option<ScrollDriver<ScrollTarget>>,
        Option<Binding<Point>>,
    ) {
        (self.axis, self.content, self.controller, self.offset)
    }

    /// Creates a `ScrollViewParts` from its parts.
    #[must_use]
    pub const fn new(
        axis: Axis,
        content: C,
        controller: Option<ScrollDriver<ScrollTarget>>,
        offset: Option<Binding<Point>>,
    ) -> Self {
        Self {
            axis,
            content,
            controller,
            offset,
        }
    }

    /// The axis or axes the view scrolls along.
    #[must_use]
    pub const fn axis(&self) -> Axis {
        self.axis
    }

    /// The scrolled content.
    #[must_use]
    pub const fn content(&self) -> &C {
        &self.content
    }

    /// The programmatic scroll controller driver, if one is connected.
    #[must_use]
    pub const fn controller(&self) -> Option<&ScrollDriver<ScrollTarget>> {
        self.controller.as_ref()
    }

    /// The binding the backend reports the content offset into, if one is
    /// connected.
    #[must_use]
    pub const fn offset(&self) -> Option<&Binding<Point>> {
        self.offset.as_ref()
    }
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

impl Axis {
    /// Returns the axes shared by `self` and `other`, or `None` when they are
    /// orthogonal.
    #[must_use]
    pub const fn intersect(self, other: Self) -> Option<Self> {
        match (self, other) {
            (Self::All, axis) | (axis, Self::All) => Some(axis),
            (Self::Horizontal, Self::Horizontal) => Some(Self::Horizontal),
            (Self::Vertical, Self::Vertical) => Some(Self::Vertical),
            (Self::Horizontal, Self::Vertical) | (Self::Vertical, Self::Horizontal) => None,
        }
    }

    /// Whether this axis includes vertical scrolling.
    #[must_use]
    pub const fn allows_vertical(self) -> bool {
        matches!(self, Self::Vertical | Self::All)
    }

    /// Whether this axis includes horizontal scrolling.
    #[must_use]
    pub const fn allows_horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::All)
    }
}

impl<C: View> ScrollView<C> {
    /// Creates a new `ScrollView` with the specified scroll axis and content.
    #[must_use]
    pub const fn new(axis: Axis, content: C) -> Self {
        Self {
            axis,
            content,
            controller: None,
            install_mapping: None,
            offset: None,
        }
    }
}

impl<C> ScrollView<C> {
    /// Decomposes the `ScrollView` into its parts.
    #[must_use]
    pub fn into_inner(self) -> ScrollViewParts<C> {
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
    pub const fn content(&self) -> &C {
        &self.content
    }

    /// The same view with its content mapped through `f`, preserving any
    /// attached controller and offset bindings.
    fn map_content<D>(self, f: impl FnOnce(C) -> D) -> ScrollView<D> {
        ScrollView {
            axis: self.axis,
            content: f(self.content),
            controller: self.controller,
            install_mapping: self.install_mapping,
            offset: self.offset,
        }
    }

    /// Connects a programmatic scroll controller: either `&ScrollController<Point>`
    /// for pixel-offset scrolling or `&ScrollController<Id>` for item-identity
    /// scrolling over a descendant `ForEach` / lazy stack.
    #[must_use]
    pub fn scroll_controller<M>(mut self, controller: impl IntoScrollDriver<M>) -> Self {
        let (driver, install_mapping) = controller.into_scroll_driver();
        self.controller = Some(driver);
        self.install_mapping = install_mapping;
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

/// A scroll's axis follows its realized content's (§6), so backends answer
/// it from the realized node through [`scroll_stretch_axis`].
impl<C: 'static> NativeView for ScrollView<C> {}

struct ScrollSubtreeEnv<C> {
    content: C,
    axis: Axis,
    install_mapping: Option<EnvInstallFn>,
}

impl<C: View> View for ScrollSubtreeEnv<C> {
    fn body(self, env: &Environment) -> impl View {
        let mut child_env = env.extending(self.axis);
        if let Some(install) = self.install_mapping {
            install(&mut child_env);
        }
        Metadata::new(self.content, child_env)
    }
}

impl<C: View> View for ScrollView<C> {
    /// The erased form, for code that expands views by hand.
    fn body(self, _env: &Environment) -> impl View {
        let Self {
            axis,
            content,
            controller,
            install_mapping,
            offset,
        } = self;
        Native::new(ScrollView {
            axis,
            content: AnyView::new(ScrollSubtreeEnv {
                content,
                axis,
                install_mapping,
            }),
            controller,
            install_mapping: None,
            offset,
        })
    }

    /// Reaches the backend with the content in the backend's child slot,
    /// publishing this scroll view's nearest scrolling [`Axis`] and any item
    /// scroll binder into the subtree's [`Environment`].
    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        let Self {
            axis,
            content,
            controller,
            install_mapping,
            offset,
        } = self;
        let wrapped = ScrollSubtreeEnv {
            content,
            axis,
            install_mapping,
        };
        #[cfg(feature = "erased-dispatch")]
        let slot = B::Slot::from_view(AnyView::new(wrapped));
        #[cfg(not(feature = "erased-dispatch"))]
        let slot = B::Slot::from_view(wrapped);
        backend.native(
            ScrollView {
                axis,
                content: slot,
                controller,
                install_mapping: None,
                offset,
            },
            env,
            cx,
        )
    }
}

impl<C> MapSlots<C> for ScrollView<C> {
    type With<T: 'static> = ScrollView<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(C) -> T + Clone + 'static) -> ScrollView<T> {
        self.map_content(f)
    }
}

/// Creates a vertical `ScrollView` with the given content.
///
/// This is the most common scroll direction for lists and long content.
/// The actual scrolling behavior is implemented by the renderer backend.
pub const fn scroll<C: View>(content: C) -> ScrollView<C> {
    ScrollView::new(Axis::Vertical, content)
}

/// Creates a horizontal `ScrollView` with the given content.
///
/// Useful for wide content that needs to scroll left-right.
/// The actual scrolling behavior is implemented by the renderer backend.
pub const fn scroll_horizontal<C: View>(content: C) -> ScrollView<C> {
    ScrollView::new(Axis::Horizontal, content)
}

/// Creates a `ScrollView` that can scroll in both directions.
///
/// Useful for large content like images or tables that may need both horizontal and vertical scrolling.
/// The actual scrolling behavior is implemented by the renderer backend.
pub const fn scroll_both<C: View>(content: C) -> ScrollView<C> {
    ScrollView::new(Axis::All, content)
}

#[cfg(test)]
mod tests {
    use super::{
        Axis, ScrollController, ScrollPosition, ScrollRequest, ScrollView, scroll_stretch_axis,
    };
    use crate::{Point, Size, StretchAxis};
    use core::time::Duration;
    use nami::Signal;
    use waterui_core::{Environment, View, animation::Animation};

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

    #[test]
    fn constructor_accepts_concrete_view_without_type_erasure() {
        #[derive(Debug, PartialEq)]
        struct Leaf;
        impl View for Leaf {
            fn body(self, _env: &Environment) -> impl View {}
        }

        let view = ScrollView::new(Axis::Horizontal, Leaf);
        assert_eq!(view.axis(), Axis::Horizontal);
        assert_eq!(view.content(), &Leaf);
    }

    #[test]
    fn item_scroll_controller_binds_into_descendant_lazy_container() {
        use super::{ScrollCollectionId, ScrollTarget, scroll};
        use crate::container::LazyContainer;
        use crate::stack::VStackLayout;
        use alloc::vec;
        use waterui_core::{AnyView, Native, id::Identifiable, views::ForEach};

        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
        struct MessageId(u64);

        #[derive(Clone)]
        struct Message {
            id: MessageId,
        }

        impl Identifiable for Message {
            type Id = MessageId;

            fn id(&self) -> Self::Id {
                self.id
            }
        }

        let controller = ScrollController::new(MessageId(0));
        let messages = vec![
            Message { id: MessageId(10) },
            Message { id: MessageId(42) },
            Message { id: MessageId(99) },
        ];
        let view = scroll(LazyContainer::new(
            VStackLayout::default(),
            ForEach::new(messages, |_| ()),
        ))
        .scroll_controller(&controller);
        let install_mapping = view
            .install_mapping
            .clone()
            .expect("item scroll controller installs a binder");
        let parts = view.into_inner();
        let driver = parts
            .controller
            .expect("item scroll controller attaches a driver");

        let mut env = Environment::new();
        env.insert(parts.axis);
        install_mapping(&mut env);

        let lazy = AnyView::new(View::body(parts.content, &env))
            .downcast::<Native<LazyContainer<AnyView>>>()
            .expect("LazyContainer body resolves to Native<LazyContainer<AnyView>>")
            .into_inner();
        let collection = lazy
            .scroll_collection()
            .expect("matching LazyContainer binds to enclosing ScrollController<MessageId>");

        // Item 1 (MessageId(42)) maps cleanly to its collection Id.
        let (_, contents) = lazy.into_inner();
        let expected_id = contents.snapshot().get_id(1).expect("item 1 exists");
        controller.scroll_to(MessageId(42));
        match driver.request().snapshot().target() {
            ScrollTarget::Item(item) => {
                assert_eq!(item.id_in(collection), Some(expected_id));
            }
            other => panic!("expected ScrollTarget::Item, got {other:?}"),
        }

        // An identity not yet present maps to a fresh Id distinct from existing items
        // (so a later push of that identity matches the pending request's Id), while
        // an unrelated collection key returns None.
        controller.scroll_to(MessageId(999));
        match driver.request().snapshot().target() {
            ScrollTarget::Item(item) => {
                let future_id = item
                    .id_in(collection)
                    .expect("bound collection maps future identities");
                let snapshot = contents.snapshot();
                assert!(
                    !(0..3).any(|i| snapshot.get_id(i).expect("item exists") == future_id),
                    "unseen identity must allocate a distinct id"
                );
                assert_eq!(item.id_in(ScrollCollectionId(999)), None);
            }
            other => panic!("expected ScrollTarget::Item, got {other:?}"),
        }
    }
}
