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
