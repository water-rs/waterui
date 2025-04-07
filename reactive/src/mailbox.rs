//! # Thread-safe Access to Reactive Values
//!
//! This module provides the `Mailbox` type, which enables safe access to reactive bindings
//! across thread boundaries and async contexts. It serves as a bridge between the single-threaded
//! reactive system and concurrent or asynchronous code.
//!
//! ## Overview
//!
//! The core reactive system in WaterUI is designed to be used from a single thread (typically the main thread).
//! However, modern applications often need to perform operations across thread boundaries, especially
//! when dealing with asynchronous operations, background tasks, or external APIs.
//!
//! `Mailbox` solves this problem by providing a thread-safe interface to access and modify reactive
//! bindings, ensuring that all operations are properly coordinated through the main thread.
//!
//! ## Usage Pattern
//!
//! ```rust
//! use waterui_reactive::{binding, mailbox::Mailbox};
//!
//! // On the main thread
//! let counter = binding(0);
//! let mailbox = Mailbox::new(&counter);
//!
//! // Pass mailbox to another thread or async context
//! async_fn(mailbox).await;
//!
//! async fn async_fn(mailbox: Mailbox<i32>) {
//!     // Get current value
//!     let value: i32 = mailbox.get().await;
//!
//!     // Set new value
//!     mailbox.set(value + 1).await;
//!
//!     // Watch for changes
//!     let _guard = mailbox.watch(|new_value| {
//!         println!("Counter changed to: {}", new_value);
//!     }).await;
//! }
//! ```
//!
//! ## Key Benefits
//!
//! - **Thread Safety**: Safely interact with reactive values from any thread
//! - **Async Support**: Seamlessly integrate with async/await code
//! - **Type Conversion**: Convert between compatible types during get/set operations
//! - **Consistent Model**: Maintain the same reactive programming model across your application

use crate::{Binding, Compute, compute::ComputeResult, watcher::WatcherGuard};
use waterui_task::MainValue;

/// A thread-safe interface for interacting with reactive bindings across thread boundaries.
///
/// `Mailbox` provides methods to safely get, set, and watch reactive values from any thread
/// by ensuring all operations are performed on the main thread where the reactive system runs.
pub struct Mailbox<T: ComputeResult> {
    /// The wrapped binding, accessed through `MainValue` to ensure thread-safety
    binding: MainValue<Binding<T>>,
}

impl<T: ComputeResult> Mailbox<T> {
    /// Creates a new `Mailbox` that provides thread-safe access to the given binding.
    ///
    /// # Parameters
    ///
    /// * `binding` - A reference to the binding this mailbox will provide access to
    ///
    /// # Returns
    ///
    /// A new `Mailbox` instance that can be safely passed between threads
    pub fn new(binding: &Binding<T>) -> Self {
        Self {
            binding: MainValue::new(binding.clone()),
        }
    }

    /// Retrieves the current value from the binding, with optional type conversion.
    ///
    /// This method dispatches the get operation to the main thread and awaits the result.
    ///
    /// # Type Parameters
    ///
    /// * `V` - The target type to convert the binding's value to
    ///
    /// # Returns
    ///
    /// The current value of the binding, converted to type `V`
    ///
    /// # Examples
    ///
    /// ```
    /// let binding = binding(42);
    /// let mailbox = Mailbox::new(&binding);
    ///
    /// // Get value as i32
    /// let value: i32 = mailbox.get().await;
    /// assert_eq!(value, 42);
    ///
    /// // Get value converted to String
    /// let value: String = mailbox.get().await;
    /// assert_eq!(value, "42");
    /// ```
    pub async fn get<V: Send + 'static + From<T>>(&self) -> V {
        self.binding.handle(|v| V::from(v.get())).await
    }

    /// Sets a new value for the binding, with optional type conversion.
    ///
    /// This method dispatches the set operation to the main thread and awaits its completion.
    ///
    /// # Parameters
    ///
    /// * `value` - The new value to set, which will be converted to the binding's type
    ///
    /// # Examples
    ///
    /// ```
    /// let binding = binding(0);
    /// let mailbox = Mailbox::new(&binding);
    ///
    /// // Set with direct compatible type
    /// mailbox.set(42).await;
    ///
    /// // Set with a type that can be converted
    /// mailbox.set("100").await; // Assuming i32::from_str works for this binding
    /// ```
    pub async fn set<V: Send + 'static + Into<T>>(&self, value: V) {
        self.binding
            .handle(|v| {
                v.set(value.into());
            })
            .await;
    }

    /// Registers a watcher that will be notified when the binding's value changes.
    ///
    /// This method dispatches the watch operation to the main thread and returns
    /// a thread-safe handle to the watcher guard.
    ///
    /// # Parameters
    ///
    /// * `watcher` - A function that will be called when the binding's value changes
    ///
    /// # Returns
    ///
    /// A `MainValue<WatcherGuard>` that, when dropped, will unregister the watcher
    ///
    /// # Examples
    ///
    /// ```
    /// let binding = binding(0);
    /// let mailbox = Mailbox::new(&binding);
    ///
    /// // Watch for changes from another thread
    /// let _guard = mailbox.watch(|value| {
    ///     println!("Value changed to: {}", value);
    /// }).await;
    /// ```
    pub async fn watch(&self, watcher: impl Fn(T) + Send + 'static) -> MainValue<WatcherGuard> {
        self.binding
            .handle(move |v| MainValue::new(v.add_watcher(watcher.into())))
            .await
    }
}
