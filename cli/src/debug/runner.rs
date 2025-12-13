//! Hot reload runner that orchestrates file watching, building, and broadcasting.

use std::path::PathBuf;

use futures::FutureExt;
use smol::Task;
use smol::channel::{self, Receiver, Sender};
use target_lexicon::Triple;

use super::file_watcher::FileWatcher;
use super::hot_reload::{BuildManager, DEFAULT_PORT, HotReloadServer};
use crate::build::RustBuild;
use crate::project::Project;

/// Events emitted by the hot reload runner.
#[derive(Debug, Clone)]
pub enum HotReloadEvent {
    /// Server started and listening.
    ServerStarted {
        /// Host address the server is bound to.
        host: String,
        /// Port the server is listening on.
        port: u16,
    },
    /// File change detected, waiting for debounce.
    FileChanged,
    /// Starting a rebuild.
    Rebuilding,
    /// Build completed successfully, broadcasting to clients.
    Built {
        /// Path to the built dylib.
        path: PathBuf,
    },
    /// Build failed with an error message.
    BuildFailed {
        /// Error message.
        error: String,
    },
    /// Library broadcast to connected clients.
    Broadcast,
}

/// Orchestrates hot reload: file watching, building, and broadcasting.
pub struct HotReloadRunner {
    server: HotReloadServer,
    event_rx: Receiver<HotReloadEvent>,
    _runner_task: Task<()>,
}

impl std::fmt::Debug for HotReloadRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HotReloadRunner")
            .field("server", &self.server)
            .finish_non_exhaustive()
    }
}

impl HotReloadRunner {
    /// Create a new hot reload runner for the given project.
    ///
    /// # Arguments
    /// * `project` - The project to watch and rebuild
    /// * `triple` - The target triple to build for
    ///
    /// # Errors
    /// Returns an error if the server or file watcher cannot be started.
    pub async fn new(project: &Project, triple: Triple) -> color_eyre::Result<Self> {
        let server = HotReloadServer::launch(DEFAULT_PORT).await?;
        let watcher = FileWatcher::new(project.root())?;

        let (event_tx, event_rx) = channel::unbounded();

        // Send initial server started event
        let _ = event_tx
            .send(HotReloadEvent::ServerStarted {
                host: server.host(),
                port: server.port(),
            })
            .await;

        let rust_build = RustBuild::new(project.root(), triple, true);
        let file_rx = watcher.receiver().clone();
        let broadcast_tx = server.broadcast_sender();
        let crate_name = project.crate_name().replace('-', "_");

        // Spawn the runner task
        let runner_task = smol::spawn(run_loop(
            rust_build,
            file_rx,
            broadcast_tx,
            event_tx,
            watcher,
            crate_name,
        ));

        Ok(Self {
            server,
            event_rx,
            _runner_task: runner_task,
        })
    }

    /// Get the host address for the hot reload server.
    #[must_use]
    pub fn host(&self) -> String {
        self.server.host()
    }

    /// Get the port the hot reload server is listening on.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.server.port()
    }

    /// Get the event receiver for hot reload events.
    #[must_use] 
    pub const fn events(&self) -> &Receiver<HotReloadEvent> {
        &self.event_rx
    }

    /// Consume the runner and return the underlying server to keep it alive.
    #[must_use] 
    pub fn into_server(self) -> HotReloadServer {
        self.server
    }
}

/// Main loop that handles file changes, debouncing, building, and broadcasting.
async fn run_loop(
    rust_build: RustBuild,
    file_rx: Receiver<()>,
    broadcast_tx: Sender<Vec<u8>>,
    event_tx: Sender<HotReloadEvent>,
    _watcher: FileWatcher, // Keep watcher alive
    crate_name: String,
) {
    let mut build_manager = BuildManager::new();
    let mut reported_change = false;

    loop {
        futures::select! {
            // File change detected
            _ = file_rx.recv().fuse() => {
                while file_rx.try_recv().is_ok() {}

                if !reported_change {
                    let _ = event_tx.send(HotReloadEvent::FileChanged).await;
                    reported_change = true;
                }
                build_manager.request_rebuild();
            }

            // Check debounce timer
            _ = FutureExt::fuse(smol::Timer::after(std::time::Duration::from_millis(50))) => {
                if let Some(result) = build_manager.poll_build().await {
                    match result {
                        Ok(lib_dir) => {
                            let lib_name = format!(
                                "{}{}{}",
                                std::env::consts::DLL_PREFIX,
                                crate_name,
                                std::env::consts::DLL_SUFFIX
                            );
                            let dylib_path = lib_dir.join(&lib_name);

                            if !dylib_path.exists() {
                                let _ = event_tx.send(HotReloadEvent::BuildFailed {
                                    error: format!("Library not found: {}", dylib_path.display()),
                                }).await;
                                reported_change = false;
                                continue;
                            }

                            let _ = event_tx.send(HotReloadEvent::Built {
                                path: dylib_path.clone(),
                            }).await;

                            // Read and broadcast the library
                            match smol::fs::read(&dylib_path).await {
                                Ok(data) => {
                                    let _ = broadcast_tx.send(data).await;
                                    let _ = event_tx.send(HotReloadEvent::Broadcast).await;
                                    reported_change = false;
                                }
                                Err(e) => {
                                    let _ = event_tx.send(HotReloadEvent::BuildFailed {
                                        error: format!("Failed to read library: {e}"),
                                    }).await;
                                    reported_change = false;
                                }
                            }
                        }
                        Err(e) => {
                            let _ = event_tx.send(HotReloadEvent::BuildFailed {
                                error: e.to_string(),
                            }).await;
                            reported_change = false;
                        }
                    }
                }

                // Check if debounce completed and we should start building
                if build_manager.should_start_build() {
                    let _ = event_tx.send(HotReloadEvent::Rebuilding).await;
                    build_manager.start_build(rust_build.clone());
                }
            }
        }
    }
}
