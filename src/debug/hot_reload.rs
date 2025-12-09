//! Hotreload view component.

use super::event::ConnectionError;
use super::CliConnection;
use crate::ViewExt;
use crate::prelude::*;
use executor_core::spawn_local;

#[derive(Debug)]
pub struct Hotreload<V> {
    initial: V,
    config: HotReloadConfig,
}

#[derive(Debug, Clone)]
pub struct HotReloadConfig {
    host: String,
    port: u16,
}

impl HotReloadConfig {
    #[must_use]
    pub const fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }

    /// Create hot reload config from environment variables.
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

    #[must_use]
    pub const fn host(&self) -> &str {
        self.host.as_str()
    }

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
}

impl<V: View> View for Hotreload<V> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        // Create dynamics for content and overlay
        let (content_handler, content_dynamic) = Dynamic::new();
        let (overlay_handler, overlay_dynamic) = Dynamic::new();

        content_handler.set(self.initial);

        let connection = CliConnection::connect(self.config);
        let (_sender, _receiver) = connection.split();

        // Show connecting overlay
        overlay_handler.set(StatusOverlay::connecting());

        spawn_local(async move { todo!() }).detach();

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

    fn reconnecting(attempt: u32, max: u32) -> impl View {
        let msg = alloc::format!("Reconnecting... ({attempt}/{max})");
        vstack((text("Hot Reload"), text(msg)))
            .spacing(8.0)
            .padding_with(16.0)
            .background(Color::srgb_f32(0.1, 0.1, 0.1).with_opacity(0.9))
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
