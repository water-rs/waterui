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

use crate::{AnyView, View, raw_view};
use alloc::{boxed::Box, rc::Rc};
use waterui_reactive::Compute;
use waterui_reactive::Computed;
use waterui_reactive::compute::ComputeResult;
/// A dynamic view that can be updated.
///
/// Represents a view whose content can be changed dynamically at runtime.
#[derive(Default)]
pub struct Dynamic(Rc<RefCell<DyanmicInner>>);

raw_view!(Dynamic);

/// A handler for updating a Dynamic view.
///
/// Provides methods to set new content for the associated Dynamic view.
pub struct DynamicHandler(Rc<RefCell<DyanmicInner>>);

#[derive(Default)]
struct DyanmicInner {
    receiver: Option<Box<dyn Fn(AnyView)>>,
    tmp: Option<AnyView>,
}

impl_debug!(Dynamic);
impl_debug!(DynamicHandler);

impl DynamicHandler {
    /// Sets a new view for the associated Dynamic view.
    ///
    /// If the Dynamic view is already connected to a receiver, the new view
    /// is immediately passed to the receiver. Otherwise, it's stored temporarily
    /// until a receiver is connected.
    ///
    /// # Arguments
    ///
    /// * `view` - The new view to set
    pub fn set(&self, view: impl View) {
        let view = AnyView::new(view);
        let mut this = self.0.borrow_mut();
        if let Some(ref receiver) = this.receiver {
            receiver(view)
        } else {
            this.tmp = Some(view);
        }
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
        let inner = Rc::new(RefCell::new(DyanmicInner::default()));
        (DynamicHandler(inner.clone()), Self(inner))
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
        waterui_reactive::watcher::watch(&value, move |value| handle.set(f(value))).leak();
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
    pub fn connect(self, receiver: impl Fn(AnyView) + 'static) {
        let mut this = self.0.borrow_mut();
        if let Some(view) = this.tmp.take() {
            receiver(view);
        }
        this.receiver = Some(Box::new(receiver));
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
