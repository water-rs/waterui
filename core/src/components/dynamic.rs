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
//! use waterui_core::{dynamic::{Dynamic,watch},Binding};
//!
//! // Create a dynamic view with a handler
//! let (handler, view) = Dynamic::new();
//! handler.set("Initial content");
//!
//! // Create a view that watches a reactive value
//! let count = Binding::container(0);
//! let counter_view = watch(count, |value| format!("Count: {}", value));
use crate::components::metadata::Retain;
use crate::{AnyView, Environment, LocalStateScope, LocalStateStore, Metadata, View};
use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::RefCell;
use core::marker::PhantomData;
use nami::watcher::Context;
use nami::{Computed, Signal, watcher::Metadata as WatcherMetadata};

/// A dynamic view that can be updated.
///
/// Represents a view whose content can be changed dynamically at runtime.
///
/// You should avoid using this component if possible,
/// most of components in `WaterUI` already provide a way to update their content reactively.
#[derive(Clone)]
pub struct Dynamic(DynamicHandler);

raw_view!(Dynamic);

/// A handler for updating a Dynamic view.
///
/// Provides methods to set new content for the associated Dynamic view.
#[derive(Clone)]
pub struct DynamicHandler(Rc<RefCell<DynamicHandlerState>>);

enum DynamicHandlerState {
    /// Connected to a receiver (Swift/native side).
    Connected {
        receiver: Receiver,
        pending_view_slot: Option<Rc<RefCell<Option<AnyView>>>>,
    },
    /// Not yet connected, stores the initial view if set before connection.
    Unconnected(Option<AnyView>),
}

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
    pub fn set_with_metadata(&self, view: impl View, metadata: WatcherMetadata) {
        let mut state = self.0.borrow_mut();
        let view = AnyView::new(view);
        match &mut *state {
            DynamicHandlerState::Connected { receiver, .. } => {
                receiver(Context::new(view, metadata));
            }
            DynamicHandlerState::Unconnected(temp_view) => {
                *temp_view = Some(view);
            }
        }
    }

    /// Sets the content of the Dynamic view with the provided view.
    ///
    /// # Arguments
    ///
    /// * `view` - The new view to display
    pub fn set(&self, view: impl View) {
        self.set_with_metadata(view, WatcherMetadata::new());
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
        let handler = DynamicHandler(Rc::new(RefCell::new(DynamicHandlerState::Unconnected(
            None,
        ))));
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
        S: Signal<Output = T> + 'static,
        T: 'static,
    {
        WatchedDynamic {
            value,
            f,
            marker: PhantomData,
        }
    }

    /// Connects the Dynamic view to a receiver function.
    ///
    /// For internal use only.
    ///
    /// The receiver function is called whenever the view content is updated.
    /// If there's a temporary view stored (set before connecting), it will
    /// be immediately passed to the receiver.
    ///
    /// # Arguments
    ///
    /// * `receiver` - A function that receives view updates
    pub fn connect(self, receiver: impl Fn(Context<AnyView>) + 'static) {
        self.connect_internal(None, receiver);
    }

    pub fn connect_with_pending_view(
        self,
        pending_view_slot: Rc<RefCell<Option<AnyView>>>,
        receiver: impl Fn(Context<AnyView>) + 'static,
    ) {
        self.connect_internal(Some(pending_view_slot), receiver);
    }

    fn connect_internal(
        self,
        pending_view_slot: Option<Rc<RefCell<Option<AnyView>>>>,
        receiver: impl Fn(Context<AnyView>) + 'static,
    ) {
        let mut state = self.0.0.borrow_mut();

        match &mut *state {
            DynamicHandlerState::Unconnected(temp_view) => {
                if let Some(view) = temp_view.take() {
                    receiver(Context::new(view, WatcherMetadata::new()));
                }
                *state = DynamicHandlerState::Connected {
                    receiver: Box::new(receiver),
                    pending_view_slot,
                };
            }
            DynamicHandlerState::Connected { .. } => unreachable!("Dynamic already connected"),
        }
    }

    /// Returns a stable identity for this dynamic node.
    #[must_use]
    pub fn identity(&self) -> usize {
        Rc::as_ptr(&self.0.0) as usize
    }

    /// Reads the current pre-connection view snapshot, if this dynamic node
    /// has not been connected yet.
    ///
    /// Returns `None` when the node is already connected to a backend receiver.
    pub fn with_unconnected_view<R>(&self, f: impl FnOnce(Option<&AnyView>) -> R) -> Option<R> {
        let state = self.0.0.borrow();
        match &*state {
            DynamicHandlerState::Unconnected(view) => Some(f(view.as_ref())),
            DynamicHandlerState::Connected { .. } => None,
        }
    }

    /// Mutates the current pre-connection view snapshot before the dynamic node connects.
    ///
    /// Returns `None` when the node is already connected to a backend receiver.
    pub fn with_unconnected_view_mut<R>(
        &self,
        f: impl FnOnce(&mut Option<AnyView>) -> R,
    ) -> Option<R> {
        let mut state = self.0.0.borrow_mut();
        match &mut *state {
            DynamicHandlerState::Unconnected(view) => Some(f(view)),
            DynamicHandlerState::Connected { .. } => None,
        }
    }

    pub fn with_measurement_view_mut<R>(
        &self,
        f: impl FnOnce(&mut Option<AnyView>) -> R,
    ) -> Option<R> {
        let mut state = self.0.0.borrow_mut();
        match &mut *state {
            DynamicHandlerState::Unconnected(view) => Some(f(view)),
            DynamicHandlerState::Connected {
                pending_view_slot: Some(slot),
                ..
            } => Some(f(&mut slot.borrow_mut())),
            DynamicHandlerState::Connected {
                pending_view_slot: None,
                ..
            } => None,
        }
    }

    pub fn with_connected_pending_view_mut<R>(
        &self,
        f: impl FnOnce(&mut Option<AnyView>) -> R,
    ) -> Option<R> {
        let mut state = self.0.0.borrow_mut();
        match &mut *state {
            DynamicHandlerState::Connected {
                pending_view_slot: Some(slot),
                ..
            } => Some(f(&mut slot.borrow_mut())),
            DynamicHandlerState::Connected {
                pending_view_slot: None,
                ..
            }
            | DynamicHandlerState::Unconnected(_) => None,
        }
    }
}

fn local_scope(env: &Environment) -> LocalStateScope {
    env.get::<LocalStateScope>()
        .unwrap_or_else(|| panic!("Dynamic::watch requires renderer LocalStateScope support"))
        .clone()
}

fn local_shared<T: 'static>(env: &Environment, init: impl FnOnce() -> T + 'static) -> Rc<T> {
    let scope = local_scope(env);
    env.get::<LocalStateStore>()
        .unwrap_or_else(|| panic!("Dynamic::watch requires renderer LocalStateStore support"))
        .get_or_init(&scope, init)
}

struct WatchedDynamic<T, S, F> {
    value: S,
    f: F,
    marker: PhantomData<fn(T)>,
}

impl<T, S, F, V> View for WatchedDynamic<T, S, F>
where
    T: 'static,
    S: Signal<Output = T> + 'static,
    F: Fn(T) -> V + 'static,
    V: View + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        let runtime = local_shared(env, Dynamic::new);
        let handle = runtime.0.clone();
        let dynamic = runtime.1.clone();
        let f = Rc::new(self.f);

        handle.set(f(self.value.get()));

        let guard = self.value.watch({
            let f = Rc::clone(&f);
            move |value| handle.set(f(value.into_value()))
        });

        Metadata::new(dynamic, Retain::new((guard, self.value)))
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
pub fn watch<T: 'static, S, V: View>(value: S, f: impl Fn(T) -> V + 'static) -> impl View
where
    S: Signal<Output = T> + 'static,
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
