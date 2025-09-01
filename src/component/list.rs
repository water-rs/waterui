//! List component implementation for WaterUI.
//!
//! This module provides the necessary components to build and configure lists
//! in the WaterUI framework. It includes the `List` component for displaying collections
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

use crate::{
    component::views::{AnyViews, ForEach},
    view::ViewExt,
};
use waterui_core::{AnyView, Environment, View};
impl_compute_result!(AnyViews<ListItem>);

/// Configuration for a list component.
#[derive(Debug)]
pub struct ListConfig {
    /// Content items to be displayed in the list.
    pub contents: AnyViews<ListItem>,
}

/// A component that displays items in a list format.
#[derive(Debug)]
pub struct List<C, F, V>(ForEach<C, F, V, V>);

impl<C, F, V> List<C, F, V>
where
    C: Collection,
    F: Fn(C::Item) -> V,
    V: View,
{
    /// Creates a new list from a collection and a function to generate views.
    ///
    /// # Arguments
    /// * `data` - The collection of data to display
    /// * `generator` - Function to transform collection items into views
    pub fn new(data: C, generator: F) -> Self {
        Self(ForEach::new(data, generator))
    }

    /// Enables search functionality for the list.
    pub fn searchable() {}
}

/// An item in a list that can be configured with various behaviors.
pub struct ListItem {
    /// The view content to display for this item.
    pub content: AnyView,
    /// Optional callback function for when the item is deleted.
    pub on_delete: Option<OnDelete>,
}

type OnDelete = Box<dyn Fn(&Environment, usize)>;

impl_debug!(ListItem);

impl ListItem {
    /// Sets a callback function to be executed when the item is deleted.
    ///
    /// # Arguments
    /// * `on_delete` - The callback function that receives environment and index
    pub fn on_delete(mut self, on_delete: impl Fn(&Environment, usize) + 'static) -> Self {
        self.on_delete = Some(Box::new(on_delete));
        self
    }

    /// Disables deletion functionality for this item.
    pub fn disable_delete(mut self) -> Self {
        self.on_delete = None;
        self
    }
}

impl<V> From<V> for ListItem
where
    V: View,
{
    fn from(value: V) -> Self {
        Self {
            content: value.anyview(),
            on_delete: None,
        }
    }
}
