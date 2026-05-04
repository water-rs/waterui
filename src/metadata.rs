//! Metadata definitions for `WaterUI`.
//!
//! `Metadata`s are some extra information that can be attached to `View`s to modify their behavior
//! or appearance, but not affect their layout.
//!
//! They are defined as types that implement the `MetadataKey` trait.

/// Context menu metadata module.
pub mod context_menu {
    use alloc::vec::Vec;
    use nami::Computed;
    use waterui_controls::menu::{MenuItem, MenuView, ResolvedMenuItem, resolve_menu_items};
    use waterui_core::{Environment, Metadata, View, metadata::MetadataKey};

    /// Context menu metadata for views.
    ///
    /// Attaches a context menu to a view that appears when the user:
    /// - Long-presses on iOS/Android
    /// - Right-clicks on macOS
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// text!("Right-click me")
    ///     .context_menu(vec![
    ///         "Copy".action(|| {}),
    ///         "Paste".action(|| {}),
    ///     ])
    /// ```
    #[derive(Debug)]
    pub struct ContextMenu {
        /// The menu items to display in the context menu.
        pub items: Computed<Vec<MenuItem>>,
    }

    impl MetadataKey for ContextMenu {}

    impl ContextMenu {
        /// Creates a new context menu with the given items.
        #[must_use]
        pub fn new(items: impl MenuView) -> Self {
            Self {
                items: items.into_menu_items(),
            }
        }
    }

    /// Resolved context menu metadata consumed by the native backends.
    #[doc(hidden)]
    #[derive(Debug)]
    pub struct ResolvedContextMenu {
        /// The resolved menu items for the current environment.
        pub items: Computed<Vec<ResolvedMenuItem>>,
    }

    impl MetadataKey for ResolvedContextMenu {}

    /// View wrapper that resolves semantic context menu items against the environment.
    #[doc(hidden)]
    #[derive(Debug)]
    pub struct ContextMenuView<Content> {
        /// The wrapped content view.
        pub content: Content,
        /// Semantic context menu items.
        pub items: Computed<Vec<MenuItem>>,
    }

    impl<Content: View> View for ContextMenuView<Content> {
        fn body(self, env: &Environment) -> impl View {
            let items = resolve_menu_items(&self.items, env);

            Metadata::new(self.content, ResolvedContextMenu { items })
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
