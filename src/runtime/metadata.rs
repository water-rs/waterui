//! Metadata definitions for `WaterUI`.
//!
//! `Metadata`s are some extra information that can be attached to `View`s to modify their behavior
//! or appearance, but not affect their layout.
//!
//! They are defined as types that implement the `MetadataKey` trait.

/// The metadata mechanism itself: the wrappers that attach a value to a view
/// and the marker trait that makes a type attachable. Re-exported here so the
/// mechanism and the concrete keys below live at one path.
pub use waterui_core::metadata::{IgnorableMetadata, Metadata, MetadataKey, Retain};

/// Context menu metadata module.
pub mod context_menu {
    use alloc::vec::Vec;
    use nami::{Binding, Computed, SignalExt, binding};
    use waterui_controls::menu::{MenuItem, MenuView, ResolvedMenuItem, resolve_menu_items};
    use waterui_core::{AnyView, Environment, Metadata, View, env::with, metadata::MetadataKey};

    /// A context menu with an optional custom preview and an interactive
    /// accessory presented alongside its commands.
    ///
    /// The menu appears when the user long-presses (iOS, Android, touch and
    /// pen elsewhere) or secondary-clicks (macOS, desktop pointers). Menu rows
    /// stay within what every platform's own menu can render; custom content
    /// belongs in the [`accessory`](Self::accessory). Placement is
    /// platform-specific: macOS presents the accessory as a native
    /// `NSMenuItem` view inside the menu. On iOS, menus with an accessory
    /// present the custom preview, accessory and commands together in a
    /// native popover; menus without an accessory use the system context
    /// menu and lifted preview.
    ///
    /// The two placements are not interchangeable capabilities. In a
    /// tracked `NSMenu` the accessory is pointer-interactive only — text
    /// entry and other keyboard input are unsupported — its row
    /// dimensions are fixed while the menu tracks, and a changed
    /// intrinsic size is applied on the next open. `Binding`-driven
    /// updates inside the accessory still render in place while it is
    /// open. On iOS, the accessory remains interactive and its intrinsic
    /// size can change while the popover is open.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// let message = text!("Hello")
    ///     .context_menu(
    ///         ContextMenu::new((
    ///             "Reply".action(|| {}),
    ///             "Delete".command().action(|| {}).destructive(),
    ///         ))
    ///         .accessory(
    ///             button("Like").action(|Use(dismiss): Use<DismissContextMenu>| dismiss.dismiss()),
    ///         ),
    ///     );
    /// ```
    #[derive(Debug)]
    #[non_exhaustive]
    pub struct ContextMenu {
        /// The menu items to display in the context menu.
        items: Computed<Vec<MenuItem>>,
        /// The custom preview shown while the menu is open. `None` uses
        /// the platform's default presentation of the source view.
        preview: Option<AnyView>,
        /// An interactive view presented alongside the menu — a
        /// pointer-interactive `NSMenuItem` view row on macOS, alongside
        /// the preview and commands in a native popover on iOS.
        accessory: Option<AnyView>,
    }

    impl ContextMenu {
        /// The menu items to display in the context menu.
        #[must_use]
        pub const fn items(&self) -> &Computed<Vec<MenuItem>> {
            &self.items
        }

        /// The custom preview shown while the menu is open. `None` uses
        /// the platform's default presentation of the source view.
        #[must_use]
        pub const fn get_preview(&self) -> Option<&AnyView> {
            self.preview.as_ref()
        }

        /// An interactive view presented alongside the menu — a
        /// pointer-interactive `NSMenuItem` view row on macOS, alongside
        /// the preview and commands in a native popover on iOS.
        #[must_use]
        pub const fn get_accessory(&self) -> Option<&AnyView> {
            self.accessory.as_ref()
        }
    }

    impl MetadataKey for ContextMenu {}

    impl ContextMenu {
        /// Creates a context menu with the given items, lifting the source
        /// view and showing no accessory.
        #[must_use]
        pub fn new(items: impl MenuView) -> Self {
            Self {
                items: items.into_menu_items(),
                preview: None,
                accessory: None,
            }
        }

        /// Shows `preview` while the menu is open, using the platform's
        /// presentation: a lifted preview for an iOS system context menu,
        /// or content inside its accessory-bearing popover.
        #[must_use]
        pub fn preview(mut self, preview: impl View) -> Self {
            self.preview = Some(AnyView::new(preview));
            self
        }

        /// Presents an interactive `accessory` alongside the menu — as a
        /// native row inside the `NSMenu` on macOS, alongside the preview
        /// and commands in a native popover on iOS.
        ///
        /// Inside a tracked `NSMenu` the accessory is pointer-interactive
        /// only: keyboard input is unsupported, and its dimensions are
        /// fixed until the menu reopens.
        ///
        /// Choosing a menu item dismisses the menu; acting inside the
        /// accessory does not. The accessory's environment carries a
        /// [`DismissContextMenu`] to call when an action should close the
        /// menu.
        #[must_use]
        pub fn accessory(mut self, accessory: impl View) -> Self {
            self.accessory = Some(AnyView::new(accessory));
            self
        }
    }

    /// Converts a value into a [`ContextMenu`]: either a full `ContextMenu`
    /// or any [`MenuView`], which becomes a menu with no preview or
    /// accessory.
    pub trait IntoContextMenu {
        /// Performs the conversion.
        fn into_context_menu(self) -> ContextMenu;
    }

    impl IntoContextMenu for ContextMenu {
        fn into_context_menu(self) -> ContextMenu {
            self
        }
    }

    impl<T: MenuView> IntoContextMenu for T {
        fn into_context_menu(self) -> ContextMenu {
            ContextMenu::new(self)
        }
    }

    /// Closes the context menu whose accessory is being acted on.
    ///
    /// Present in the environment of a context menu's
    /// [`accessory`](ContextMenu::accessory). Extract it in a handler with
    /// `Use<DismissContextMenu>`.
    #[derive(Debug, Clone)]
    pub struct DismissContextMenu {
        requests: Binding<i32>,
    }

    impl DismissContextMenu {
        /// Asks the backend to close the context menu.
        pub fn dismiss(&self) {
            self.requests
                .with_mut(|requests| *requests = requests.wrapping_add(1));
        }
    }

    /// Resolved context menu metadata consumed by the native backends.
    #[doc(hidden)]
    #[derive(Debug)]
    pub struct ResolvedContextMenu {
        /// The resolved menu items for the current environment.
        pub items: Computed<Vec<ResolvedMenuItem>>,
        /// The custom preview shown while the menu is open; `None` uses
        /// the platform's default presentation of the source view.
        pub preview: Option<AnyView>,
        /// The accessory presented alongside the menu, with
        /// [`DismissContextMenu`] installed in its environment.
        pub accessory: Option<AnyView>,
        /// Counts the accessory's dismiss requests, wrapping on overflow.
        /// Only a change matters: every change asks the backend to close the
        /// open menu.
        pub dismiss_requests: Computed<i32>,
    }

    impl MetadataKey for ResolvedContextMenu {}

    /// View wrapper that resolves a context menu against the environment.
    #[doc(hidden)]
    #[derive(Debug)]
    #[non_exhaustive]
    pub struct ContextMenuView<Content> {
        /// The wrapped content view.
        content: Content,
        /// The context menu to resolve.
        menu: ContextMenu,
    }

    impl<Content> ContextMenuView<Content> {
        /// Wraps `content` with the `menu` to resolve against the environment.
        pub const fn new(content: Content, menu: ContextMenu) -> Self {
            Self { content, menu }
        }

        /// The wrapped content view.
        #[must_use]
        pub const fn content(&self) -> &Content {
            &self.content
        }

        /// The context menu to resolve.
        #[must_use]
        pub const fn menu(&self) -> &ContextMenu {
            &self.menu
        }
    }

    impl<Content: View> View for ContextMenuView<Content> {
        fn body(self, env: &Environment) -> impl View {
            let ContextMenu {
                items,
                preview,
                accessory,
            } = self.menu;
            let requests = binding(0_i32);
            let dismiss = DismissContextMenu {
                requests: requests.clone(),
            };
            Metadata::new(
                self.content,
                ResolvedContextMenu {
                    items: resolve_menu_items(&items, env),
                    preview,
                    accessory: accessory.map(|accessory| AnyView::new(with(accessory, dismiss))),
                    dismiss_requests: requests.computed(),
                },
            )
        }
    }

    #[cfg(test)]
    mod tests {
        use nami::Signal;
        use waterui_controls::menu::CommandExt;
        use waterui_core::{AnyView, Environment, Metadata, View};
        use waterui_text::text::text;

        use super::{ContextMenu, DismissContextMenu, ResolvedContextMenu};
        use crate::ViewExt;

        /// Resolving `.context_menu` attaches `Metadata<ResolvedContextMenu>`
        /// with the accessory kept as a `With` view whose extended environment
        /// is itself attached as `Metadata<Environment>`.
        #[test]
        fn accessory_environment_carries_dismiss_context_menu() {
            let env = Environment::new();
            let resolved = AnyView::new(
                text("target")
                    .context_menu(ContextMenu::new("Reply".action(|| {})).accessory(text("like")))
                    .body(&env),
            )
            .downcast::<Metadata<ResolvedContextMenu>>()
            .expect("context_menu attaches ResolvedContextMenu")
            .value;

            let accessory_env = AnyView::new(
                resolved
                    .accessory
                    .expect("resolved menu keeps the accessory")
                    .body(&env),
            )
            .downcast::<Metadata<Environment>>()
            .expect("the accessory carries an extended environment")
            .value;

            let dismiss = accessory_env
                .get::<DismissContextMenu>()
                .expect("the accessory environment carries DismissContextMenu");

            let before = resolved.dismiss_requests.snapshot();
            dismiss.dismiss();
            assert_ne!(resolved.dismiss_requests.snapshot(), before);
        }
    }
}

/// Anchored overlay metadata module.
pub mod anchored_overlay {
    use nami::Binding;
    use waterui_core::{
        handler::{AnyViewBuilder, ViewBuilder},
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
    pub struct AnchoredOverlay {
        /// Builds a fresh view for each presentation next to the anchor.
        pub content: AnyViewBuilder,
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

    impl MetadataKey for AnchoredOverlay {}

    impl AnchoredOverlay {
        /// An overlay presenting the view `content` builds while
        /// `is_presented` is `true`.
        ///
        /// It sits below the anchor, centered, with no gap, flips when there
        /// is no room below, stays inside the window, and closes when the
        /// user interacts outside it.
        #[must_use]
        pub fn new(is_presented: &Binding<bool>, content: impl ViewBuilder) -> Self {
            Self {
                content: AnyViewBuilder::new(content).erase(),
                is_presented: is_presented.clone(),
                placement: AnchorPlacement::default(),
                dismissal: Dismissal::OutsideInteraction,
                placed_edge: nami::binding(AnchorPlacement::default().edge),
            }
        }

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
    /// assert_eq!(placement.edge, AnchorEdge::Top);
    /// ```
    ///
    /// The defaults describe a dropdown: below the anchor, centered, no gap,
    /// flipping when there is no room, kept inside the window.
    ///
    /// ```rust
    /// use waterui::metadata::anchored_overlay::{AnchorEdge, AnchorPlacement, Clamp, EdgeAlignment};
    ///
    /// let default = AnchorPlacement::default();
    /// assert_eq!(default.edge, AnchorEdge::Bottom);
    /// assert_eq!(default.alignment, EdgeAlignment::Center);
    /// assert_eq!(default.gap, 0.0);
    /// assert!(default.flip);
    /// assert_eq!(default.clamp, Clamp::Window { margin: 0.0 });
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

            let first = overlay
                .content
                .build()
                .downcast::<ProbeView>()
                .expect("the content builder produces ProbeView");
            let second = overlay
                .content
                .build()
                .downcast::<ProbeView>()
                .expect("the content builder produces ProbeView");

            assert_eq!(builds.get(), 2);
            assert_ne!(first.build, second.build);

            first.shared.set(7);
            assert_eq!(second.shared.snapshot(), 7);
        }
    }
}

/// Secure metadata module.
pub mod secure {
    use waterui_core::metadata::MetadataKey;

    /// Secure metadata for secure fields.
    ///
    /// User would be forbidden to take a screenshot of the view that has this metadata.
    #[derive(Debug)]
    pub struct Secure;

    impl MetadataKey for Secure {}

    impl Default for Secure {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Secure {
        /// Creates a new Secure metadata.
        #[must_use]
        pub const fn new() -> Self {
            Self
        }
    }

    /// Selects the color space for a subtree.
    ///
    /// This is the friendlier API surface for what would otherwise be
    /// [`StandardDynamicRange`] / [`HighDynamicRange`] metadata. Use
    /// [`crate::view::ViewExt::color_space`] to apply it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    #[non_exhaustive]
    pub enum ColorSpace {
        /// Standard dynamic range. Use for content that should not exceed
        /// `1.0` per channel — avatars, screenshots, UI chrome.
        Sdr,
        /// High dynamic range. Honors per-color `headroom` so highlights can
        /// extend beyond the display's nominal white point.
        #[default]
        Hdr,
    }

    nami::impl_constant!(ColorSpace);

    /// Apply standard dynamic range color for this views.
    ///
    /// By default, `WaterUI` enables high dynamic range color for all views.
    ///
    /// However, in some cases, you may want to apply standard dynamic range color for certain views,
    /// for instance, user avatar.
    #[derive(Debug)]
    pub struct StandardDynamicRange;
    impl MetadataKey for StandardDynamicRange {}

    impl StandardDynamicRange {
        /// Creates a new `StandardDynamicRange` metadata.
        #[must_use]
        pub const fn new() -> Self {
            Self
        }
    }

    impl Default for StandardDynamicRange {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Apply high dynamic range color for this views.
    ///
    /// By default, `WaterUI` already applies high dynamic range color for all views.
    ///
    /// But if your parent view applied `StandardDynamicRange` metadata, you would use this metadata to override it.
    #[derive(Debug)]
    pub struct HighDynamicRange;
    impl MetadataKey for HighDynamicRange {}

    impl HighDynamicRange {
        /// Creates a new `HighDynamicRange` metadata.
        #[must_use]
        pub const fn new() -> Self {
            Self
        }
    }

    impl Default for HighDynamicRange {
        fn default() -> Self {
            Self::new()
        }
    }
}
