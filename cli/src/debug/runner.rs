//! Hot reload runner that orchestrates file watching, building, and broadcasting.

use std::path::PathBuf;

use futures::{FutureExt, StreamExt};
use smol::channel::{self, Receiver, Sender};
use smol::Task;
use target_lexicon::Triple;

use super::file_watcher::FileWatcher;
use super::hot_reload::{BuildManager, HotReloadServer, DEFAULT_PORT};
use crate::build::RustBuild;
use crate::project::Project;

/// Events emitted by the hot reload runner.
#[derive(Debug, Clone)]
pub enum HotReloadEvent {
    /// Server started and listening.
    ServerStarted { host: String, port: u16 },
    /// File change detected, waiting for debounce.
    FileChanged,
    /// Starting a rebuild.
    Rebuilding,
    /// Build completed successfully, broadcasting to clients.
    Built { path: PathBuf },
    /// Build failed with an error message.
    BuildFailed { error: String },
    /// Library broadcast to connected clients.
    Broadcast,
}

/// Orchestrates hot reload: file watching, building, and broadcasting.
pub struct HotReloadRunner {
    server: HotReloadServer,
    event_rx: Receiver<HotReloadEvent>,
    _runner_task: Task<()>,
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

        let rust_build = RustBuild::new(project.root(), triple);
        let file_rx = watcher.receiver().clone();
        let broadcast_tx = server.broadcast_sender();

        // Spawn the runner task
        let runner_task = smol::spawn(run_loop(
            rust_build,
            file_rx,
            broadcast_tx,
            event_tx,
            watcher,
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
    pub fn port(&self) -> u16 {
        self.server.port()
    }

    /// Get the event receiver for hot reload events.
    pub fn events(&self) -> &Receiver<HotReloadEvent> {
        &self.event_rx
    }

    /// Consume the runner and return the underlying server to keep it alive.
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
) {
    let mut build_manager = BuildManager::new();

    loop {
        futures::select! {
            // File change detected
            _ = file_rx.recv().fuse() => {
                let _ = event_tx.send(HotReloadEvent::FileChanged).await;
                build_manager.request_rebuild();
            }

            // Check debounce timer
            _ = FutureExt::fuse(smol::Timer::after(std::time::Duration::from_millis(50))) => {
                // Check if debounce completed and we should start building
                if build_manager.should_start_build() {
                    let _ = event_tx.send(HotReloadEvent::Rebuilding).await;

                    // Build synchronously in this task for simplicity
                    match rust_build.build_hot_reload_lib().await {
                        Ok(lib_dir) => {
                            // Find the dylib in the output directory
                            if let Some(dylib_path) = find_dylib(&lib_dir).await {
                                let _ = event_tx.send(HotReloadEvent::Built {
                                    path: dylib_path.clone(),
                                }).await;

                                // Read and broadcast the library
                                if let Ok(data) = smol::fs::read(&dylib_path).await {
                                    let _ = broadcast_tx.send(data).await;
                                    let _ = event_tx.send(HotReloadEvent::Broadcast).await;
                                }
                            }
                        }
                        Err(e) => {
                            let _ = event_tx.send(HotReloadEvent::BuildFailed {
                                error: e.to_string(),
                            }).await;
                        }
                    }
                }
            }
        }
    }
}

/// Find the cdylib in the build output directory.
async fn find_dylib(lib_dir: &PathBuf) -> Option<PathBuf> {
    let mut entries = smol::fs::read_dir(lib_dir).await.ok()?;

    while let Some(entry) = entries.next().await {
        let entry = entry.ok()?;
        let path = entry.path();

        // Look for .dylib (macOS), .so (Linux/Android), .dll (Windows)
        if let Some(ext) = path.extension() {
            if ext == "dylib" || ext == "so" || ext == "dll" {
                // Skip deps directory artifacts
                if path
                    .file_name()
                    .is_some_and(|name| !name.to_string_lossy().contains("deps"))
                {
                    return Some(path);
                }
            }
        }
    }

    None
}
