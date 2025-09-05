//! Button component for `WaterUI`
//!
//! This module provides a Button component that allows users to trigger actions
//! when clicked.
//!
//! # Examples
//!
//! ```
//! use waterui::button;
//!
//! let button = button("Click me").action(|| {
//!     println!("Button clicked!");
//! });
//! ```
//!
//! Tip: `action` receives a `HandlerFn`, it can extract value from environment and pass it to the action.
//! To learn more about `HandlerFn`, see the [`HandlerFn`] documentation.

use core::fmt::Debug;

use alloc::boxed::Box;
use waterui_core::Environment;
use waterui_core::components::Native;
use waterui_core::handler::{
    ActionObject, Handler, HandlerFn, HandlerFnWithState, IntoHandler, IntoHandlerWithState,
    into_handler, into_handler_with_state,
};
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};

use crate::View;
use crate::{AnyView, ViewExt};

/// Configuration for a button component.
///
/// Use the `Button` struct's methods to customize these properties.
#[non_exhaustive]
pub struct ButtonConfig {
    /// The label displayed on the button
    pub label: AnyView,
    /// The action to execute when the button is clicked
    pub action: ActionObject,
}

impl_debug!(ButtonConfig);

impl<Label, Action> View for Button<Label, Action>
where
    Label: View,
    Action: Handler<()>,
{
    fn body(self, env: &Environment) -> impl View {
        let config = self.config();
        if let Some(hook) = env.get::<Hook<ButtonConfig>>() {
            hook.apply(env, config)
        } else {
            Native(config).anyview()
        }
    }
}

impl ViewConfiguration for ButtonConfig {
    type View = Button<AnyView, ActionObject>;

    fn render(self) -> Self::View {
        Button {
            label: self.label,
            action: self.action,
        }
    }
}

impl<Label, Action> ConfigurableView for Button<Label, Action>
where
    Label: View,
    Action: Handler<()>,
{
    type Config = ButtonConfig;

    fn config(self) -> Self::Config {
        ButtonConfig {
            label: AnyView::new(self.label),
            action: Box::new(self.action),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Button<Label, Action> {
    label: Label,
    action: Action,
}

impl<Label> Button<Label, ()> {
    /// Creates a new button with the specified label.
    ///
    /// # Arguments
    ///
    /// * `label` - The text or view to display on the button
    pub fn new(label: Label) -> Self {
        Self { label, action: () }
    }
}

impl<Label, Action> Button<Label, Action> {
    /// Sets the action to be performed when the button is clicked.
    ///
    /// # Arguments
    ///
    /// * `action` - The callback function to execute when button is clicked
    ///
    /// # Returns
    ///
    /// The modified button with the action set
    #[must_use]
    pub fn action<H, P>(self, action: H) -> Button<Label, IntoHandler<H, P, ()>>
    where
        H: HandlerFn<P, ()>,
        P: 'static,
    {
        Button {
            label: self.label,
            action: into_handler(action),
        }
    }

    #[must_use]
    pub fn action_with<H, P, S>(
        self,
        state: &S,
        action: H,
    ) -> Button<Label, IntoHandlerWithState<H, P, (), S>>
    where
        H: HandlerFnWithState<P, (), S>,
        S: 'static + Clone,
        P: 'static,
    {
        into_handler_with_state(action, state.clone())
    }
}

/// Convenience function to create a new button with the specified label.
///
/// # Arguments
///
/// * `label` - The text or view to display on the button
///
/// # Returns
///
/// A new button instance
pub fn button<Label>(label: Label) -> Button<Label, ()> {
    Button::new(label)
}
