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
//! ```
//! use waterui::dynamic::{Dynamic, watch};
//! use waterui_reactive::Mutable;
//!
//! // Create a dynamic view with a handler
//! let (handler, view) = Dynamic::new();
//! handler.set(text("Initial content"));
//!
//! // Create a view that watches a reactive value
//! let count = Mutable::new(0);
//! let counter_view = watch(count, |value| text(format!("Count: {}", value)));
use core::cell::RefCell;

use crate::{raw_view, AnyView, View};
use alloc::rc::Rc;
use waterui_reactive::compute::ComputeResult;
use waterui_reactive::watcher::Metadata;
use waterui_reactive::Compute;
use waterui_reactive::Computed;

/// A dynamic view that can be updated.
///
/// Represents a view whose content can be changed dynamically at runtime.
pub struct Dynamic(DynamicHandler);

mod ffi {
    use std::sync::Arc;

    use waterui_task::OnceValue;

    use super::Dynamic;
    #[derive(uniffi::Object)]
    pub struct FFIDynamic(OnceValue<Dynamic>);
    #[uniffi::export]
    impl FFIDynamic {
        pub fn connect(&self) {}
    }

    uniffi::custom_type!(Dynamic, Arc<FFIDynamic>,{
        lower:|value| {
            Arc::new(FFIDynamic(value.into()))
        },
        try_lift:|value| {
           Ok(value.0.take())
        }
    });
}

raw_view!(Dynamic);

/// A handler for updating a Dynamic view.
///
/// Provides methods to set new content for the associated Dynamic view.
#[derive(Clone)]
pub struct DynamicHandler(Rc<RefCell<Receiver>>);

type Receiver = Box<dyn Fn(AnyView, Metadata)>;

impl_debug!(Dynamic);
impl_debug!(DynamicHandler);

impl DynamicHandler {
    pub fn set_with_metadata(&self, view: impl View, metadata: Metadata) {
        (self.0.borrow())(AnyView::new(view), metadata)
    }

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
    /// A tuple containing the DynamicHandler and Dynamic view
    pub fn new() -> (DynamicHandler, Self) {
        let handler = DynamicHandler(Rc::new(RefCell::new(Box::new(|_, _| {}))));
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
    pub fn watch<T, V: View>(
        value: impl Compute<Output = T>,
        f: impl Fn(T) -> V + 'static,
    ) -> Self {
        let (handle, dyanmic) = Self::new();
        handle.set(f(value.compute()));
        waterui_reactive::watcher::watch(&value, move |value, _| handle.set(f(value))).leak();
        dyanmic
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
    pub fn connect(self, receiver: impl Fn(AnyView, Metadata) + 'static) {
        self.0 .0.replace(Box::new(receiver));
    }
}

/// Creates a view that watches a reactive value.
///
/// A convenience function that calls Dynamic::watch.
///
/// # Arguments
///
/// * `value` - The reactive value to watch
/// * `f` - A function that converts the value to a view
///
/// # Returns
///
/// A view that updates when the value changes
pub fn watch<T, V: View>(
    value: impl Compute<Output = T>,
    f: impl Fn(T) -> V + 'static,
) -> impl View {
    Dynamic::watch(value, f)
}

impl<V: ComputeResult + View> View for Computed<V> {
    fn body(self, _env: &crate::Environment) -> impl View {
        Dynamic::watch(self, |view| view)
    }
}
