//! Full-screen overlay system for `WaterUI`.
//!
//! This module provides a centralized overlay manager that can display
//! full-screen overlays such as modal dialogs, loading screens, and
//! toast notifications.

use crate::{View, ViewExt};
use core::marker::PhantomData;
use waterui_core::Environment;
use waterui_core::dynamic::{Dynamic, DynamicHandler};
use waterui_core::plugin::Plugin;
use waterui_macros::state;

/// Manages full-screen overlays for the application.
///
/// This manager uses a `DynamicHandler` to control typed overlay content.
/// Handlers take it bare (`manager: FullScreenOverlayManager<V>`) once an
/// ancestor view has attached [`ViewExt::full_screen_overlay_host`] or
/// installed it through `.overlay(overlay_view).state(&manager)`.
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

    /// Builds a view subtree paired with a typed overlay layer, inferring `V`
    /// from how `build` uses the handed manager and installing it for descendants.
    pub fn host<R: View>(build: impl FnOnce(Self) -> R) -> impl View {
        let (manager, overlay) = Self::new();
        build(manager.clone()).overlay(overlay).state(&manager)
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

/// Hosts a [`FullScreenOverlayManager<O>`] and its overlay for a view subtree.
///
/// Attach with [`ViewExt::full_screen_overlay_host`]. Each built instance
/// creates its own manager and overlay view, so multiple windows or scenes stay
/// isolated.
#[derive(Debug)]
pub struct FullScreenOverlayHost<V, O> {
    content: V,
    _overlay: PhantomData<fn() -> O>,
}

impl<V, O> FullScreenOverlayHost<V, O> {
    /// Wraps `content` in a full-screen overlay host for overlay views of type `O`.
    #[must_use]
    pub const fn new(content: V) -> Self {
        Self {
            content,
            _overlay: PhantomData,
        }
    }
}

impl<V: View, O: View> View for FullScreenOverlayHost<V, O> {
    fn body(self, _env: &Environment) -> impl View {
        let (manager, overlay) = FullScreenOverlayManager::<O>::new();
        self.content.overlay(overlay).state(&manager)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inferred_host_builds_without_turbofish() {
        let view = FullScreenOverlayManager::host(|overlay| {
            overlay.show("Modal");
            "Content"
        });
        let _ = view.body(&Environment::new());
    }
}
