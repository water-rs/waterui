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
use waterui_core::handler::{BoxHandler, HandlerFn, into_handler};

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
    pub action: BoxHandler<()>,
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

/// FFI bindings for button component integration with native platforms.
pub(crate) mod ffi {
    use super::{Button, ButtonConfig};
    use waterui_core::components::Native;
    use waterui_core::handler::BoxHandler;
    use waterui_core::{AnyView, ffi_view};
    use waterui_ffi::ffi_struct;

    /// C representation of a WaterUI button for FFI purposes.
    #[derive(Debug)]
    #[repr(C)]
    pub struct WuiButton {
        /// Pointer to the button's label view
        pub label: *mut AnyView,
        /// Pointer to the button's action handler
        pub action: *mut BoxHandler<()>,
    }

    ffi_struct!(ButtonConfig, WuiButton, label, action);
    ffi_view!(
        Native<ButtonConfig>,
        WuiButton,
        waterui_button_id,
        waterui_force_as_button
    );
}
