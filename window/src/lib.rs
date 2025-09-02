//! Window management utilities for `WaterUI` applications.
//!
//! This crate provides window management abstractions that work across different
//! platforms and rendering backends. It includes window creation, management,
//! and dialog handling capabilities.

/// Dialog utilities for modal interactions.
pub mod dialog;

use nami::Computed;
use waterui_core::{AnyView, Str};

/// Represents a window with configurable properties.
///
/// A `Window` contains all the information needed to create and manage
/// a window, including its title, dimensions, and content.
#[derive(Debug)]
#[allow(dead_code)] // These will be used by window managers
pub struct Window {
    /// The window's title.
    title: Computed<Str>,
    /// The window's width in pixels.
    width: Computed<u32>,
    /// The window's height in pixels.
    height: Computed<u32>,
    /// The content to display in the window.
    content: AnyView,
}

/// Trait for handling window lifecycle events.
///
/// Implementors of this trait can respond to window events such as closing.
pub trait WindowHandler {
    /// Closes the window.
    fn close(self);
}

/// Trait for managing multiple windows.
///
/// A `WindowManager` is responsible for creating and managing windows
/// across different platforms.
pub trait WindowManager {
    /// Opens a new window with the specified configuration.
    ///
    /// # Arguments
    ///
    /// * `window` - The window configuration to use.
    ///
    /// # Returns
    ///
    /// A window handler that can be used to control the window.
    fn open(&self, window: Window) -> impl WindowHandler;
}

trait WindowHandlerImpl {
    fn close(self: Box<Self>);
}

/// Type-erased window handler that can store any window handler implementation.
pub struct AnyWindowHandler(Box<dyn WindowHandlerImpl>);

impl std::fmt::Debug for AnyWindowHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnyWindowHandler").finish_non_exhaustive()
    }
}

impl WindowHandler for AnyWindowHandler {
    fn close(self) {
        self.0.close();
    }
}

trait WindowManagerImpl {
    fn open(&self, window: Window) -> AnyWindowHandler;
}

/// Type-erased window manager that can store any window manager implementation.
pub struct AnyWindowManager(Box<dyn WindowManagerImpl>);

impl std::fmt::Debug for AnyWindowManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnyWindowManager").finish_non_exhaustive()
    }
}

impl WindowManager for AnyWindowManager {
    fn open(&self, window: Window) -> impl WindowHandler {
        self.0.open(window)
    }
}
