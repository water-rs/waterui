//! Dynamic views that can be updated at runtime.
//!
//! This module provides components for creating views that can change their content
//! based on reactive state or explicit updates.
//!
//! - `Dynamic` - A view that can be updated through a `DynamicHandler`
//! - `watch` - Helper for the exceptional case where reactive state changes view structure
//!
//! # Examples
//!
//! ```rust
//! use waterui_core::{dynamic::{Dynamic, watch}, Binding};
//!
//! // Create a dynamic view with a handler
//! let (handler, view) = Dynamic::new();
//! handler.set("Initial content");
//!
//! // Replace a subtree only when the semantic view type genuinely changes.
//! let show_details = Binding::container(false);
//! let content = watch(show_details, |show| {
//!     if show { "Details" } else { "Summary" }
//! });
use crate::components::metadata::Retain;
use crate::{AnyView, Environment, LatestDispatch, Metadata, SerialDispatch, View};
use alloc::rc::Rc;
use core::cell::RefCell;
use core::marker::PhantomData;
use nami::watcher::Context;
use nami::{Signal, watcher::Metadata as WatcherMetadata};
use waterui_macros::state;

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
///
/// Extracts through `State<Self>`, so `.state(&handler)` injects it and a bare
/// `handler: DynamicHandler` parameter shares positions with
/// `State<DynamicHandler>` ones.
#[state]
#[derive(Clone)]
pub struct DynamicHandler(Rc<DynamicHandlerInner>);

struct DynamicHandlerInner {
    state: RefCell<DynamicHandlerState>,
    /// Updates issued while a receiver call is in flight are delivered after
    /// it returns, in order, rather than re-entering the receiver.
    dispatch: SerialDispatch<(AnyView, WatcherMetadata)>,
}

enum DynamicHandlerState {
    /// Connected to a receiver (Swift/native side).
    Connected {
        receiver: Receiver,
        pending_view_slot: Option<Rc<RefCell<Option<AnyView>>>>,
    },
    /// Not yet connected, stores the initial view if set before connection.
    Unconnected(Option<AnyView>),
}

type Receiver = Rc<dyn Fn(Context<AnyView>)>;

/// Metadata marker for the body-time snapshot installed by [`Dynamic::watch`].
#[derive(Clone, Copy, Debug)]
pub struct DynamicInitialContent;

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
        self.set_any_with_metadata(AnyView::new(view), metadata);
    }

    fn set_any_with_metadata(&self, view: AnyView, metadata: WatcherMetadata) {
        // A `set` that lands while the receiver runs only enqueues; the
        // delivery in flight drains it once the receiver returns, so the
        // receiver is never called re-entrantly.
        self.0
            .dispatch
            .deliver((view, metadata), |(view, metadata)| {
                let receiver = {
                    let mut inner = self.0.state.borrow_mut();
                    match &mut *inner {
                        DynamicHandlerState::Connected { receiver, .. } => Rc::clone(receiver),
                        DynamicHandlerState::Unconnected(temp_view) => {
                            *temp_view = Some(view);
                            return;
                        }
                    }
                };
                receiver(Context::new(view, metadata));
            });
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
        let handler = DynamicHandler(Rc::new(DynamicHandlerInner {
            state: RefCell::new(DynamicHandlerState::Unconnected(None)),
            dispatch: SerialDispatch::new(),
        }));
        (handler.clone(), Self(handler))
    }

    /// Creates a Dynamic view that watches structural reactive state.
    ///
    /// The provided function is used to convert the value to a view.
    /// Whenever the watched value changes, the entire child subtree is replaced.
    /// State owned by the replaced subtree is discarded. Prefer signal-aware
    /// component inputs, modifiers, metadata, and reactive collections for scalar
    /// values or collection membership. Use this only when the semantic view
    /// structure itself must change.
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

    /// Connects the dynamic node while preserving a pending measurement view.
    ///
    /// This is used by renderers that need to stage a temporary child view
    /// before the final backend receiver is attached.
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
        let initial = {
            let mut inner = self.0.0.state.borrow_mut();

            match &mut *inner {
                DynamicHandlerState::Unconnected(temp_view) => {
                    let initial = temp_view.take();
                    *inner = DynamicHandlerState::Connected {
                        receiver: Rc::new(receiver),
                        pending_view_slot,
                    };
                    initial
                }
                DynamicHandlerState::Connected { .. } => {
                    unreachable!("Dynamic already connected")
                }
            }
        };

        // The pre-connection view is delivered through the queue so a set from
        // inside the receiver cannot re-enter it.
        if let Some(view) = initial {
            self.0.set_any_with_metadata(view, WatcherMetadata::new());
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
        let inner = self.0.0.state.borrow();
        match &*inner {
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
        let mut inner = self.0.0.state.borrow_mut();
        match &mut *inner {
            DynamicHandlerState::Unconnected(view) => Some(f(view)),
            DynamicHandlerState::Connected { .. } => None,
        }
    }

    /// Mutates whichever view snapshot should be used for layout measurement.
    ///
    /// Before connection this is the unconnected snapshot; after connection it
    /// targets the pending connected snapshot when one exists.
    pub fn with_measurement_view_mut<R>(
        &self,
        f: impl FnOnce(&mut Option<AnyView>) -> R,
    ) -> Option<R> {
        let mut inner = self.0.0.state.borrow_mut();
        match &mut *inner {
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

    /// Mutates the pending connected snapshot without affecting the
    /// pre-connection snapshot.
    pub fn with_connected_pending_view_mut<R>(
        &self,
        f: impl FnOnce(&mut Option<AnyView>) -> R,
    ) -> Option<R> {
        let mut inner = self.0.0.state.borrow_mut();
        match &mut *inner {
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
    fn body(self, _env: &Environment) -> impl View {
        let (handle, dynamic) = Dynamic::new();
        let f = Rc::new(self.f);

        handle.set_with_metadata(
            f(self.value.snapshot()),
            WatcherMetadata::new().with(DynamicInitialContent),
        );

        let guard = self.value.watch({
            let f = Rc::clone(&f);
            // A write the builder makes right now lands here re-entrantly;
            // keep the latest value and let the dispatch in flight drain it
            // once the builder and the receiver return.
            let updates = LatestDispatch::new();
            move |context| {
                updates.deliver(context.into_value(), |next| handle.set(f(next)));
            }
        });

        Metadata::new(dynamic, Retain::new((guard, self.value)))
    }
}

/// Creates a view that watches structural reactive state.
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
/// A view whose entire child subtree is replaced when the value changes
pub fn watch<T: 'static, S, V: View>(value: S, f: impl Fn(T) -> V + 'static) -> impl View
where
    S: Signal<Output = T> + 'static,
{
    Dynamic::watch(value, f)
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;
    use core::cell::Cell;

    use nami::Binding;

    use super::*;
    use crate::metadata::MetadataKey;

    /// Marks the payload of a delivered view so a test can observe ordering.
    struct Probe(usize);
    impl MetadataKey for Probe {}

    fn probe_of(view: AnyView) -> usize {
        view.downcast::<Metadata<Probe>>()
            .expect("the delivered view carries a Probe")
            .value
            .0
    }

    /// A `set` issued while the receiver still runs must queue behind the
    /// current delivery instead of re-entering the receiver.
    #[test]
    fn a_nested_set_is_delivered_after_the_receiver_returns() {
        let (handle, dynamic) = Dynamic::new();
        let delivered = Rc::new(RefCell::new(Vec::new()));
        let delivering = Cell::new(false);
        let nested = Cell::new(false);
        dynamic.connect({
            let handle = handle.clone();
            let delivered = Rc::clone(&delivered);
            move |context| {
                assert!(
                    !delivering.replace(true),
                    "the receiver must not run re-entrantly"
                );
                delivered.borrow_mut().push(probe_of(context.into_value()));
                if !nested.replace(true) {
                    handle.set(Metadata::new((), Probe(2)));
                }
                delivering.set(false);
            }
        });

        handle.set(Metadata::new((), Probe(1)));

        assert_eq!(&*delivered.borrow(), &[1, 2]);
    }

    /// A `Dynamic::watch` builder that writes the watched signal queues that
    /// write as the pending value and runs again once it has returned.
    #[test]
    fn watch_drains_a_nested_write_after_the_builder_returns() {
        let source = Binding::i32(1);
        let delivered = Rc::new(RefCell::new(Vec::new()));
        let view = Dynamic::watch(source.clone(), {
            let source = source.clone();
            move |value| {
                if value == 2 {
                    source.set(3);
                }
                Metadata::new(
                    (),
                    Probe(usize::try_from(value).expect("watch values are non-negative")),
                )
            }
        });

        let body = AnyView::new(view.body(&Environment::new()));
        // The retain value owns the watcher guard; it must outlive the
        // notifications this test drives.
        let Metadata {
            content,
            value: _retained,
        } = *body
            .downcast::<Metadata<Retain>>()
            .expect("the watched dynamic body carries its retain guard");
        let dynamic = content
            .downcast::<Dynamic>()
            .expect("the watched dynamic body resolves to a Dynamic");
        dynamic.connect({
            let delivered = Rc::clone(&delivered);
            move |context| {
                delivered.borrow_mut().push(probe_of(context.into_value()));
            }
        });

        source.set(2);

        assert_eq!(&*delivered.borrow(), &[1, 2, 3]);
        assert_eq!(source.snapshot(), 3);
    }
}
