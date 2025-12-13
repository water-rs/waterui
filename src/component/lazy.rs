//! Lazy loading utilities for efficient rendering of large collections.
//!
//! This module provides convenience methods for creating lazy layouts that reconstruct
//! views on-demand. This is particularly useful for rendering large collections of items
//! where loading all items at once would be inefficient.
//!
//! # Usage
//!
//! ```ignore
//! use waterui::prelude::*;
//!
//! // Create a lazy vertical stack with 1000 items
//! let list = Lazy::vstack((0..1000).map(|i| text(format!("Item {}", i))));
//! ```

use nami::collection::Collection;
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
    /// ```ignore
    /// let list = Lazy::vstack((0..1000).map(|i| text(format!("Item {}", i))));
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
                spacing,
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
                spacing,
                ..Default::default()
            },
            contents,
        ))
    }

    /// Creates a lazy vertical stack by iterating over a collection and generating views.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let items = vec![Item::new(1, "First"), Item::new(2, "Second")];
    /// let list = Lazy::for_each(items, |item| text(item.name));
    /// ```
    pub fn for_each<C, F, V>(collection: C, generator: F) -> impl View
    where
        C: Collection,
        C::Item: Identifiable,
        F: 'static + Fn(C::Item) -> V,
        V: View,
    {
        Self::vstack(ForEach::new(collection, generator))
    }
}
