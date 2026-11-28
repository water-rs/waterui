//! Anchored overlay metadata.

use crate::slot::SlotBuilder;
use nami::Binding;
use waterui_core::{
    AnyView, Dismiss, Environment, Metadata, View,
    backend::{Backend, ChildSlot, MapSlots},
    handler::ViewBuilder,
    metadata::MetadataKey,
};

/// A view presented next to the view it is attached to (its anchor),
/// above all other content in the window.
///
/// The backend places the overlay: it knows where the anchor sits in the
/// window and how large the window is, which a layout inside the anchor
/// cannot see. It puts the overlay against the preferred
/// [`edge`](AnchorPlacement::edge), moves it to the opposite edge when
/// the preferred one has no room and [`flip`](AnchorPlacement::flip) is
/// set, then keeps it inside the window as [`clamp`](AnchorPlacement::clamp)
/// asks. Tooltips, popovers and dropdowns are built on it.
///
/// The edge the backend actually used is written to
/// [`placed_edge`](Self::placed_edge) on every placement, so content can
/// grow out of, or point at, the anchor from the right side after a flip.
///
/// When `is_presented` becomes `false`, the content leaves the semantic
/// tree immediately. Its removal transition may retain a visual ghost
/// until completion. Every new presentation calls the content builder,
/// including a presentation started while an older ghost is exiting.
/// State shared between presentations belongs outside the builder and is
/// passed into each fresh view explicitly.
///
/// # Example
///
/// ```rust
/// use waterui::metadata::anchored_overlay::{AnchorEdge, AnchoredOverlay, Clamp};
/// use waterui::prelude::*;
///
/// let shown = binding(false);
/// let icon = text!("?").anchored_overlay(
///     AnchoredOverlay::new(&shown, move || text!("Opens the help page"))
///         .edge(AnchorEdge::Top)
///         .gap(4.0)
///         .clamp(Clamp::Window { margin: 2.0 }),
/// );
/// ```
#[derive(Debug)]
pub struct AnchoredOverlay<S = AnyView> {
    /// Builds a fresh view for each presentation next to the anchor.
    pub content: SlotBuilder<S>,
    /// Whether the overlay is presented. The backend writes `false` when
    /// it dismisses the overlay itself, as [`dismissal`](Self::dismissal)
    /// allows.
    pub is_presented: Binding<bool>,
    /// Where the overlay sits relative to the anchor.
    pub placement: AnchorPlacement,
    /// What besides the binding closes the overlay.
    pub dismissal: Dismissal,
    /// The edge the overlay was placed against after any flip. The backend
    /// writes it on every placement; until the first placement it holds
    /// [`placement.edge`](AnchorPlacement::edge) as it was when
    /// [`new`](Self::new) ran, or the value a binding passed to
    /// [`placed_edge`](Self::placed_edge) already held. `Leading` and
    /// `Trailing` keep their layout-direction meaning.
    pub placed_edge: Binding<AnchorEdge>,
}

impl<S: 'static> MetadataKey for AnchoredOverlay<S> {}

impl<V: View> AnchoredOverlay<V> {
    /// An overlay presenting the view `content` builds while
    /// `is_presented` is `true`.
    ///
    /// It sits below the anchor, centered, with no gap, flips when there
    /// is no room below, stays inside the window, and closes when the
    /// user interacts outside it.
    #[must_use]
    pub fn new(is_presented: &Binding<bool>, content: impl ViewBuilder<Output = V>) -> Self {
        Self {
            content: SlotBuilder::new(move || content.build()),
            is_presented: is_presented.clone(),
            placement: AnchorPlacement::default(),
            dismissal: Dismissal::OutsideInteraction,
            placed_edge: nami::binding(AnchorPlacement::default().edge),
        }
    }
}

impl<S> AnchoredOverlay<S> {
    /// Places the overlay against `edge` of the anchor.
    #[must_use]
    pub const fn edge(mut self, edge: AnchorEdge) -> Self {
        self.placement.edge = edge;
        self
    }

    /// Lines the overlay up with the anchor along its edge.
    #[must_use]
    pub const fn alignment(mut self, alignment: EdgeAlignment) -> Self {
        self.placement.alignment = alignment;
        self
    }

    /// Leaves `gap` points between the anchor and the overlay.
    #[must_use]
    pub const fn gap(mut self, gap: f32) -> Self {
        self.placement.gap = gap;
        self
    }

    /// Whether the overlay moves to the opposite edge when the preferred
    /// edge has no room for it.
    #[must_use]
    pub const fn flip(mut self, flip: bool) -> Self {
        self.placement.flip = flip;
        self
    }

    /// How the overlay is kept inside the window.
    #[must_use]
    pub const fn clamp(mut self, clamp: Clamp) -> Self {
        self.placement.clamp = clamp;
        self
    }

    /// What besides the binding closes the overlay.
    #[must_use]
    pub const fn dismissal(mut self, dismissal: Dismissal) -> Self {
        self.dismissal = dismissal;
        self
    }

    /// Reports the edge the overlay was placed against, after any flip,
    /// into `placed_edge`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::style::Anchor;
    /// use waterui::metadata::anchored_overlay::{AnchorEdge, AnchoredOverlay};
    /// use waterui::prelude::*;
    ///
    /// let shown = binding(false);
    /// let placed = binding(AnchorEdge::Top);
    /// // A tooltip that grows out of the anchor: from its bottom edge when
    /// // it sits above the anchor, from its top edge after a flip below.
    /// let origin = placed.map(|edge| match edge {
    ///     AnchorEdge::Bottom => Anchor::new(0.5, 0.0),
    ///     _ => Anchor::new(0.5, 1.0),
    /// });
    /// text!("?").anchored_overlay(
    ///     AnchoredOverlay::new(&shown, move || text!("Help"))
    ///         .edge(AnchorEdge::Top)
    ///         .placed_edge(&placed),
    /// );
    /// # let _ = origin;
    /// ```
    #[must_use]
    pub fn placed_edge(mut self, placed_edge: &Binding<AnchorEdge>) -> Self {
        self.placed_edge = placed_edge.clone();
        self
    }

    /// Returns the [`Dismiss`] environment handle bound to this overlay.
    #[doc(hidden)]
    #[must_use]
    pub fn dismiss_handle(&self) -> Dismiss {
        let is_presented = self.is_presented.clone();
        Dismiss::new(move || is_presented.set(false))
    }
}

impl<S: 'static> AnchoredOverlay<S> {
    /// Maps the overlay's built content slot through `f`.
    #[must_use]
    pub fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> AnchoredOverlay<T> {
        AnchoredOverlay {
            content: self.content.map_slot(f),
            is_presented: self.is_presented,
            placement: self.placement,
            dismissal: self.dismissal,
            placed_edge: self.placed_edge,
        }
    }
}

impl<S: 'static> MapSlots<S> for AnchoredOverlay<S> {
    type With<T: 'static> = AnchoredOverlay<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> AnchoredOverlay<T> {
        Self::map_slots(self, f)
    }
}

/// View wrapper that attaches an [`AnchoredOverlay`] with its overlay
/// content projected into the backend's child slot without `AnyView`
/// erasure.
#[doc(hidden)]
#[derive(Debug)]
pub struct AnchoredOverlayView<Content, V> {
    content: Content,
    overlay: AnchoredOverlay<V>,
}

impl<Content, V> AnchoredOverlayView<Content, V> {
    /// Creates a wrapper attaching `overlay` to `content`.
    #[must_use]
    pub const fn new(content: Content, overlay: AnchoredOverlay<V>) -> Self {
        Self { content, overlay }
    }
}

impl<Content: View, V: View> View for AnchoredOverlayView<Content, V> {
    fn body(self, _env: &Environment) -> impl View {
        Metadata::new(self.content, self.overlay.map_slots(AnyView::new))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.metadata(
            Metadata::new(
                B::Slot::from_view(self.content),
                self.overlay.map_slots(B::Slot::from_view),
            ),
            env,
            cx,
        )
    }
}

/// Where an [`AnchoredOverlay`] sits relative to its anchor.
///
/// # Example
///
/// ```rust
/// use waterui::metadata::anchored_overlay::{AnchorEdge, AnchorPlacement, Clamp, EdgeAlignment};
///
/// // A tooltip placement: above the anchor, centered, a small gap, and
/// // kept two points inside the window.
/// let placement = AnchorPlacement::default()
///     .edge(AnchorEdge::Top)
///     .alignment(EdgeAlignment::Center)
///     .gap(4.0)
///     .flip(true)
///     .clamp(Clamp::Window { margin: 2.0 });
/// assert_eq!(placement.get_edge(), AnchorEdge::Top);
/// ```
///
/// The defaults describe a dropdown: below the anchor, centered, no gap,
/// flipping when there is no room, kept inside the window.
///
/// ```rust
/// use waterui::metadata::anchored_overlay::{AnchorEdge, AnchorPlacement, Clamp, EdgeAlignment};
///
/// let default = AnchorPlacement::default();
/// assert_eq!(default.get_edge(), AnchorEdge::Bottom);
/// assert_eq!(default.get_alignment(), EdgeAlignment::Center);
/// assert_eq!(default.get_gap(), 0.0);
/// assert!(default.get_flip());
/// assert_eq!(default.get_clamp(), Clamp::Window { margin: 0.0 });
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct AnchorPlacement {
    /// The edge of the anchor the overlay is placed against.
    edge: AnchorEdge,
    /// How the overlay lines up with the anchor along that edge.
    alignment: EdgeAlignment,
    /// The distance between the anchor and the overlay, in points.
    gap: f32,
    /// Whether the overlay moves to the opposite edge when the preferred
    /// edge has no room for it.
    flip: bool,
    /// How the overlay is kept inside the window.
    clamp: Clamp,
}

impl AnchorPlacement {
    /// The edge of the anchor the overlay is placed against.
    #[must_use]
    pub const fn get_edge(&self) -> AnchorEdge {
        self.edge
    }

    /// How the overlay lines up with the anchor along that edge.
    #[must_use]
    pub const fn get_alignment(&self) -> EdgeAlignment {
        self.alignment
    }

    /// The distance between the anchor and the overlay, in points.
    #[must_use]
    pub const fn get_gap(&self) -> f32 {
        self.gap
    }

    /// Whether the overlay moves to the opposite edge when the preferred
    /// edge has no room for it.
    #[must_use]
    pub const fn get_flip(&self) -> bool {
        self.flip
    }

    /// How the overlay is kept inside the window.
    #[must_use]
    pub const fn get_clamp(&self) -> Clamp {
        self.clamp
    }
}

impl Default for AnchorPlacement {
    fn default() -> Self {
        Self {
            edge: AnchorEdge::Bottom,
            alignment: EdgeAlignment::Center,
            gap: 0.0,
            flip: true,
            clamp: Clamp::Window { margin: 0.0 },
        }
    }
}

impl AnchorPlacement {
    /// Places the overlay against `edge` of the anchor.
    #[must_use]
    pub const fn edge(mut self, edge: AnchorEdge) -> Self {
        self.edge = edge;
        self
    }

    /// Lines the overlay up with the anchor by `alignment` along its edge.
    #[must_use]
    pub const fn alignment(mut self, alignment: EdgeAlignment) -> Self {
        self.alignment = alignment;
        self
    }

    /// Leaves `gap` points between the anchor and the overlay.
    #[must_use]
    pub const fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// Whether the overlay moves to the opposite edge when the preferred
    /// edge has no room for it.
    #[must_use]
    pub const fn flip(mut self, flip: bool) -> Self {
        self.flip = flip;
        self
    }

    /// Keeps the overlay inside the window by `clamp`.
    #[must_use]
    pub const fn clamp(mut self, clamp: Clamp) -> Self {
        self.clamp = clamp;
        self
    }
}

/// An edge of the anchor. `Leading` and `Trailing` follow the layout
/// direction.
///
/// # Example
///
/// ```rust
/// use waterui::metadata::anchored_overlay::AnchorEdge;
///
/// // `opposite` is the edge a `flip` moves the overlay to.
/// assert_eq!(AnchorEdge::Top.opposite(), AnchorEdge::Bottom);
/// assert_eq!(AnchorEdge::Leading.opposite(), AnchorEdge::Trailing);
/// ```
///
/// In a right-to-left layout `Leading` is the anchor's right edge and
/// `Trailing` its left; `Top` and `Bottom` never swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed set of the four logical anchor edges, which the overlay placement resolves variant by variant"
)]
pub enum AnchorEdge {
    /// Above the anchor.
    Top,
    /// Below the anchor.
    Bottom,
    /// Before the anchor in the layout direction.
    Leading,
    /// After the anchor in the layout direction.
    Trailing,
}

impl AnchorEdge {
    /// The edge across the anchor from this one.
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
            Self::Leading => Self::Trailing,
            Self::Trailing => Self::Leading,
        }
    }
}

/// How an overlay lines up with its anchor along the edge it sits
/// against. Along the top and bottom edges, start is the leading side;
/// along the leading and trailing edges, start is the top.
///
/// # Example
///
/// ```rust
/// use waterui::metadata::anchored_overlay::{AnchorEdge, AnchoredOverlay, EdgeAlignment};
/// use waterui::prelude::*;
///
/// let shown = binding(false);
/// // A tooltip whose trailing end lines up with the anchor's trailing end.
/// text!("i").anchored_overlay(
///     AnchoredOverlay::new(&shown, move || text!("Storage used: 80%"))
///         .edge(AnchorEdge::Top)
///         .alignment(EdgeAlignment::End),
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed: an overlay lines up with the start, the center, or the end of its anchor edge"
)]
pub enum EdgeAlignment {
    /// The overlay's start side lines up with the anchor's start side.
    Start,
    /// The overlay is centered on the anchor.
    Center,
    /// The overlay's end side lines up with the anchor's end side.
    End,
}

/// How an overlay is kept inside the window.
///
/// # Example
///
/// ```rust
/// use waterui::metadata::anchored_overlay::{AnchoredOverlay, Clamp};
/// use waterui::prelude::*;
///
/// let shown = binding(false);
/// // A margin keeps the overlay off the window's edges; `Off` lets a
/// // deliberately oversized overlay extend past them.
/// text!("anchor").anchored_overlay(
///     AnchoredOverlay::new(&shown, move || text!("overlay"))
///         .clamp(Clamp::Window { margin: 8.0 }),
/// );
/// ```
///
/// An overlay larger than the window minus the margins is pinned to the
/// leading/top margin instead of overflowing on both ends.
#[derive(Debug, Clone, Copy, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "the overlay placement realizes each clamping policy variant by variant; a new policy must fail its build instead of falling into a wildcard arm"
)]
pub enum Clamp {
    /// The overlay may extend past the window's edges.
    Off,
    /// The overlay is shifted to stay at least `margin` points inside
    /// the window's edges.
    Window {
        /// The minimum distance from the window's edges, in points.
        margin: f32,
    },
}

/// What besides its binding closes an overlay.
///
/// # Example
///
/// ```rust
/// use waterui::metadata::anchored_overlay::{AnchoredOverlay, Dismissal};
/// use waterui::prelude::*;
///
/// let shown = binding(false);
/// // `Manual` is for overlays with their own close affordance: outside
/// // interaction goes to the content underneath without dismissing.
/// button("anchor").action({ let shown = shown.clone(); move || shown.set(true) })
///     .anchored_overlay(
///         AnchoredOverlay::new(&shown, move || text!("overlay"))
///             .dismissal(Dismissal::Manual),
///     );
/// ```
///
/// With [`OutsideInteraction`](Dismissal::OutsideInteraction) a pointer or
/// touch down outside the overlay writes `false` to its binding; the same
/// event still reaches its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Dismissal {
    /// Only setting the binding to `false` closes the overlay.
    Manual,
    /// The backend also closes the overlay, writing `false` to its
    /// binding, when the user interacts outside it.
    OutsideInteraction,
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use core::cell::Cell;

    use nami::{Binding, Signal, binding};
    use waterui_core::{Environment, View};

    use super::AnchoredOverlay;

    /// A view that records which build produced it and writes through a
    /// caller-owned binding.
    #[derive(Debug)]
    struct ProbeView {
        build: usize,
        shared: Binding<i32>,
    }

    impl View for ProbeView {
        fn body(self, _env: &Environment) -> impl View {}
    }

    /// Each `content.build()` yields an independent view while bindings
    /// the caller owns stay shared between the views it built.
    #[test]
    fn content_builder_builds_independent_views_sharing_state() {
        let presented = binding(false);
        let shared = binding(0_i32);
        let builds = Rc::new(Cell::new(0_usize));

        let overlay = AnchoredOverlay::new(&presented, {
            let builds = Rc::clone(&builds);
            move || {
                builds.set(builds.get() + 1);
                ProbeView {
                    build: builds.get(),
                    shared: shared.clone(),
                }
            }
        });

        let first = overlay.content.build();
        let second = overlay.content.build();

        assert_eq!(builds.get(), 2);
        assert_ne!(first.build, second.build);

        first.shared.set(7);
        assert_eq!(second.shared.snapshot(), 7);
    }

    #[test]
    fn anchored_overlay_view_dispatches_typed_slot_without_anyview() {
        use core::any::TypeId;

        use waterui_core::backend::{Backend, ChildSlot, cast};
        use waterui_core::metadata::MetadataKey;
        use waterui_core::view::hook::{HookList, HookableView};
        use waterui_core::{AnyView, Metadata, NativeView};

        use super::AnchoredOverlayView;

        struct RecordedSlot(TypeId);

        impl ChildSlot for RecordedSlot {
            fn from_view<V: View>(_view: V) -> Self {
                Self(TypeId::of::<V>())
            }
        }

        struct CapturingBackend {
            seen: Option<TypeId>,
        }

        impl Backend for CapturingBackend {
            type Output = ();
            type Ctx = ();
            type Slot = RecordedSlot;

            fn native<T: NativeView + 'static>(&mut self, _native: T, _env: &Environment, (): ()) {}

            fn metadata<K: MetadataKey>(
                &mut self,
                metadata: Metadata<K, Self::Slot>,
                _env: &Environment,
                (): (),
            ) {
                if let Ok(overlay) = cast::<K, AnchoredOverlay<RecordedSlot>>(metadata.value) {
                    self.seen = Some(overlay.content.build().0);
                }
            }

            fn erased(&mut self, _view: AnyView, _env: &Environment, (): ()) {
                panic!("AnchoredOverlayView must dispatch statically without AnyView");
            }

            fn scoped<H: HookList, V: View>(
                &mut self,
                _hooks: H,
                view: V,
                env: &Environment,
                (): (),
            ) {
                view.dispatch(self, env, ());
            }

            fn hookable<T: HookableView>(&mut self, view: T, env: &Environment, (): ()) {
                self.native(view, env, ());
            }
        }

        let presented = binding(false);
        let shared = binding(0_i32);
        let view = AnchoredOverlayView::new(
            (),
            AnchoredOverlay::new(&presented, move || ProbeView {
                build: 1,
                shared: shared.clone(),
            }),
        );

        let mut backend = CapturingBackend { seen: None };
        view.dispatch(&mut backend, &Environment::new(), ());
        assert_eq!(backend.seen, Some(TypeId::of::<ProbeView>()));
    }
}
