//! Task management utilities and async support.
//!
//! This module provides utilities for handling asynchronous operations and managing
//! reactive state changes in the WaterUI framework. It includes components for
//! watching computed values and executing callbacks when they change.

use core::cell::RefCell;

pub use executor_core::{spawn, spawn_local};
use nami::watcher::WatcherGuard;
use waterui_core::{Signal, View};

/// A view that executes a callback when a computed value changes.
#[derive(Debug)]
pub struct OnChange<V, G> {
    content: V,
    _guard: G,
}

impl<V, G> OnChange<V, G> {
    /// Creates a new `OnChange` view that will execute the provided handler
    /// whenever the source value changes.
    ///
    /// # Arguments
    ///
    /// * `content` - The view to render
    /// * `source` - The computed value to watch for changes
    /// * `handler` - The callback to execute when the value changes
    pub fn new<C, F>(content: V, source: &C, handler: F) -> OnChange<V, C::Guard>
    where
        C: Signal,
        V: View,
        C::Output: PartialEq + Clone,
        F: Fn(C::Output) + 'static,
    {
        let cache: RefCell<Option<C::Output>> = RefCell::new(None);
        let guard = source.watch(move |context| {
            let value = context.value;
            if let Some(cache) = &mut *cache.borrow_mut()
                && *cache != value
            {
                *cache = value.clone();
                handler(value);
            }
        });
        OnChange {
            content,
            _guard: guard,
        }
    }
}

impl<V: View, G: WatcherGuard> View for OnChange<V, G> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        self.content
    }
}

/// Creates an `OnChange` view that watches a computed value and executes a callback when it changes.
///
/// This is a convenience function for creating `OnChange` views without having to specify
/// the full type parameters.
///
/// # Arguments
///
/// * `content` - The view to render
/// * `source` - The computed value to watch for changes  
/// * `handler` - The callback to execute when the value changes
///
/// # Returns
///
/// An `OnChange` view that will execute the handler when the source value changes.
///
/// # Example
///
/// ```rust
/// # use waterui::prelude::*;
/// let counter = binding(0);
/// let view = task(
///     "Hello World",
///     &counter,
///     |value| println!("Counter changed to: {}", value)
/// );
/// ```
pub fn task<V, C, F>(content: V, source: &C, handler: F) -> OnChange<V, C::Guard>
where
    C: Signal,
    V: View,
    C::Output: PartialEq + Clone,
    F: Fn(C::Output) + 'static,
{
    OnChange::<V, C::Guard>::new(content, source, handler)
}
