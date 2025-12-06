//! Hot reload session orchestration.
//!
//! This module provides `HotReloadSession`, which encapsulates the complete
//! hot reload lifecycle:
//! - File watching with debouncing
//! - Async builds with cancel-and-restart strategy
//! - Connection event monitoring
//! - Structured cancellation via `CancellationToken`
//!
//! ## Cancel-and-Restart Strategy
//!
//! When a file change is detected while a build is in progress, the session
//! cancels the current build immediately and starts a new one. This ensures:
//! - Fast feedback when saving multiple files in quick succession
//! - No wasted time completing a build that's already outdated
//! - Immediate response to Ctrl+C

use std::path::PathBuf;

use color_eyre::eyre::{Context, Result};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::build::{BuildOptions, BuildResult, Builder};

use super::{
    DisconnectReason, FileChanged, FileWatcher, NativeConnectionEvent,
    NativeConnectionEvents, Server,
};

/// Outcome of a hot reload session run.
#[derive(Debug)]
pub enum SessionOutcome {
    /// Session was cancelled (e.g., Ctrl+C)
    Cancelled,
    /// App disconnected from hot reload server
    AppDisconnected(DisconnectReason),
    /// Server shut down unexpectedly
    ServerShutdown,
}

/// A running hot reload session.
///
/// The session manages the lifecycle of file watching, building, and
/// notifying connected apps. It uses a cancel-and-restart strategy
/// for builds: when a new file change is detected while building,
/// the current build is cancelled and a new one starts immediately.
///
/// # Type Parameters
///
/// * `B` - The builder type used for compiling the project
///
/// # Example
///
/// ```ignore
/// let session = HotReloadSession::new(
///     server,
///     connection_events,
///     watcher,
///     builder,
/// );
///
/// match session.run(cancel).await {
///     Ok(SessionOutcome::Cancelled) => println!("User cancelled"),
///     Ok(SessionOutcome::AppDisconnected(reason)) => println!("App disconnected: {reason}"),
///     Ok(SessionOutcome::ServerShutdown) => println!("Server shutdown"),
///     Err(e) => eprintln!("Error: {e}"),
/// }
/// ```
#[derive(Debug)]
pub struct HotReloadSession<B: Builder> {
    server: Server,
    connection_events: NativeConnectionEvents,
    watcher: FileWatcher,
    builder: B,
    build_options: BuildOptions,
    /// Currently running build task (if any)
    current_build: Option<BuildInProgress>,
}

/// A build that is currently in progress.
struct BuildInProgress {
    /// Handle to the spawned build task
    handle: JoinHandle<Result<BuildResult>>,
    /// Token to cancel this specific build
    cancel: CancellationToken,
}

impl std::fmt::Debug for BuildInProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildInProgress")
            .field("is_finished", &self.handle.is_finished())
            .finish()
    }
}

impl<B: Builder + Clone + Send + 'static> HotReloadSession<B> {
    /// Create a new hot reload session.
    ///
    /// The session does not start running until `run()` is called.
    #[must_use]
    pub fn new(
        server: Server,
        connection_events: NativeConnectionEvents,
        watcher: FileWatcher,
        builder: B,
        build_options: BuildOptions,
    ) -> Self {
        Self {
            server,
            connection_events,
            watcher,
            builder,
            build_options,
            current_build: None,
        }
    }

    /// Get a reference to the server.
    #[must_use]
    pub const fn server(&self) -> &Server {
        &self.server
    }

    /// Run the hot reload session until cancelled or the app disconnects.
    ///
    /// This method:
    /// 1. Watches for file changes
    /// 2. Triggers rebuilds when changes are detected
    /// 3. Sends updated libraries to connected apps
    /// 4. Monitors connection state
    ///
    /// # Cancellation
    ///
    /// When the cancellation token is triggered:
    /// - Any in-progress build is cancelled
    /// - The session exits cleanly with `SessionOutcome::Cancelled`
    ///
    /// # Errors
    ///
    /// Returns an error if the file watcher or builder fails unexpectedly.
    pub async fn run(mut self, cancel: CancellationToken) -> Result<SessionOutcome> {
        info!(
            "Hot reload session started on {}",
            self.server.address()
        );

        loop {
            // Check connection events (non-blocking)
            if let Some(outcome) = self.check_connection_events() {
                return Ok(outcome);
            }

            // Poll the current build handle if we have one
            let build_result = if let Some(ref mut build) = self.current_build {
                let handle = &mut build.handle;
                tokio::select! {
                    biased;

                    // Global cancellation (Ctrl+C)
                    _ = cancel.cancelled() => {
                        self.cancel_current_build();
                        return Ok(SessionOutcome::Cancelled);
                    }

                    // File change detected
                    Some(FileChanged) = self.watcher.recv() => {
                        self.handle_file_change(&cancel);
                        continue;
                    }

                    // Build completed
                    result = handle => {
                        self.current_build = None;
                        match result {
                            Ok(r) => Some(r),
                            Err(join_error) => {
                                warn!("Build task panicked: {join_error}");
                                None
                            }
                        }
                    }
                }
            } else {
                // No build in progress, just wait for file changes or cancellation
                tokio::select! {
                    biased;

                    // Global cancellation (Ctrl+C)
                    _ = cancel.cancelled() => {
                        return Ok(SessionOutcome::Cancelled);
                    }

                    // File change detected
                    Some(FileChanged) = self.watcher.recv() => {
                        self.handle_file_change(&cancel);
                        continue;
                    }
                }
            };

            // Handle completed build result
            if let Some(result) = build_result {
                self.handle_build_result(result);
            }
        }
    }

    /// Check for connection events and return outcome if session should end.
    fn check_connection_events(&self) -> Option<SessionOutcome> {
        match self.connection_events.try_recv() {
            Ok(NativeConnectionEvent::Connected) => {
                debug!("App connected to hot reload server");
                None
            }
            Ok(NativeConnectionEvent::Disconnected(reason)) => {
                info!("App disconnected: {reason}");
                Some(SessionOutcome::AppDisconnected(reason))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                warn!("Hot reload connection event channel closed");
                Some(SessionOutcome::ServerShutdown)
            }
        }
    }

    /// Handle a file change event by starting a new build.
    fn handle_file_change(&mut self, parent_cancel: &CancellationToken) {
        info!("File change detected, triggering rebuild");

        // Cancel any in-progress build
        if self.current_build.is_some() {
            debug!("Cancelling in-progress build due to new file change");
            self.cancel_current_build();
        }

        // Start a new build
        let build_cancel = parent_cancel.child_token();
        let builder = self.builder.clone();
        let options = self.build_options.clone();

        let handle = tokio::spawn({
            let cancel = build_cancel.clone();
            async move { builder.build(&options, cancel).await }
        });

        self.current_build = Some(BuildInProgress {
            handle,
            cancel: build_cancel,
        });
    }

    /// Handle a completed build result.
    fn handle_build_result(&mut self, result: Result<BuildResult>) {
        match result {
            Ok(build_output) => {
                info!(
                    "Build succeeded: {} ({})",
                    build_output.artifact_path.display(),
                    build_output.target
                );
                self.server.notify_native_reload(build_output.artifact_path);
            }
            Err(e) if is_cancelled_error(&e) => {
                debug!("Build was cancelled");
            }
            Err(e) => {
                warn!("Build failed: {e}");
            }
        }
    }

    /// Cancel the current build if one is in progress.
    fn cancel_current_build(&mut self) {
        if let Some(build) = self.current_build.take() {
            build.cancel.cancel();
            // Don't await the handle - let it clean up in the background
        }
    }
}

/// Check if an error indicates the operation was cancelled.
fn is_cancelled_error(error: &color_eyre::eyre::Error) -> bool {
    let msg = error.to_string().to_lowercase();
    msg.contains("cancel") || msg.contains("interrupt")
}

/// Builder for constructing a `HotReloadSession` with all required components.
///
/// This provides a more ergonomic way to set up a hot reload session
/// when you have a project and device rather than raw components.
#[derive(Debug)]
pub struct HotReloadSessionBuilder {
    port: u16,
    static_path: Option<PathBuf>,
    watch_paths: Vec<PathBuf>,
    log_filter: Option<String>,
}

impl HotReloadSessionBuilder {
    /// Create a new session builder with the given port.
    #[must_use]
    pub const fn new(port: u16) -> Self {
        Self {
            port,
            static_path: None,
            watch_paths: Vec::new(),
            log_filter: None,
        }
    }

    /// Set the static file serving path (for web hot reload).
    #[must_use]
    pub fn static_path(mut self, path: PathBuf) -> Self {
        self.static_path = Some(path);
        self
    }

    /// Add paths to watch for changes.
    #[must_use]
    pub fn watch_paths(mut self, paths: Vec<PathBuf>) -> Self {
        self.watch_paths = paths;
        self
    }

    /// Set the log filter.
    #[must_use]
    pub fn log_filter(mut self, filter: Option<String>) -> Self {
        self.log_filter = filter;
        self
    }

    /// Build the session with the given builder and options.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The hot reload server fails to start
    /// - The file watcher fails to initialize
    pub fn build<B: Builder + Clone + Send + 'static>(
        self,
        builder: B,
        build_options: BuildOptions,
    ) -> Result<HotReloadSession<B>> {
        let static_path = self.static_path.unwrap_or_else(|| PathBuf::from("."));

        let (server, connection_events) =
            Server::start(self.port, static_path, self.log_filter)
                .map_err(|e| color_eyre::eyre::eyre!("{e}"))?;

        let watcher = FileWatcher::new(self.watch_paths)
            .context("failed to initialize file watcher")?;

        Ok(HotReloadSession::new(
            server,
            connection_events,
            watcher,
            builder,
            build_options,
        ))
    }
}
