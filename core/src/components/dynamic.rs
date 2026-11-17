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
use crate::backend::{Backend, ChildSlot, MapSlots};
use crate::components::metadata::Retain;
use crate::{
    AnyView, Environment, LatestDispatch, Metadata, Native, NativeView, SerialDispatch, View,
};
use alloc::rc::Rc;
use core::cell::RefCell;
use core::marker::PhantomData;
use nami::watcher::Context;
use nami::{Signal, watcher::Metadata as WatcherMetadata};
use waterui_macros::state;

/// A view whose content is replaced at runtime, by a [`DynamicHandler`].
///
/// `V` is the type of the content it shows. A dynamic fed views of different
/// types holds [`AnyView`], the default.
///
/// You should avoid using this component if possible,
/// most of components in `WaterUI` already provide a way to update their content reactively.
pub struct Dynamic<V: 'static = AnyView>(DynamicHandler<V>);

impl<V> Clone for Dynamic<V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// A handler for updating a [`Dynamic`] view.
///
/// Provides methods to set new content for the associated Dynamic view.
///
/// Extracts through `State<Self>`, so `.state(&handler)` injects it and a bare
/// `handler: DynamicHandler` parameter shares positions with
/// `State<DynamicHandler>` ones.
#[state]
pub struct DynamicHandler<V: 'static = AnyView>(Rc<DynamicHandlerInner<V>>);

impl<V> Clone for DynamicHandler<V> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

struct DynamicHandlerInner<V> {
    state: RefCell<DynamicHandlerState<V>>,
    /// Updates issued while a receiver call is in flight are delivered after
    /// it returns, in order, rather than re-entering the receiver.
    dispatch: SerialDispatch<(V, WatcherMetadata)>,
}

enum DynamicHandlerState<V> {
    /// Connected to the backend's receiver.
    Connected(Rc<dyn Fn(Context<V>)>),
    /// Not yet connected; holds the latest view set before the connection.
    Unconnected(Option<V>),
}

/// Metadata marker for the body-time snapshot installed by [`Dynamic::watch`].
#[derive(Clone, Copy, Debug)]
pub struct DynamicInitialContent;

impl<V> core::fmt::Debug for Dynamic<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Dynamic").finish_non_exhaustive()
    }
}

impl<V> core::fmt::Debug for DynamicHandler<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("DynamicHandler").finish_non_exhaustive()
    }
}

impl<V> DynamicHandler<V> {
    /// Sets the content of the Dynamic view with the provided view and metadata.
    ///
    /// # Arguments
    ///
    /// * `view` - The new view to display
    /// * `metadata` - Additional metadata associated with the update
    pub fn set_with_metadata(&self, view: V, metadata: WatcherMetadata) {
        // A `set` that lands while the receiver runs only enqueues; the
        // delivery in flight drains it once the receiver returns, so the
        // receiver is never called re-entrantly.
        self.0
            .dispatch
            .deliver((view, metadata), |(view, metadata)| {
                let receiver = {
                    let mut inner = self.0.state.borrow_mut();
                    match &mut *inner {
                        DynamicHandlerState::Connected(receiver) => Rc::clone(receiver),
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
    pub fn set(&self, view: V) {
        self.set_with_metadata(view, WatcherMetadata::new());
    }
}

impl<V> Dynamic<V> {
    /// Creates a new Dynamic view along with its handler.
    ///
    /// Returns a tuple of (handler, view) where the handler can be used to update
    /// the view's content.
    ///
    /// # Returns
    ///
    /// A tuple containing the [`DynamicHandler`] and Dynamic view
    #[must_use]
    pub fn new() -> (DynamicHandler<V>, Self) {
        let handler = DynamicHandler(Rc::new(DynamicHandlerInner {
            state: RefCell::new(DynamicHandlerState::Unconnected(None)),
            dispatch: SerialDispatch::new(),
        }));
        (handler.clone(), Self(handler))
    }

    /// Connects the Dynamic view to a receiver function.
    ///
    /// For backends.
    ///
    /// The receiver function is called whenever the view content is updated.
    /// A view set before the connection is delivered to the receiver at once.
    ///
    /// # Arguments
    ///
    /// * `receiver` - A function that receives view updates
    pub fn connect(self, receiver: impl Fn(Context<V>) + 'static) {
        let initial = {
            let mut inner = self.0.0.state.borrow_mut();

            match &mut *inner {
                DynamicHandlerState::Unconnected(temp_view) => {
                    let initial = temp_view.take();
                    *inner = DynamicHandlerState::Connected(Rc::new(receiver));
                    initial
                }
                DynamicHandlerState::Connected(_) => {
                    // A clone of a connected `Dynamic` was realized again.
                    // The first receiver keeps the node; this realization
                    // stays empty.
                    crate::contract_violation!(
                        "a Dynamic was connected twice: the second realization stays empty and the first keeps receiving updates"
                    );
                    return;
                }
            }
        };

        // The pre-connection view is delivered through the queue so a set from
        // inside the receiver cannot re-enter it.
        if let Some(view) = initial {
            self.0.set_with_metadata(view, WatcherMetadata::new());
        }
    }

    /// Returns a stable identity for this dynamic node.
    #[must_use]
    pub fn identity(&self) -> usize {
        Rc::as_ptr(&self.0.0).cast::<()>() as usize
    }
}

impl<V: View> Dynamic<V> {
    /// The same dynamic delivering each view in the slot `S`: a dynamic
    /// forwarding to this one's handler, converting every view it delivers.
    fn into_slots<S: ChildSlot>(self) -> Dynamic<S> {
        let (handler, slots) = Dynamic::new();
        self.connect(move |context| {
            let (view, metadata) = context.into_parts();
            handler.set_with_metadata(S::from_view(view), metadata);
        });
        slots
    }
}

impl Dynamic {
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
}

/// A dynamic reaches its backend as a `Dynamic` of the backend's child
/// slot; the content it shows has its own stretch axis, answered by the
/// realized node.
impl<S: 'static> NativeView for Dynamic<S> {}

impl<S: 'static> MapSlots<S> for Dynamic<S> {
    type With<T: 'static> = Dynamic<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> Dynamic<T> {
        let (handler, slots) = Dynamic::new();
        self.connect(move |context| {
            let (view, metadata) = context.into_parts();
            handler.set_with_metadata(f(view), metadata);
        });
        slots
    }
}

impl<V: View> View for Dynamic<V> {
    /// The erased form, for code that expands views by hand.
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self.into_slots::<AnyView>())
    }

    /// Reaches the backend with each delivered view in the backend's child
    /// slot.
    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_slots::<B::Slot>(), env, cx)
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
            // once the builder and the receiver return. The change's metadata
            // (`NoAnimation`, an explicit animation) travels with its value to
            // the receiver, so the structural site can honor it.
            let updates = LatestDispatch::new();
            move |context: Context<T>| {
                updates.deliver(context, |next| {
                    let (value, metadata) = next.into_parts();
                    handle.set_with_metadata(f(value), metadata);
                });
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

    fn probe_of(view: &Metadata<Probe, ()>) -> usize {
        view.value.0
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
                delivered.borrow_mut().push(probe_of(&context.into_value()));
                if !nested.replace(true) {
                    handle.set(Metadata::new((), Probe(2)));
                }
                delivering.set(false);
            }
        });

        handle.set(Metadata::new((), Probe(1)));

        assert_eq!(&*delivered.borrow(), &[1, 2]);
    }

    /// Realizing a clone of a connected `Dynamic` leaves the first receiver
    /// in place and the second realization empty.
    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "connected twice"))]
    fn a_second_connection_keeps_the_first_receiver() {
        let (handle, dynamic) = Dynamic::new();
        let first = Rc::new(Cell::new(0));
        let second = Rc::new(Cell::new(0));
        dynamic.clone().connect({
            let first = Rc::clone(&first);
            move |_| first.set(first.get() + 1)
        });
        dynamic.connect({
            let second = Rc::clone(&second);
            move |_| second.set(second.get() + 1)
        });

        handle.set(Metadata::new((), Probe(1)));

        assert_eq!((first.get(), second.get()), (1, 0));
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
            .downcast::<Metadata<Retain, Dynamic<Metadata<Probe, ()>>>>()
            .expect("the watched dynamic body is a retained Dynamic");
        let dynamic = content;
        dynamic.connect({
            let delivered = Rc::clone(&delivered);
            move |context| {
                delivered.borrow_mut().push(probe_of(&context.into_value()));
            }
        });

        source.set(2);

        assert_eq!(&*delivered.borrow(), &[1, 2, 3]);
        assert_eq!(source.snapshot(), 3);
    }

    /// A change that carries `NoAnimation` must reach the receiver with that
    /// metadata, so a structural transition can be suppressed.
    #[test]
    fn watch_forwards_change_metadata_to_the_receiver() {
        use crate::animation::NoAnimation;
        use nami::SignalExt as _;

        let source = Binding::i32(1);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let view = Dynamic::watch(source.with(NoAnimation), |value| {
            Metadata::new(
                (),
                Probe(usize::try_from(value).expect("watch values are non-negative")),
            )
        });
        let body = AnyView::new(view.body(&Environment::new()));
        let Metadata {
            content,
            value: _retained,
        } = *body
            .downcast::<Metadata<Retain, Dynamic<Metadata<Probe, ()>>>>()
            .expect("the watched dynamic body is a retained Dynamic");
        let dynamic = content;
        dynamic.connect({
            let seen = Rc::clone(&seen);
            move |context| {
                seen.borrow_mut()
                    .push(context.metadata().try_get::<NoAnimation>().is_some());
            }
        });

        source.set(2);

        // The initial snapshot carries only `DynamicInitialContent`; the
        // change made through `.with(NoAnimation)` carries `NoAnimation`.
        assert_eq!(&*seen.borrow(), &[false, true]);
    }
}
