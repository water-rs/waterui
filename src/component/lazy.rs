//! Lazy loading utilities for efficient rendering of large collections.
//!
//! This module provides convenience methods for creating lazy layouts that reconstruct
//! views on-demand. This is particularly useful for rendering large collections of items
//! where loading all items at once would be inefficient.
//!
//! # Usage
//!
//! ```rust
//! use waterui::component::lazy::Lazy;
//! use waterui::prelude::*;
//!
//! // Create a lazy vertical stack with 1000 items
//! let list = Lazy::vstack((0..1000).map(|i| text!("Item {i}")).collect::<Vec<_>>());
//! ```

use nami::{Computed, collection::Collection};
use waterui_core::{View, id::Identifiable};
use waterui_layout::{
    LazyContainer,
    scroll::scroll,
    stack::{HStackLayout, VStackLayout},
};

use crate::views::{ForEach, Views};

/// Convenience wrapper for creating lazy layouts.
///
/// `Lazy` provides static methods for creating scrollable, lazy-loading layouts
/// that efficiently render large collections by reconstructing views on-demand.
#[derive(Debug)]
pub struct Lazy;

impl Lazy {
    /// Creates a lazy vertical stack wrapped in a scroll view.
    ///
    /// Views are reconstructed on-demand as they become visible,
    /// making this suitable for large collections.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::component::lazy::Lazy;
    /// use waterui::prelude::*;
    ///
    /// let list = Lazy::vstack((0..1000).map(|i| text!("Item {i}")).collect::<Vec<_>>());
    /// ```
    pub fn vstack<V: View>(contents: impl Views<View = V> + 'static) -> impl View {
        scroll(LazyContainer::new(VStackLayout::default(), contents))
    }

    /// Creates a lazy vertical stack with custom spacing, wrapped in a scroll view.
    pub fn vstack_spaced<V: View>(
        spacing: f32,
        contents: impl Views<View = V> + 'static,
    ) -> impl View {
        scroll(LazyContainer::new(
            VStackLayout {
                spacing: Computed::constant(spacing),
                ..Default::default()
            },
            contents,
        ))
    }

    /// Creates a lazy horizontal stack wrapped in a scroll view.
    ///
    /// Views are reconstructed on-demand as they become visible,
    /// making this suitable for large collections.
    pub fn hstack<V: View>(contents: impl Views<View = V> + 'static) -> impl View {
        scroll(LazyContainer::new(HStackLayout::default(), contents))
    }

    /// Creates a lazy horizontal stack with custom spacing, wrapped in a scroll view.
    pub fn hstack_spaced<V: View>(
        spacing: f32,
        contents: impl Views<View = V> + 'static,
    ) -> impl View {
        scroll(LazyContainer::new(
            HStackLayout {
                spacing: Computed::constant(spacing),
                ..Default::default()
            },
            contents,
        ))
    }

    /// Creates a lazy vertical stack by iterating over a collection and generating views.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::component::lazy::Lazy;
    /// use waterui::prelude::*;
    /// use waterui::id::Id;
    ///
    /// #[derive(Clone)]
    /// struct Item {
    ///     id: Id,
    ///     name: &'static str,
    /// }
    ///
    /// impl waterui_core::id::Identifiable for Item {
    ///     type Id = Id;
    ///
    ///     fn id(&self) -> Id {
    ///         self.id
    ///     }
    /// }
    ///
    /// let items = vec![
    ///     Item { id: Id::try_from(1).unwrap(), name: "First" },
    ///     Item { id: Id::try_from(2).unwrap(), name: "Second" },
    /// ];
    /// let list = Lazy::for_each(items, |item| text::text(item.name));
    /// ```
    pub fn for_each<C, F, V>(collection: C, generator: F) -> impl View
    where
        C: Collection + Clone,
        C::Item: Identifiable,
        F: 'static + Fn(C::Item) -> V,
        V: View,
    {
        Self::vstack(ForEach::new(collection, generator))
    }
}
