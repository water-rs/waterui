//! Event handling components and utilities.

use crate::{
    Metadata, View,
    handler::{BoxHandlerOnce, HandlerFnOnce, HandlerOnce, into_handler_once},
    metadata::MetadataKey,
};
use alloc::boxed::Box;
/// An enumeration of events that can occur within the UI framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Event {
    /// The event representing when a component appears.
    Appear,
    /// The event representing when a component disappears.
    Disappear,
}

/// An event handler that triggers a specified action when a certain event occurs.
#[derive(Debug)]
pub struct OnEvent {
    event: Event,
    handler: BoxHandlerOnce<()>,
}

impl MetadataKey for OnEvent {}

impl OnEvent {
    /// Creates a new `OnEvent` handler for the specified event and action.
    ///
    /// # Arguments
    ///
    /// * `event` - The event to listen for.
    /// * `handler` - The action to execute when the event occurs.
    #[must_use]
    pub fn new<H: 'static>(event: Event, handler: impl HandlerFnOnce<H, ()>) -> Self {
        Self {
            event,
            handler: Box::new(into_handler_once(handler)),
        }
    }

    /// Returns the event associated with this handler.
    #[must_use]
    pub const fn event(&self) -> Event {
        self.event
    }

    /// Consumes the `OnEvent` and returns the boxed handler.
    #[must_use]
    pub fn into_handler(self) -> BoxHandlerOnce<()> {
        self.handler
    }

    /// Handles the event by invoking the stored handler.
    pub fn handle(self, env: &crate::Environment) {
        (self.handler).handle(env);
    }
}

/// A value associated with an view, having a same lifecycle as the view.
#[derive(Debug)]
pub struct Associated<T, V> {
    content: V,
    value: T,
}

impl<T, V> Associated<T, V> {
    /// Creates a new `Associated` view that ties the lifecycle of the given value
    /// to the provided content view.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to associate with the view
    /// * `content` - The content view that will have the same lifecycle as the value
    #[must_use]
    pub const fn new(value: T, content: V) -> Self {
        Self { content, value }
    }
}

impl<T, V> View for Associated<T, V>
where
    T: 'static,
    V: View,
{
    fn body(self, _env: &crate::Environment) -> impl View {
        Metadata::new(
            self.content,
            OnEvent::new(Event::Disappear, move || drop(self.value)),
        )
    }
}
