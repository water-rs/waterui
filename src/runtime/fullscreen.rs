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
/// This manager uses a `DynamicHandler` to control typed overlay content.
/// Handlers take it bare (`manager: FullScreenOverlayManager<V>`) once a window
/// or parent view has installed it through `.state(&manager)`.
#[state]
pub struct FullScreenOverlayManager<V: 'static = ()> {
    handler: DynamicHandler<Option<V>>,
}

impl<V: 'static> Clone for FullScreenOverlayManager<V> {
    fn clone(&self) -> Self {
        Self {
            handler: self.handler.clone(),
        }
    }
}

impl<V: 'static> Plugin for FullScreenOverlayManager<V> {}

impl<V: View> FullScreenOverlayManager<V> {
    /// Creates a new overlay manager and returns the overlay view.
    ///
    /// The returned view should be placed in a window overlay above the main content.
    ///
    /// # Returns
    ///
    /// A tuple containing the manager and the overlay view.
    pub fn new() -> (Self, impl View + Clone) {
        let (handler, dynamic) = Dynamic::<Option<V>>::new();
        handler.set(None);
        (Self { handler }, dynamic)
    }

    /// Shows an overlay view.
    ///
    /// The view will be displayed above all other content.
    pub fn show(&self, view: V) {
        self.handler.set(Some(view));
    }

    /// Hides the current overlay.
    ///
    /// This clears the overlay, making it transparent.
    pub fn hide(&self) {
        self.handler.set(None);
    }
}

impl<V: 'static> core::fmt::Debug for FullScreenOverlayManager<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FullScreenOverlayManager").finish()
    }
}
