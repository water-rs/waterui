//! Event handling components and utilities.
//!
//! This module provides two types of event handling:
//! - [`LifeCycleHook`] - One-time handlers for lifecycle events (appear/disappear)
//! - [`OnEvent`] - Repeatable handlers for interaction events (hover enter/move/exit)

use crate::{
    handler::{
        BoxedAction, BoxedActionOnce, boxed_action, boxed_action_once, boxed_action_with_env,
    },
    layout::Point,
    metadata::MetadataKey,
};

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
    pub fn new(lifecycle: LifeCycle, handler: impl FnOnce() + 'static) -> Self {
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
    #[must_use]
    pub fn into_handler(self) -> BoxedActionOnce<()> {
        self.handler
    }

    /// Handles the lifecycle event by invoking the stored handler.
    /// This consumes the hook since the handler is one-time.
    pub fn handle(self, env: &crate::Environment) {
        (self.handler)(env);
    }
}

impl core::fmt::Debug for LifeCycleHook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LifeCycleHook")
            .field("lifecycle", &self.lifecycle)
            .finish_non_exhaustive()
    }
}

/// Pointer hover event carrying the current local pointer location.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoverEvent {
    /// Current local pointer location in logical points.
    pub location: Point,
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
    pub fn new(event: Event, handler: impl FnMut() + 'static) -> Self {
        Self {
            event,
            handler: boxed_action(handler),
        }
    }

    /// Creates a new event handler that reads payload from the environment.
    #[must_use]
    pub fn new_with_env(
        event: Event,
        handler: impl FnMut(&crate::Environment) + 'static,
    ) -> Self {
        Self {
            event,
            handler: boxed_action_with_env(handler),
        }
    }

    /// Returns the event associated with this handler.
    #[must_use]
    pub const fn event(&self) -> Event {
        self.event
    }

    /// Handles the event by invoking the stored handler.
    pub fn handle(&mut self, env: &crate::Environment) {
        (self.handler)(env);
    }
}

impl core::fmt::Debug for OnEvent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("OnEvent")
            .field("event", &self.event)
            .finish_non_exhaustive()
    }
}
