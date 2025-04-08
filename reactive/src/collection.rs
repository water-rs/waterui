//! # Reactive Collections
//!
//! This module provides efficient reactive collections that can be observed for changes.
//! Unlike the `Compute` trait which is optimized for small, cheaply-cloneable values,
//! the `Collection` trait addresses the performance challenges of working with large
//! data sets in a reactive context.
//!
//! ## Key Benefits
//!
//! - **Efficient Access**: Items are accessed by index without cloning the entire collection
//! - **Granular Reactivity**: Notifications are sent only when the collection structure changes
//! - **Memory Efficiency**: Avoids unnecessary cloning of potentially large collections
//! - **Consistent API**: Works with the same watcher system as other reactive primitives
//!
//! ## Architecture
//!
//! The `Collection` trait provides a minimal interface for reactive collections,
//! focusing on:
//!
//! 1. Individual item access by index
//! 2. Collection modification operations
//! 3. Size information
//! 4. Change notification through watchers
//!
//! Implementations like `Array<T>` provide concrete collection types with
//! specific behavior and storage characteristics.

use core::cell::RefCell;

use alloc::vec::Vec;

use crate::{
    compute::ComputeResult,
    watcher::{Watcher, WatcherGuard, WatcherManager},
};

/// A reactive collection that can be observed for changes.
///
/// The `Collection` trait solves performance challenges when working with large
/// data sets in reactive contexts. Unlike the `Compute` trait which requires
/// cloning entire values for each observation, `Collection` enables:
///
/// - Access to individual items by index
/// - Notifications when the collection structure changes
/// - Efficient memory usage by avoiding unnecessary cloning
///
/// # Type Parameters
///
/// * `Item` - The type of elements stored in the collection
///
/// # Examples
///
/// ```
/// use waterui_reactive::collection::{Collection, Array};
///
/// // Create a reactive array
/// let items = Array::<String>::new(vec!["Item 1".to_string(), "Item 2".to_string()]);
///
/// // Access items individually
/// assert_eq!(items.get(0), Some("Item 1".to_string()));
///
/// // Watch for changes
/// let _guard = items.add_watcher(|_| {
///     println!("Collection changed!");
/// });
///
/// // Modify the collection (triggers the watcher)
/// items.remove(0);
/// ```
pub trait Collection {
    /// The type of items contained in this collection.
    type Item;

    /// Retrieves an item at the specified index.
    ///
    /// # Parameters
    ///
    /// * `index` - The position of the item to retrieve
    ///
    /// # Returns
    ///
    /// * `Some(Item)` if an item exists at the specified index
    /// * `None` if the index is out of bounds
    fn get(&self, index: usize) -> Option<Self::Item>;

    /// Removes an item at the specified index.
    ///
    /// This operation will notify any registered watchers about the change.
    ///
    /// # Parameters
    ///
    /// * `index` - The position of the item to remove
    fn remove(&self, index: usize);

    /// Returns the number of items in the collection.
    ///
    /// # Returns
    ///
    /// The count of items currently in the collection
    fn len(&self) -> usize;

    /// Checks if the collection is empty.
    ///
    /// # Returns
    ///
    /// `true` if the collection contains no items, `false` otherwise
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Registers a watcher to be notified when the collection changes.
    ///
    /// # Parameters
    ///
    /// * `watcher` - The watcher function to call when changes occur
    ///
    /// # Returns
    ///
    /// A guard that, when dropped, will unregister the watcher
    fn add_watcher(&self, watcher: Watcher<()>) -> WatcherGuard;
}

/// A reactive array implementation backed by a `Vec`.
///
/// `Array<T>` provides a reactive collection with efficient index-based
/// access to elements. It uses interior mutability via `RefCell` to allow
/// modifications while maintaining shared ownership.
///
/// # Type Parameters
///
/// * `T` - The type of elements stored in the array
///
/// # Examples
///
/// ```
/// use waterui_reactive::collection::Array;
///
/// // Create a new array with initial values
/// let numbers = Array::new(vec![1, 2, 3, 4, 5]);
///
/// // Access elements
/// assert_eq!(numbers.get(2), Some(3));
///
/// // Watch for changes
/// let _guard = numbers.add_watcher(|_| {
///     println!("Array was modified!");
/// });
///
/// // Modify the array (triggers watcher)
/// numbers.remove(1);
/// ```
pub struct Array<T> {
    /// The underlying vector storage with interior mutability
    inner: RefCell<Vec<T>>,
    /// Manager for watchers interested in collection changes
    watchers: WatcherManager<()>,
}

impl<T> Array<T> {
    /// Creates a new reactive array from a vector.
    ///
    /// # Parameters
    ///
    /// * `items` - Initial items for the array
    ///
    /// # Returns
    ///
    /// A new reactive array containing the provided items
    pub fn new(items: Vec<T>) -> Self {
        Self {
            inner: RefCell::new(items),
            watchers: WatcherManager::default(),
        }
    }

    /// Adds a new item to the end of the array.
    ///
    /// # Parameters
    ///
    /// * `item` - The item to add to the array
    pub fn push(&self, item: T) {
        self.inner.borrow_mut().push(item);
        self.watchers.notify(());
    }

    /// Clears all items from the array.
    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
        self.watchers.notify(());
    }
}

impl<T: ComputeResult> Collection for Array<T> {
    type Item = T;
    fn get(&self, index: usize) -> Option<Self::Item> {
        self.inner.borrow().get(index).cloned()
    }
    fn remove(&self, index: usize) {
        if index < self.len() {
            self.inner.borrow_mut().remove(index);
            self.watchers.notify(());
        }
    }

    fn len(&self) -> usize {
        self.inner.borrow().len()
    }

    fn add_watcher(&self, watcher: Watcher<()>) -> WatcherGuard {
        WatcherGuard::from_id(&self.watchers, self.watchers.register(watcher))
    }
}
