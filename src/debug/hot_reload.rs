//! Hotreload view component.

use super::connection::CliEvent;
use super::event::ConnectionError;
use super::CliConnection;
use super::library;
use crate::ViewExt;
use crate::prelude::*;
use executor_core::spawn_local;

/// A view wrapper that enables hot reloading of the contained view.
#[derive(Debug)]
pub struct Hotreload<V> {
    initial: V,
    config: HotReloadConfig,
}

/// Configuration for connecting to the hot reload server.
#[derive(Debug, Clone)]
pub struct HotReloadConfig {
    host: String,
    port: u16,
}

impl HotReloadConfig {
    /// Create a new hot reload configuration.
    #[must_use]
    pub const fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }

    /// Create hot reload config from environment variables.
    ///
    /// Reads `WATERUI_HOT_RELOAD_HOST` and `WATERUI_HOT_RELOAD_PORT`.
    ///
    /// # Panics
    /// Panics if the environment variables are not set or invalid.
    #[must_use]
    pub fn from_env() -> Self {
        let host =
            std::env::var("WATERUI_HOT_RELOAD_HOST").expect("WATERUI_HOT_RELOAD_HOST not set");
        let port = std::env::var("WATERUI_HOT_RELOAD_PORT")
            .expect("WATERUI_HOT_RELOAD_PORT not set")
            .parse::<u16>()
            .expect("WATERUI_HOT_RELOAD_PORT is not a valid u16");
        Self::new(host, port)
    }

    /// Get the host address.
    #[must_use]
    pub fn host(&self) -> &str {
        self.host.as_str()
    }

    /// Get the port number.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }
}

impl<V: View> Hotreload<V> {
    /// Create a new hot-reloadable view with explicit config.
    pub const fn new(initial: V, config: HotReloadConfig) -> Self {
        Self { initial, config }
    }

    /// Create a new hot-reloadable view using environment variables for config.
    pub fn with_env(initial: V) -> Self {
        Self {
            initial,
            config: HotReloadConfig::from_env(),
        }
    }

    /// Create a hot-reloadable view, automatically detecting configuration from environment.
    ///
    /// This checks for `WATERUI_HOT_RELOAD_HOST` and `WATERUI_HOT_RELOAD_PORT` environment
    /// variables at runtime. If they are set (by the CLI's `water run` command), hot reload
    /// is enabled. Otherwise, the view is returned without hot reload capability.
    ///
    /// This is used by the FFI export macro to automatically inject hot reload in debug builds.
    #[cfg(all(not(target_arch = "wasm32"), debug_assertions))]
    pub fn try_from_env(initial: V) -> Self {
        // Check if hot reload is configured via environment variables (set by CLI)
        if std::env::var("WATERUI_HOT_RELOAD_HOST").is_ok() {
            Self::with_env(initial)
        } else {
            // No hot reload configured, just wrap the view with disabled config
            Self {
                initial,
                config: HotReloadConfig::new(String::new(), 0),
            }
        }
    }
}

impl<V: View> View for Hotreload<V> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        // Create dynamics for content and overlay
        let (content_handler, content_dynamic) = Dynamic::new();
        let (overlay_handler, overlay_dynamic) = Dynamic::new();

        content_handler.set(self.initial);

        // If no hot reload configured (port 0), just show the content without overlay
        if self.config.port == 0 {
            overlay_handler.set(());
        } else {
            // Show connecting overlay
            overlay_handler.set(StatusOverlay::connecting());

            // Spawn the hot reload connection task
            let config = self.config;
            spawn_local(async move {
                // Try to connect to the CLI
                let connection = match CliConnection::connect(config).await {
                    Ok(conn) => conn,
                    Err(e) => {
                        overlay_handler.set(StatusOverlay::error(e, overlay_handler.clone()));
                        return;
                    }
                };

                // Connected successfully, hide the overlay
                overlay_handler.set(());

                // Convert to receiver and start listening for events
                let mut receiver = connection.into_receiver();
                while let Some(event) = receiver.recv().await {
                    match event {
                        CliEvent::HotReload { binary } => {
                            // Create a temp file with the library data
                            let path = library::create_library(&binary).await;

                            // Load the new view from the library
                            // SAFETY: The library was just written by us and should be valid
                            let new_view = unsafe { library::load_view(&path) };

                            // Replace the content with the new view
                            content_handler.set(new_view);
                        }
                    }
                }

                // Connection closed
                overlay_handler.set(StatusOverlay::disconnected());
            })
            .detach();
        }

        overlay(content_dynamic, overlay_dynamic)
    }
}

struct StatusOverlay;

impl StatusOverlay {
    fn connecting() -> impl View {
        vstack((text("Hot Reload"), text("Connecting...")))
            .spacing(8.0)
            .padding_with(16.0)
            .background(Color::srgb_f32(0.1, 0.1, 0.1).with_opacity(0.9))
    }

    #[allow(dead_code)]
    fn reconnecting(attempt: u32, max: u32) -> impl View {
        let msg = alloc::format!("Reconnecting... ({attempt}/{max})");
        vstack((text("Hot Reload"), text(msg)))
            .spacing(8.0)
            .padding_with(16.0)
            .background(Color::srgb_f32(0.1, 0.1, 0.1).with_opacity(0.9))
    }

    fn disconnected() -> impl View {
        vstack((text("Hot Reload"), text("Disconnected")))
            .spacing(8.0)
            .padding_with(16.0)
            .background(Color::srgb_f32(0.5, 0.3, 0.1).with_opacity(0.9))
    }

    fn error(err: ConnectionError, overlay_handler: DynamicHandler) -> impl View {
        let msg = alloc::format!("{err}");

        vstack((
            text("Hot Reload Error"),
            text(msg),
            button(text("Dismiss")).action(move || {
                overlay_handler.set(()); // clear overlay
            }),
        ))
        .spacing(12.0)
        .padding_with(16.0)
        .background(Color::srgb_f32(0.6, 0.1, 0.1).with_opacity(0.95))
    }
}
