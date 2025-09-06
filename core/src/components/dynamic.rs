//! Dynamic views that can be updated at runtime.
//!
//! This module provides components for creating views that can change their content
//! based on reactive state or explicit updates.
//!
//! - `Dynamic` - A view that can be updated through a `DynamicHandler`
//! - `watch` - Helper function to create views that respond to reactive state changes
//!
//! # Examples
//!
//! ```rust
//! use waterui_core::{components::dynamic::{Dynamic,watch},Binding};
//!
//! // Create a dynamic view with a handler
//! let (handler, view) = Dynamic::new();
//! handler.set("Initial content");
//!
//! // Create a view that watches a reactive value
//! let count = Binding::container(0);
//! let counter_view = watch(count, |value| format!("Count: {}", value));
use crate::components::With;
use crate::{AnyView, View, raw_view};
use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::RefCell;
use nami::watcher::Context;
use nami::{Computed, Signal, watcher::Metadata};

/// A dynamic view that can be updated.
///
/// Represents a view whose content can be changed dynamically at runtime.
pub struct Dynamic(DynamicHandler);

raw_view!(Dynamic);

/// A handler for updating a Dynamic view.
///
/// Provides methods to set new content for the associated Dynamic view.
#[derive(Clone)]
pub struct DynamicHandler(Rc<RefCell<Receiver>>);

type Receiver = Box<dyn Fn(Context<AnyView>)>;

impl_debug!(Dynamic);
impl_debug!(DynamicHandler);

impl DynamicHandler {
    /// Sets the content of the Dynamic view with the provided view and metadata.
    ///
    /// # Arguments
    ///
    /// * `view` - The new view to display
    /// * `metadata` - Additional metadata associated with the update
    pub fn set_with_metadata(&self, view: impl View, metadata: Metadata) {
        (self.0.borrow())(Context::new(AnyView::new(view), metadata));
    }

    /// Sets the content of the Dynamic view with the provided view.
    ///
    /// # Arguments
    ///
    /// * `view` - The new view to display
    pub fn set(&self, view: impl View) {
        self.set_with_metadata(view, Metadata::new());
    }
}

impl Dynamic {
    /// Creates a new Dynamic view along with its handler.
    ///
    /// Returns a tuple of (handler, view) where the handler can be used to update
    /// the view's content.
    ///
    /// # Returns
    ///
    /// A tuple containing the [`DynamicHandler`] and Dynamic view
    #[must_use]
    pub fn new() -> (DynamicHandler, Self) {
        let handler = DynamicHandler(Rc::new(RefCell::new(Box::new(|_| {}))));
        (handler.clone(), Self(handler))
    }

    /// Creates a Dynamic view that watches a reactive value.
    ///
    /// The provided function is used to convert the value to a view.
    /// Whenever the watched value changes, the view will update automatically.
    ///
    /// # Arguments
    ///
    /// * `value` - The reactive value to watch
    /// * `f` - A function that converts the value to a view
    ///
    /// # Returns
    ///
    /// A Dynamic view that updates when the value changes
    pub fn watch<T, S, V: View>(value: S, f: impl 'static + Fn(T) -> V) -> impl View
    where
        S: Signal<Output = T>,
    {
        let (handle, dynamic) = Self::new();
        handle.set(f(value.get()));

        let guard = value.watch(move |value| handle.set(f(value.value)));

        With::new(dynamic, (guard, value))
    }

    /// Connects the Dynamic view to a receiver function.
    ///
    /// The receiver function is called whenever the view content is updated.
    /// If there's a temporary view stored (set before connecting), it will
    /// be immediately passed to the receiver.
    ///
    /// # Arguments
    ///
    /// * `receiver` - A function that receives view updates
    pub fn connect(self, receiver: impl Fn(Context<AnyView>) + 'static) {
        #[allow(unused_must_use)]
        // It would be used on swift side
        self.0.0.replace(Box::new(receiver));
    }
}

/// Creates a view that watches a reactive value.
///
/// A convenience function that calls [`Dynamic::watch`].
///
/// # Arguments
///
/// * `value` - The reactive value to watch
/// * `f` - A function that converts the value to a view
///
/// # Returns
///
/// A view that updates when the value changes
pub fn watch<T, S, V: View>(value: S, f: impl Fn(T) -> V + 'static) -> impl View
where
    S: Signal<Output = T>,
{
    Dynamic::watch(value, f)
}

impl<V: View> View for Computed<V>
where
    Self: 'static,
{
    fn body(self, _env: &crate::Environment) -> impl View {
        Dynamic::watch(self, |view| view)
    }
}
