//! Hotreload view component.

use super::event::{CliEvent, ConnectionError};
use super::{CliConnection, library, logging};
use crate::ViewExt;
use crate::prelude::*;
use futures::StreamExt;

/// A view wrapper that enables hot reloading.
///
/// Wraps an initial view and sets up the hot reload infrastructure:
/// - WebSocket connection to CLI
/// - Panic forwarding
/// - Log forwarding
/// - Status overlay
///
/// # Example
///
/// ```ignore
/// fn main() -> impl View {
///     Hotreload::new(app_content())
/// }
/// ```
#[derive(Debug)]
pub struct Hotreload<V> {
    initial: V,
    config: HotReloadConfig,
}

/// Default hot reload server port (must match CLI default).
pub const DEFAULT_HOT_RELOAD_PORT: u16 = 2006;

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

    /// Create config from compile-time environment variables.
    ///
    /// Reads `WATERUI_HOT_RELOAD_HOST` and `WATERUI_HOT_RELOAD_PORT` that are
    /// set by the CLI via `cargo:rustc-env` in the user project's build.rs.
    ///
    /// Falls back to defaults if not set:
    /// - Host: "127.0.0.1"
    /// - Port: 2006 (`DEFAULT_HOT_RELOAD_PORT`)
    #[must_use]
    pub fn from_compile_env() -> Self {
        let host = option_env!("WATERUI_HOT_RELOAD_HOST")
            .unwrap_or("127.0.0.1")
            .to_string();
        let port = option_env!("WATERUI_HOT_RELOAD_PORT")
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_HOT_RELOAD_PORT);
        Self { host, port }
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

    /// Create a new hot-reloadable view using compile-time config.
    ///
    /// This reads the hot reload configuration from environment variables
    /// that were set at compile time by the CLI.
    pub fn with_compile_env(initial: V) -> Self {
        Self {
            initial,
            config: HotReloadConfig::from_compile_env(),
        }
    }
}

impl<V: View> View for Hotreload<V> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        // Create dynamics for content and overlay
        let (content_handler, content_dynamic) = Dynamic::new();
        let (overlay_handler, overlay_dynamic) = Dynamic::new();

        content_handler.set(self.initial);

        logging::install_panic_forwarder();
        logging::install_tracing_forwarder();

        let (connection, outbound) = CliConnection::connect(self.config);

        logging::register_sender(outbound);

        // Show connecting overlay
        overlay_handler.set(StatusOverlay::connecting());

        executor_core::spawn_local(async move {
            run_event_loop(connection, content_handler, overlay_handler).await;
            logging::clear_sender();
        })
        .detach();

        overlay(content_dynamic, overlay_dynamic)
    }
}

/// Process events from CLI connection.
async fn run_event_loop(
    mut connection: CliConnection,
    content_handler: DynamicHandler,
    overlay_handler: DynamicHandler,
) {
    while let Some(event) = connection.next().await {
        match event {
            CliEvent::Connected => {
                overlay_handler.set(());
            }

            CliEvent::Disconnected => {
                // Will show reconnecting on next event
            }

            CliEvent::Reconnecting {
                attempt,
                max_attempts,
            } => {
                overlay_handler.set(StatusOverlay::reconnecting(attempt, max_attempts));
            }

            CliEvent::LibraryReady(path) => {
                let view = unsafe { library::load_view(&path) };
                content_handler.set(view);
            }

            CliEvent::LogFilterChanged(level) => {
                logging::set_log_level(&level);
            }

            CliEvent::Error(err) => {
                overlay_handler.set(StatusOverlay::error(err, overlay_handler.clone()));
                break;
            }
        }
    }
}

// ============================================================================
// Status Overlay Views
// ============================================================================

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
