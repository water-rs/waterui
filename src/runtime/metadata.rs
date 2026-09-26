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

    /// A context menu: the menu itself, plus an optional lifted preview of
    /// the view it acts on and an optional interactive accessory anchored to
    /// that preview.
    ///
    /// The menu appears when the user long-presses (iOS, Android, touch and
    /// pen elsewhere) or secondary-clicks (macOS, desktop pointers). Menu rows
    /// stay within what every platform's own menu can render; custom content
    /// belongs in the [`accessory`](Self::accessory), which each backend
    /// presents next to the preview, outside the menu.
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
    pub struct ContextMenu {
        /// The menu items to display in the context menu.
        pub items: Computed<Vec<MenuItem>>,
        /// The view lifted while the menu is open. `None` lifts the source
        /// view itself.
        pub preview: Option<AnyView>,
        /// An interactive view anchored to the lifted preview, presented
        /// outside the menu.
        pub accessory: Option<AnyView>,
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

        /// Lifts `preview` instead of the source view while the menu is open.
        #[must_use]
        pub fn preview(mut self, preview: impl View) -> Self {
            self.preview = Some(AnyView::new(preview));
            self
        }

        /// Anchors an interactive `accessory` to the lifted preview.
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
        /// The view to lift while the menu is open; `None` lifts the source
        /// view.
        pub preview: Option<AnyView>,
        /// The accessory to anchor to the lifted preview, with
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
    pub struct ContextMenuView<Content> {
        /// The wrapped content view.
        pub content: Content,
        /// The context menu to resolve.
        pub menu: ContextMenu,
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
    use waterui_core::{AnyView, View, metadata::MetadataKey};

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
    /// # Example
    ///
    /// ```rust
    /// use waterui::metadata::anchored_overlay::{AnchorEdge, AnchoredOverlay, Clamp};
    /// use waterui::prelude::*;
    ///
    /// let shown = binding(false);
    /// let icon = text!("?").anchored_overlay(
    ///     AnchoredOverlay::new(&shown, text!("Opens the help page"))
    ///         .edge(AnchorEdge::Top)
    ///         .gap(4.0)
    ///         .clamp(Clamp::Window { margin: 2.0 }),
    /// );
    /// ```
    #[derive(Debug)]
    pub struct AnchoredOverlay {
        /// The view presented next to the anchor.
        pub content: AnyView,
        /// Whether the overlay is presented. The backend writes `false` when
        /// it dismisses the overlay itself, as [`dismissal`](Self::dismissal)
        /// allows.
        pub is_presented: Binding<bool>,
        /// Where the overlay sits relative to the anchor.
        pub placement: AnchorPlacement,
        /// What besides the binding closes the overlay.
        pub dismissal: Dismissal,
    }

    impl MetadataKey for AnchoredOverlay {}

    impl AnchoredOverlay {
        /// An overlay showing `content` while `is_presented` is `true`.
        ///
        /// It sits below the anchor, centered, with no gap, flips when there
        /// is no room below, stays inside the window, and closes when the
        /// user interacts outside it.
        #[must_use]
        pub fn new(is_presented: &Binding<bool>, content: impl View) -> Self {
            Self {
                content: AnyView::new(content),
                is_presented: is_presented.clone(),
                placement: AnchorPlacement::default(),
                dismissal: Dismissal::OutsideInteraction,
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
    }

    /// Where an [`AnchoredOverlay`] sits relative to its anchor.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct AnchorPlacement {
        /// The edge of the anchor the overlay is placed against.
        pub edge: AnchorEdge,
        /// How the overlay lines up with the anchor along that edge.
        pub alignment: EdgeAlignment,
        /// The distance between the anchor and the overlay, in points.
        pub gap: f32,
        /// Whether the overlay moves to the opposite edge when the preferred
        /// edge has no room for it.
        pub flip: bool,
        /// How the overlay is kept inside the window.
        pub clamp: Clamp,
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

    /// An edge of the anchor. `Leading` and `Trailing` follow the layout
    /// direction.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum EdgeAlignment {
        /// The overlay's start side lines up with the anchor's start side.
        Start,
        /// The overlay is centered on the anchor.
        Center,
        /// The overlay's end side lines up with the anchor's end side.
        End,
    }

    /// How an overlay is kept inside the window.
    #[derive(Debug, Clone, Copy, PartialEq)]
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
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum Dismissal {
        /// Only setting the binding to `false` closes the overlay.
        Manual,
        /// The backend also closes the overlay, writing `false` to its
        /// binding, when the user interacts outside it.
        OutsideInteraction,
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
