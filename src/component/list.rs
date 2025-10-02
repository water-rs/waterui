//! List component implementation for `WaterUI`.
//!
//! This module provides the necessary components to build and configure lists
//! in the `WaterUI` framework. It includes the `List` component for displaying collections
//! of data, and `ListItem` for configuring individual items in the list.
//!
//! # Examples
//!
//! ```
//! use waterui::list::List;
//! use nami::collection::Vec;
//!
//! // Create a simple list from a vector of strings
//! let data = Vec::from(["Item 1", "Item 2", "Item 3"]);
//! let list = List::new(data, |item| Text::new(item));
//! ```

use alloc::boxed::Box;
use nami::collection::Collection;

use crate::component::views::{AnyViews, ForEach, Views};
use waterui_core::{AnyView, Environment, View, id::Identifable};

/// Configuration for a list component.
#[derive(Debug)]
pub struct ListConfig {
    /// Content items to be displayed in the list.
    pub contents: AnyViews<ListItem>,
}

/// A component that displays items in a list format.
#[derive(Debug)]
pub struct List<V: Views<View = ListItem>>(V);

impl<V> List<V>
where
    V: Views<View = ListItem>,
{
    /// Creates a new list with the specified contents.
    ///
    /// # Arguments
    /// * `contents` - A collection of items to display in the list.
    pub const fn new(contents: V) -> Self {
        Self(contents)
    }
}

impl<C, F> List<ForEach<C, F, ListItem>>
where
    C: Collection,
    C::Item: Identifable,
    F: 'static + Fn(C::Item) -> ListItem,
{
    /// Creates a new list by iterating over a collection and generating items.
    ///
    /// # Arguments
    /// * `data` - The collection to iterate over
    /// * `generator` - A function that converts each collection item into a `ListItem`
    pub fn for_each(data: C, generator: F) -> Self {
        Self(ForEach::new(data, generator))
    }
}

/// An item in a list that can be configured with various behaviors.
pub struct ListItem {
    /// The view content to display for this item.
    pub content: AnyView,
    /// Optional callback function for when the item is deleted.
    pub on_delete: Option<OnDelete>,
}

impl View for ListItem {
    fn body(self, _env: &Environment) -> impl View {
        self.content
    }
}

type OnDelete = Box<dyn Fn(&Environment, usize)>;

impl_debug!(ListItem);

impl ListItem {
    /// Sets a callback function to be executed when the item is deleted.
    ///
    /// # Arguments
    /// * `on_delete` - The callback function that receives environment and index
    #[must_use]
    pub fn on_delete(mut self, on_delete: impl Fn(&Environment, usize) + 'static) -> Self {
        self.on_delete = Some(Box::new(on_delete));
        self
    }

    /// Disables deletion functionality for this item.
    #[must_use] 
    pub fn disable_delete(mut self) -> Self {
        self.on_delete = None;
        self
    }
}
