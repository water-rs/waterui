//! Event handling components and utilities.
//!
//! This module provides two types of event handling:
//! - [`LifeCycleHook`] - One-time handlers for lifecycle events (appear/disappear)
//! - [`OnEvent`] - Repeatable handlers for interaction events (hover enter/exit)

use crate::{
    handler::{BoxHandler, BoxHandlerOnce, HandlerFn, HandlerFnOnce, into_handler, into_handler_once},
    metadata::MetadataKey,
};
use alloc::boxed::Box;

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
#[derive(Debug)]
pub struct LifeCycleHook {
    lifecycle: LifeCycle,
    handler: BoxHandlerOnce<()>,
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
    pub fn new<H: 'static>(lifecycle: LifeCycle, handler: impl HandlerFnOnce<H, ()> + 'static) -> Self {
        Self {
            lifecycle,
            handler: Box::new(into_handler_once(handler)),
        }
    }

    /// Returns the lifecycle event associated with this hook.
    #[must_use]
    pub const fn lifecycle(&self) -> LifeCycle {
        self.lifecycle
    }

    /// Consumes the hook and returns the boxed handler.
    #[must_use]
    pub fn into_handler(self) -> BoxHandlerOnce<()> {
        self.handler
    }

    /// Handles the lifecycle event by invoking the stored handler.
    /// This consumes the hook since the handler is one-time.
    pub fn handle(self, env: &crate::Environment) {
        self.handler.call_box(env);
    }
}

/// Interaction events that can occur multiple times.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Event {
    /// The event representing when the cursor enters a component's bounds.
    HoverEnter,
    /// The event representing when the cursor exits a component's bounds.
    HoverExit,
}

/// An event handler for repeatable interaction events.
///
/// This handler can be called multiple times, suitable for events like
/// hover enter/exit that may occur repeatedly during user interaction.
#[derive(Debug)]
pub struct OnEvent {
    event: Event,
    handler: BoxHandler<()>,
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
    pub fn new<H: 'static>(event: Event, handler: impl HandlerFn<H, ()> + 'static) -> Self {
        Self {
            event,
            handler: Box::new(into_handler(handler)),
        }
    }

    /// Returns the event associated with this handler.
    #[must_use]
    pub const fn event(&self) -> Event {
        self.event
    }

    /// Handles the event by invoking the stored handler.
    pub fn handle(&mut self, env: &crate::Environment) {
        self.handler.handle(env);
    }
}
