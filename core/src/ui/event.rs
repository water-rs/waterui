//! Event handling components and utilities.
//!
//! This module provides two types of event handling:
//! - [`LifeCycleHook`] - One-time handlers for lifecycle events (appear/disappear)
//! - [`OnEvent`] - Repeatable handlers for interaction events (hover enter/move/exit)
//! - [`OnChange`] - Repeatable handlers for a signal's value changes

use crate::{
    Environment, LatestDispatch,
    handler::{
        BoxedAction, BoxedActionOnce, EventHandler, Handler, HandlerOnce, ReportExtraction,
        boxed_action, boxed_action_once, boxed_event_handler,
    },
    layout::Point,
    metadata::MetadataKey,
};
use alloc::{boxed::Box, rc::Rc};
use core::{cell::RefCell, fmt};
use nami::Signal;
use nami::watcher::BoxWatcherGuard;

/// Lifecycle events that occur once per view attachment/detachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LifeCycle {
    /// The event representing when a component appears (attached to view hierarchy).
    Appear,
    /// The event representing when a component disappears (detached from view hierarchy).
    Disappear,
}

/// A one-time lifecycle hook that triggers when a component appears or disappears.
///
/// This handler is consumed after being called once, suitable for lifecycle events
/// that only fire once per view attachment.
pub struct LifeCycleHook {
    lifecycle: LifeCycle,
    handler: BoxedActionOnce<()>,
}

impl MetadataKey for LifeCycleHook {}

impl LifeCycleHook {
    /// Creates a new lifecycle hook for the specified lifecycle event.
    ///
    /// # Arguments
    ///
    /// * `lifecycle` - The lifecycle event to listen for.
    /// * `handler` - The action to execute when the event occurs (called once).
    #[must_use]
    pub fn new<Args>(lifecycle: LifeCycle, handler: impl HandlerOnce<Args, ()>) -> Self {
        Self {
            lifecycle,
            handler: boxed_action_once(handler),
        }
    }

    /// Returns the lifecycle event associated with this hook.
    #[must_use]
    pub const fn lifecycle(&self) -> LifeCycle {
        self.lifecycle
    }

    /// Consumes the hook and returns the boxed handler.
    #[must_use = "the hook is consumed, so dropping the handler discards the callback"]
    pub fn into_handler(self) -> BoxedActionOnce<()> {
        self.handler
    }

    /// Handles the lifecycle event by invoking the stored handler.
    /// This consumes the hook since the handler is one-time.
    pub fn handle(self, env: &crate::Environment) {
        (self.handler)(env).report_extraction();
    }
}

impl fmt::Debug for LifeCycleHook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LifeCycleHook")
            .field("lifecycle", &self.lifecycle)
            .finish_non_exhaustive()
    }
}

/// Pointer hover event carrying the current local pointer location.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct HoverEvent {
    /// Current local pointer location in logical points.
    location: Point,
}

impl HoverEvent {
    /// Current local pointer location in logical points.
    #[must_use]
    pub const fn location(&self) -> Point {
        self.location
    }
}

impl HoverEvent {
    /// Creates a new hover event payload.
    #[must_use]
    pub const fn new(location: Point) -> Self {
        Self { location }
    }
}

/// Interaction events that can occur multiple times.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Event {
    /// The event representing when the cursor enters a component's bounds.
    HoverEnter,
    /// The event representing pointer motion within a component's bounds.
    HoverMove,
    /// The event representing when the cursor exits a component's bounds.
    HoverExit,
}

/// An event handler for repeatable interaction events.
///
/// This handler can be called multiple times, suitable for events like
/// hover enter/move/exit that may occur repeatedly during user interaction.
pub struct OnEvent {
    event: Event,
    handler: BoxedAction<()>,
}

impl MetadataKey for OnEvent {}

impl OnEvent {
    /// Creates a new event handler for the specified interaction event.
    ///
    /// # Arguments
    ///
    /// * `event` - The event to listen for.
    /// * `handler` - The action to execute when the event occurs (can be called multiple times).
    #[must_use]
    pub fn new<Args>(event: Event, handler: impl Handler<Args, ()>) -> Self {
        Self {
            event,
            handler: boxed_action(handler),
        }
    }
    /// Returns the event associated with this handler.
    #[must_use]
    pub const fn event(&self) -> Event {
        self.event
    }

    /// Handles the event by invoking the stored handler.
    pub fn handle(&mut self, env: &crate::Environment) {
        (self.handler)(env).report_extraction();
    }
}

impl fmt::Debug for OnEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OnEvent")
            .field("event", &self.event)
            .finish_non_exhaustive()
    }
}

/// A handler that runs whenever a signal's value changes.
///
/// The handler receives the new value first and any extractor arguments after
/// it — `State<T>`, `Environment`, … — resolved from the environment its
/// modified view resolves in, exactly as an [`OnEvent`] handler is: a
/// `.state(..)` install on either side of the handler in the same modifier
/// chain reaches it. A backend renders `Metadata<OnChange>` by resolving that
/// environment and calling [`subscribe`](Self::subscribe) with it, keeping the
/// returned guard for as long as the view is mounted.
///
/// Consecutive equal values are collapsed: the handler runs only when the
/// value differs from the last one it saw.
pub struct OnChange {
    subscribe: Box<dyn FnOnce(&Environment) -> BoxWatcherGuard>,
}

impl MetadataKey for OnChange {}

impl OnChange {
    /// Creates a handler that runs `handler` whenever `source`'s value
    /// changes.
    ///
    /// # Arguments
    ///
    /// * `source` - The signal to watch for changes.
    /// * `handler` - The [`EventHandler`] to run with each new value.
    #[must_use]
    pub fn new<C, H, A>(source: &C, handler: H) -> Self
    where
        C: Signal,
        C::Output: PartialEq + Clone,
        H: EventHandler<C::Output, A>,
    {
        let source = source.clone();
        let handler = boxed_event_handler(handler);
        Self {
            subscribe: Box::new(move |env: &Environment| {
                let env = env.clone();
                let handler = RefCell::new(handler);
                let cache = Rc::new(RefCell::new(None));
                // A write the handler is making right now lands here
                // re-entrantly; keep the latest value and let the dispatch in
                // flight drain it once the handler returns.
                let serial = LatestDispatch::new();
                let guard = source.watch({
                    let cache = Rc::clone(&cache);
                    move |context| {
                        serial.deliver(context.into_value(), |next| {
                            let changed = cache
                                .borrow_mut()
                                .replace(next.clone())
                                .is_none_or(|cached| cached != next);
                            if changed {
                                handler.borrow_mut()(next, &env).report_extraction();
                            }
                        });
                    }
                });
                if cache.borrow().is_none() {
                    cache.borrow_mut().replace(source.snapshot());
                }
                Box::new(guard)
            }),
        }
    }

    /// Subscribes the handler to its signal, resolving the handler's
    /// extractor arguments in `env`.
    ///
    /// Nothing runs before this call. The subscription lasts as long as the
    /// returned guard does.
    #[must_use = "dropping the guard ends the subscription"]
    pub fn subscribe(self, env: &Environment) -> BoxWatcherGuard {
        (self.subscribe)(env)
    }
}

impl fmt::Debug for OnChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OnChange").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::RefCell;

    use nami::{Binding, Signal, binding};

    use super::OnChange;
    use crate::Environment;
    use crate::extract::State;

    #[derive(Clone)]
    struct ChangesBeforeWatch {
        source: Binding<i32>,
        replacement: i32,
    }

    impl Signal for ChangesBeforeWatch {
        type Output = i32;
        type Guard = <Binding<i32> as Signal>::Guard;

        fn snapshot(&self) -> Self::Output {
            self.source.snapshot()
        }

        fn watch(
            &self,
            watcher: impl Fn(nami::watcher::Context<Self::Output>) + 'static,
        ) -> Self::Guard {
            self.source.set(self.replacement);
            self.source.watch(watcher)
        }
    }

    #[test]
    fn fires_on_first_update() {
        let source = Binding::i32(0);
        let seen = Rc::new(RefCell::new(Vec::new()));

        let _guard = OnChange::new(&source, {
            let seen = Rc::clone(&seen);
            move |value: i32| seen.borrow_mut().push(value)
        })
        .subscribe(&Environment::new());

        source.set(1);
        assert_eq!(&*seen.borrow(), &[1]);

        source.set(1);
        assert_eq!(&*seen.borrow(), &[1]);

        source.set(2);
        assert_eq!(&*seen.borrow(), &[1, 2]);
    }

    #[test]
    fn establishes_initial_cache_after_subscribing() {
        let source = Binding::i32(0);
        let changes_before_watch = ChangesBeforeWatch {
            source: source.clone(),
            replacement: 1,
        };
        let seen = Rc::new(RefCell::new(Vec::new()));

        let _guard = OnChange::new(&changes_before_watch, {
            let seen = Rc::clone(&seen);
            move |value: i32| seen.borrow_mut().push(value)
        })
        .subscribe(&Environment::new());

        source.set(1);
        source.set(2);

        assert_eq!(&*seen.borrow(), &[2]);
    }

    #[test]
    fn handler_extracts_injected_state() {
        let source = binding(0_i32);
        let total = Binding::i32(0);
        let mut env = Environment::new();
        env.insert(State(total.clone()));

        let _guard = OnChange::new(&source, |value: i32, State(total): State<Binding<i32>>| {
            total.add_assign(value);
        })
        .subscribe(&env);

        source.set(2);
        source.set(3);

        assert_eq!(total.snapshot(), 5);
    }

    /// A handler that writes its own watched binding must not panic on the
    /// nested notification: the write is delivered after the handler returns,
    /// and the handler stops once the value settles.
    #[test]
    fn handler_writing_watched_binding_runs_again_then_settles() {
        let source = Binding::i32(0);
        let seen = Rc::new(RefCell::new(Vec::new()));

        let _guard = OnChange::new(&source, {
            let seen = Rc::clone(&seen);
            let source = source.clone();
            move |value: i32| {
                seen.borrow_mut().push(value);
                if value == 1 {
                    source.set(7);
                } else if value == 7 {
                    // Rewriting the value the handler already saw must not
                    // run it again.
                    source.set(7);
                }
            }
        })
        .subscribe(&Environment::new());

        source.set(1);
        assert_eq!(&*seen.borrow(), &[1, 7]);
        assert_eq!(source.snapshot(), 7);
    }

    /// Each write the handler makes is delivered back to it, so a handler that
    /// keeps moving the value forward converges instead of panicking.
    #[test]
    fn self_writes_chain_until_the_value_settles() {
        let source = Binding::i32(0);
        let seen = Rc::new(RefCell::new(Vec::new()));

        let _guard = OnChange::new(&source, {
            let seen = Rc::clone(&seen);
            let source = source.clone();
            move |value: i32| {
                seen.borrow_mut().push(value);
                if value < 3 {
                    source.set(value + 1);
                }
            }
        })
        .subscribe(&Environment::new());

        source.set(1);
        assert_eq!(&*seen.borrow(), &[1, 2, 3]);
        assert_eq!(source.snapshot(), 3);
    }

    #[test]
    fn nothing_fires_before_subscribing() {
        let source = Binding::i32(0);
        let seen = Rc::new(RefCell::new(Vec::new()));

        let on_change = OnChange::new(&source, {
            let seen = Rc::clone(&seen);
            move |value: i32| seen.borrow_mut().push(value)
        });
        source.set(1);
        assert_eq!(*seen.borrow(), Vec::<i32>::new());

        let _guard = on_change.subscribe(&Environment::new());
        source.set(2);
        assert_eq!(&*seen.borrow(), &[2]);
    }
}
