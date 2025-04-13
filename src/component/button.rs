//! Button component for WaterUI
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
//! To learn more about `HandlerFn`, see the [HandlerFn] documentation.

use core::fmt::Debug;

use alloc::boxed::Box;
use waterui_core::configurable;
use waterui_core::handler::{ActionObject, HandlerFn, into_handler};

use crate::View;
use crate::{AnyView, ViewExt};

/// Configuration for a button component.
///
/// Use the `Button` struct's methods to customize these properties.
#[non_exhaustive]
#[derive(uniffi::Record)]
pub struct ButtonConfig {
    /// The label displayed on the button
    pub label: AnyView,
    /// The action to execute when the button is clicked
    pub action: ActionObject,
}

impl_debug!(ButtonConfig);

configurable!(Button, ButtonConfig);

impl Default for Button {
    fn default() -> Self {
        Self(ButtonConfig {
            label: ().anyview(),
            action: Box::new(into_handler(|| {})),
        })
    }
}

impl Button {
    /// Creates a new button with the specified label.
    ///
    /// # Arguments
    ///
    /// * `label` - The text or view to display on the button
    pub fn new(label: impl View) -> Self {
        let mut button = Self::default();
        button.0.label = label.anyview();
        button
    }

    /// Sets the action to be performed when the button is clicked.
    ///
    /// # Arguments
    ///
    /// * `action` - The callback function to execute when button is clicked
    ///
    /// # Returns
    ///
    /// The modified button with the action set
    pub fn action<P: 'static>(mut self, action: impl HandlerFn<P, ()>) -> Self {
        self.0.action = Box::new(into_handler(action));
        self
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
pub fn button(label: impl View) -> Button {
    Button::new(label)
}

macro_rules! bindgen {
    ($ffi_ty:ty,$($name:ident,$ty:ty),*) => {
        #[uniffi:export]
        impl $ffi_ty {
            $(
                pub fn $name(&self) -> $ty {
                    self.$name.take()
                }
            )*
        }
    };
}
