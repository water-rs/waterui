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
pub mod ffi {
    use waterui_core::AnyView;
    use waterui_core::handler::BoxHandler;

    /// C representation of a WaterUI button for FFI purposes.
    #[derive(Debug)]
    #[repr(C)]
    pub struct WuiButton {
        /// Pointer to the button's label view
        pub label: *mut AnyView,
        /// Pointer to the button's action handler
        pub action: *mut BoxHandler<()>,
    }

    /// Creates a new button instance for FFI usage.
    ///
    /// # Safety
    ///
    /// Pointers must be valid and properly aligned.
    ///
    /// # Arguments
    ///
    /// * `label` - Pointer to the button's label view
    /// * `action` - Pointer to the button's action handler
    ///
    /// # Returns
    ///
    /// A new WuiButton instance
    #[unsafe(no_mangle)]
    extern "C" fn waterui_button_new(
        _label: *mut AnyView,
        _action: *mut BoxHandler<()>,
    ) -> WuiButton {
        todo!()
    }
}
