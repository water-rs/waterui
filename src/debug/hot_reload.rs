//! Hotreload view component.
//!
//! This module provides hot reload functionality for WaterUI applications.
//! It uses the `FullScreenOverlayManager` for showing status overlays.
//!
//! # Per-Function Hot Reload
//!
//! The `#[hot_reload]` attribute macro enables hot reloading of individual view functions:
//!
//! ```ignore
//! #[hot_reload]
//! fn sidebar() -> impl View {
//!     vstack((text("Sidebar"), ...))
//! }
//! ```
//!
//! Each hot-reloadable function registers itself with a global registry. When the
//! CLI rebuilds the library, all registered handlers are updated with new views.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use super::CliConnection;
use super::connection::CliEvent;
use super::event::ConnectionError;
use super::library::{self, HotReloadLibrary};
use crate::ViewExt;
use crate::overlay::FullScreenOverlayManager;
use crate::prelude::*;
use executor_core::spawn_local;
use std::cell::Cell;

thread_local! {
    /// Thread-local state for hot reload system.
    /// Since hot reload runs on the main UI thread, we use thread_local storage.
    static HOT_RELOAD_STATE: RefCell<HotReloadState> = RefCell::new(HotReloadState::new());

    /// Stores the environment pointer from native for hot reload access.
    /// This is set by `waterui_app` and used by `load_view` during hot reload.
    static ENV_PTR: Cell<*mut ()> = const { Cell::new(core::ptr::null_mut()) };
}

/// Thread-local state for the hot reload system.
struct HotReloadState {
    /// Map of function IDs to their update handlers.
    handlers: BTreeMap<String, Vec<DynamicHandler>>,
    /// Overlay manager for showing status (captured from first registration).
    overlay_manager: Option<FullScreenOverlayManager>,
    /// Whether the connection has been started.
    connection_started: bool,
}

impl HotReloadState {
    const fn new() -> Self {
        Self {
            handlers: BTreeMap::new(),
            overlay_manager: None,
            connection_started: false,
        }
    }
}

/// Store the environment pointer for hot reload access.
///
/// # Safety
/// The pointer must remain valid for the lifetime of the app.
pub fn set_env_ptr(ptr: *mut ()) {
    ENV_PTR.set(ptr);
}

/// Get the stored environment pointer.
pub fn get_env_ptr() -> *mut () {
    ENV_PTR.get()
}

/// Register a hot reload handler for a specific function ID.
///
/// This is called by `HotReloadView` when it's created to register its handler
/// with the global registry.
fn register_handler(id: &str, handler: DynamicHandler, overlay_manager: Option<FullScreenOverlayManager>) {
    HOT_RELOAD_STATE.with(|state| {
        let mut state = state.borrow_mut();

        // Register the handler
        state
            .handlers
            .entry(id.to_string())
            .or_default()
            .push(handler);

        // Store overlay manager if not already set
        if state.overlay_manager.is_none() {
            if let Some(mgr) = overlay_manager {
                state.overlay_manager = Some(mgr);
            }
        }
    });
}

/// Start the hot reload connection if not already started.
fn start_connection_if_needed(config: HotReloadConfig) {
    let should_start = HOT_RELOAD_STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.connection_started {
            return false;
        }
        state.connection_started = true;
        true
    });

    if !should_start {
        return;
    }

    // Get overlay manager for status display
    let overlay_manager =
        HOT_RELOAD_STATE.with(|state| state.borrow().overlay_manager.clone());

    // Spawn the connection task
    spawn_local(async move {
        run_hot_reload_loop(config, overlay_manager).await;
    })
    .detach();
}

/// Main hot reload loop that handles connection and library updates.
async fn run_hot_reload_loop(config: HotReloadConfig, overlay_manager: Option<FullScreenOverlayManager>) {
    // Show connecting overlay
    if let Some(ref mgr) = overlay_manager {
        mgr.show(StatusOverlay::connecting());
    }

    // Try to connect to the CLI
    let connection = match CliConnection::connect(config).await {
        Ok(conn) => conn,
        Err(e) => {
            tracing::error!("Hot reload connection failed: {e}");
            if let Some(ref mgr) = overlay_manager {
                mgr.show(StatusOverlay::error(&e, mgr.clone()));
            }
            return;
        }
    };

    // Connected successfully, hide the overlay
    if let Some(ref mgr) = overlay_manager {
        mgr.hide();
    }

    // Convert to receiver and start listening for events
    let mut receiver = connection.into_receiver();
    while let Some(event) = receiver.recv().await {
        match event {
            CliEvent::Building => {
                // Show reloading overlay immediately for instant feedback
                if let Some(ref mgr) = overlay_manager {
                    mgr.show(StatusOverlay::reloading());
                }
            }
            CliEvent::HotReload { binary } => {
                // Create a temp file with the library data
                let path = library::create_library(&binary).await;

                // Load the library
                let lib = match unsafe { HotReloadLibrary::load(&path) } {
                    Ok(lib) => lib,
                    Err(e) => {
                        tracing::error!("Failed to load hot reload library: {e}");
                        if let Some(ref mgr) = overlay_manager {
                            mgr.show(StatusOverlay::error_message(
                                alloc::format!("{e}"),
                                mgr.clone(),
                            ));
                        }
                        continue;
                    }
                };

                // Update all registered handlers
                HOT_RELOAD_STATE.with(|state| {
                    let state = state.borrow();

                    for (id, handler_list) in state.handlers.iter() {
                        // Extract function name from ID (e.g., "my_crate::sidebar" -> "sidebar")
                        let fn_name = id.rsplit("::").next().unwrap_or(id);
                        let symbol = format!("waterui_hot_reload_{fn_name}\0");

                        if lib.has_symbol(&symbol) {
                            // Load a view for each handler separately
                            for handler in handler_list {
                                match unsafe { lib.load_symbol(&symbol) } {
                                    Ok(view) => {
                                        handler.set(view);
                                    }
                                    Err(e) => {
                                        tracing::warn!("Failed to load symbol {symbol}: {e}");
                                    }
                                }
                            }
                        }
                    }

                    // Also try to load the legacy main symbol for backward compatibility
                    let main_symbol = "waterui_hot_reload_main\0";
                    if lib.has_symbol(main_symbol) {
                        if let Some(main_handlers) = state.handlers.get("main") {
                            for handler in main_handlers {
                                if let Ok(view) = unsafe { lib.load_symbol(main_symbol) } {
                                    handler.set(view);
                                }
                            }
                        }
                    }
                });

                // Hide the overlay after successful reload
                if let Some(ref mgr) = overlay_manager {
                    mgr.hide();
                }
            }
        }
    }

    // Connection closed
    tracing::warn!("Hot reload connection lost");
    if let Some(ref mgr) = overlay_manager {
        mgr.show(StatusOverlay::disconnected());
    }
}

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
    pub const fn host(&self) -> &str {
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
    #[allow(unused_variables)]
    pub fn new(initial: V, config: HotReloadConfig, env: &Environment) -> Self {
        Self { initial, config }
    }

    /// Create a new hot-reloadable view using environment variables for config.
    #[allow(unused_variables)]
    pub fn with_env(initial: V, env: &Environment) -> Self {
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
    #[allow(unused_variables)]
    pub fn try_from_env(initial: V, env: &Environment) -> Self {
        // Check if hot reload is configured via environment variables (set by CLI)
        if std::env::var("WATERUI_HOT_RELOAD_HOST").is_ok() {
            Self::with_env(initial, env)
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
    fn body(self, env: &waterui_core::Environment) -> impl View {
        // Create dynamic handler for content updates
        let (content_handler, content_dynamic) = Dynamic::new();
        content_handler.set(self.initial);

        // If no hot reload configured (port 0), just show the content
        if self.config.port == 0 {
            return content_dynamic;
        }

        // Get the overlay manager from the environment (injected by App::new)
        let overlay_manager = env.get::<FullScreenOverlayManager>().cloned();

        // Show connecting overlay
        if let Some(ref mgr) = overlay_manager {
            mgr.show(StatusOverlay::connecting());
        }

        // Spawn the hot reload connection task
        let config = self.config;
        spawn_local(async move {
            // Try to connect to the CLI
            let connection = match CliConnection::connect(config).await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::error!("Hot reload connection failed: {e}");
                    if let Some(ref mgr) = overlay_manager {
                        mgr.show(StatusOverlay::error(&e, mgr.clone()));
                    }
                    return;
                }
            };

            // Connected successfully, hide the overlay
            if let Some(ref mgr) = overlay_manager {
                mgr.hide();
            }

            // Convert to receiver and start listening for events
            let mut receiver = connection.into_receiver();
            while let Some(event) = receiver.recv().await {
                match event {
                    CliEvent::Building => {
                        // Show reloading overlay immediately for instant feedback
                        if let Some(ref mgr) = overlay_manager {
                            mgr.show(StatusOverlay::reloading());
                        }
                    }
                    CliEvent::HotReload { binary } => {
                        // Create a temp file with the library data
                        let path = library::create_library(&binary).await;

                        // Load the new view from the library
                        // SAFETY: The library was just written by us and should be valid
                        let new_view = match unsafe { library::load_view(&path) } {
                            Ok(view) => view,
                            Err(e) => {
                                tracing::error!("Failed to load hot reload library: {e}");
                                if let Some(ref mgr) = overlay_manager {
                                    mgr.show(StatusOverlay::error_message(
                                        alloc::format!("{e}"),
                                        mgr.clone(),
                                    ));
                                }
                                continue;
                            }
                        };

                        // Replace the content with the new view
                        content_handler.set(new_view);

                        // Hide the overlay after successful reload
                        if let Some(ref mgr) = overlay_manager {
                            mgr.hide();
                        }
                    }
                }
            }

            // Connection closed
            tracing::warn!("Hot reload connection lost");
            if let Some(ref mgr) = overlay_manager {
                mgr.show(StatusOverlay::disconnected());
            }
        })
        .detach();

        content_dynamic
    }
}

struct StatusOverlay;

impl StatusOverlay {
    fn connecting() -> impl View {
        Self::overlay_container(vstack((text("Hot Reload"), text("Connecting..."))).spacing(8.0))
    }

    fn reloading() -> impl View {
        Self::overlay_container(
            vstack((text("Hot Reload"), loading(), text("Reloading..."))).spacing(8.0),
        )
    }

    #[allow(dead_code)]
    fn reconnecting(attempt: u32, max: u32) -> impl View {
        let msg = alloc::format!("Reconnecting... ({attempt}/{max})");
        Self::overlay_container(vstack((text("Hot Reload"), text(msg))).spacing(8.0))
    }

    fn disconnected() -> impl View {
        vstack((text("Hot Reload"), text("Disconnected")))
            .spacing(8.0)
            .padding_with(16.0)
            .background(Color::srgb_f32(0.5, 0.3, 0.1).with_opacity(0.9))
    }

    fn error(err: &ConnectionError, overlay_manager: FullScreenOverlayManager) -> impl View {
        Self::error_message(alloc::format!("{err}"), overlay_manager)
    }

    fn error_message(msg: String, overlay_manager: FullScreenOverlayManager) -> impl View {
        vstack((
            text("Hot Reload Error"),
            text(msg),
            button(text("Dismiss")).action(move || {
                overlay_manager.hide();
            }),
        ))
        .spacing(12.0)
        .padding_with(16.0)
        .background(Color::srgb_f32(0.6, 0.1, 0.1).with_opacity(0.95))
    }

    /// Common container for status overlays - centered with semi-transparent background
    fn overlay_container(content: impl View) -> impl View {
        zstack((
            // Semi-transparent background covering entire screen
            spacer().background(Color::srgb_f32(0.0, 0.0, 0.0).with_opacity(0.4)),
            // Centered status card
            content
                .padding_with(16.0)
                .background(Color::srgb_f32(0.1, 0.1, 0.1).with_opacity(0.95)),
        ))
    }
}

// =============================================================================
// Per-Function Hot Reload
// =============================================================================

/// A view component that enables hot reloading of a single view function.
///
/// This is used by the `#[hot_reload]` attribute macro to wrap view functions.
/// Each `HotReloadView` registers itself with the global hot reload system and
/// receives updates when the library is rebuilt.
///
/// # Example
///
/// The `#[hot_reload]` macro generates code like:
///
/// ```ignore
/// fn sidebar() -> impl View {
///     HotReloadView::new(
///         "my_crate::sidebar",
///         || { vstack((text("Sidebar"), ...)) }
///     )
/// }
/// ```
pub struct HotReloadView<F> {
    /// Unique identifier for this hot-reloadable function (e.g., "my_crate::sidebar").
    id: &'static str,
    /// Builder function that creates the initial view.
    builder: F,
}

impl<F> HotReloadView<F> {
    /// Create a new hot-reloadable view.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for this function (typically `module_path!() + "::" + fn_name`)
    /// * `builder` - Closure that builds the initial view
    #[must_use]
    pub const fn new(id: &'static str, builder: F) -> Self {
        Self { id, builder }
    }
}

impl<F, V> View for HotReloadView<F>
where
    F: FnOnce() -> V + 'static,
    V: View,
{
    fn body(self, env: &waterui_core::Environment) -> impl View {
        // Check if hot reload is enabled via environment variables
        let config = if std::env::var("WATERUI_HOT_RELOAD_HOST").is_ok() {
            Some(HotReloadConfig::from_env())
        } else {
            None
        };

        // Create dynamic handler for content updates
        let (content_handler, content_dynamic) = Dynamic::new();

        // Set initial content from builder
        content_handler.set((self.builder)());

        // If hot reload is not configured, just return the content
        let Some(config) = config else {
            return content_dynamic;
        };

        // Get overlay manager from environment
        let overlay_manager = env.get::<FullScreenOverlayManager>().cloned();

        // Register this handler with the global registry
        register_handler(self.id, content_handler, overlay_manager);

        // Start the connection if not already started
        start_connection_if_needed(config);

        content_dynamic
    }
}

impl<F> core::fmt::Debug for HotReloadView<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HotReloadView")
            .field("id", &self.id)
            .finish()
    }
}
