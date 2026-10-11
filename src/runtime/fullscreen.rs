//! Full-screen overlay system for `WaterUI`.
//!
//! This module provides a centralized overlay manager that can display
//! full-screen overlays such as modal dialogs, loading screens, and
//! toast notifications.

use crate::View;
use waterui_core::dynamic::{Dynamic, DynamicHandler};
use waterui_core::plugin::Plugin;
use waterui_macros::state;

/// Manages full-screen overlays for the application.
///
/// This manager uses a `DynamicHandler` to control overlay content.
/// Handlers take it bare (`manager: FullScreenOverlayManager`) once a window
/// has installed it through `.state(&manager)`.
#[state]
#[derive(Clone)]
pub struct FullScreenOverlayManager {
    handler: DynamicHandler,
}

impl Plugin for FullScreenOverlayManager {}

impl FullScreenOverlayManager {
    /// Creates a new overlay manager and returns the overlay view.
    ///
    /// The returned view should be placed in a window overlay above the main content.
    ///
    /// # Returns
    ///
    /// A tuple containing the manager and the overlay view.
    pub fn new() -> (Self, impl View + Clone) {
        let (handler, dynamic) = Dynamic::new();
        handler.set(()); // Initially empty
        (Self { handler }, dynamic)
    }

    /// Shows an overlay view.
    ///
    /// The view will be displayed above all other content.
    pub fn show(&self, view: impl View) {
        self.handler.set(view);
    }

    /// Hides the current overlay.
    ///
    /// This clears the overlay, making it transparent.
    pub fn hide(&self) {
        self.handler.set(());
    }
}

impl core::fmt::Debug for FullScreenOverlayManager {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FullScreenOverlayManager").finish()
    }
}
