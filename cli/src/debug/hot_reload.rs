//! Hot reload server for WaterUI CLI.
//!
//! Provides a WebSocket server that broadcasts dylib updates to connected apps.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_tungstenite::tungstenite::Message;
use futures::{FutureExt, StreamExt};
use smol::channel::{self, Receiver, Sender};
use smol::lock::Mutex;
use smol::net::{TcpListener, TcpStream};
use smol::Task;

/// Default starting port for hot reload server.
pub const DEFAULT_PORT: u16 = 2006;

/// Number of ports to try before giving up.
pub const PORT_RETRY_COUNT: u16 = 50;

/// Debounce duration for file changes before triggering a rebuild.
pub const DEBOUNCE_DURATION: Duration = Duration::from_millis(150);

/// Hot reload server that broadcasts dylib updates to connected apps.
#[derive(Debug)]
pub struct HotReloadServer {
    port: u16,
    addr: SocketAddr,
    broadcast_tx: Sender<Vec<u8>>,
    _server_task: Task<()>,
}

/// Errors that can occur when launching the hot reload server.
#[derive(Debug, thiserror::Error)]
pub enum FailToLaunch {
    /// No available port found after trying all candidates.
    #[error("No available port found (tried ports {0}..{1})")]
    NoAvailablePort(u16, u16),

    /// Failed to bind to a specific port.
    #[error("Failed to bind to port {0}: {1}")]
    BindError(u16, std::io::Error),
}

/// Shared state for managing connected WebSocket clients.
struct ServerState {
    /// Senders to all connected clients.
    clients: Vec<Sender<Vec<u8>>>,
}

impl ServerState {
    fn new() -> Self {
        Self {
            clients: Vec::new(),
        }
    }

    fn add_client(&mut self, sender: Sender<Vec<u8>>) {
        self.clients.push(sender);
    }

    fn broadcast(&mut self, data: Vec<u8>) {
        // Remove disconnected clients and send to remaining ones
        self.clients.retain(|sender| sender.try_send(data.clone()).is_ok());
    }
}

impl HotReloadServer {
    /// Launch the hot reload server, trying ports starting from `starting_port`.
    ///
    /// Will try up to `PORT_RETRY_COUNT` consecutive ports if the initial port is busy.
    ///
    /// # Errors
    /// Returns `FailToLaunch::NoAvailablePort` if no port could be bound.
    pub async fn launch(starting_port: u16) -> Result<Self, FailToLaunch> {
        let end_port = starting_port.saturating_add(PORT_RETRY_COUNT);

        for port in starting_port..end_port {
            match Self::try_launch_on_port(port).await {
                Ok(server) => return Ok(server),
                Err(FailToLaunch::BindError(_, _)) => continue,
                Err(e) => return Err(e),
            }
        }

        Err(FailToLaunch::NoAvailablePort(starting_port, end_port))
    }

    /// Try to launch the server on a specific port.
    async fn try_launch_on_port(port: u16) -> Result<Self, FailToLaunch> {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| FailToLaunch::BindError(port, e))?;

        let actual_addr = listener
            .local_addr()
            .map_err(|e| FailToLaunch::BindError(port, e))?;

        // Channel for broadcasting dylib updates to the server task
        let (broadcast_tx, broadcast_rx) = channel::unbounded::<Vec<u8>>();

        // Shared state for managing clients
        let state = Arc::new(Mutex::new(ServerState::new()));

        // Spawn background task to handle broadcasts
        let state_for_broadcast = state.clone();
        let broadcast_task = smol::spawn(async move {
            while let Ok(data) = broadcast_rx.recv().await {
                let mut state = state_for_broadcast.lock().await;
                state.broadcast(data);
            }
        });

        // Spawn the server task
        let server_task = smol::spawn(async move {
            Self::run_server(listener, state).await;
            drop(broadcast_task);
        });

        Ok(Self {
            port: actual_addr.port(),
            addr: actual_addr,
            broadcast_tx,
            _server_task: server_task,
        })
    }

    /// Run the server loop accepting connections.
    async fn run_server(listener: TcpListener, state: Arc<Mutex<ServerState>>) {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let state = state.clone();
                    smol::spawn(async move {
                        if let Err(e) = Self::handle_connection(stream, addr, state).await {
                            tracing::debug!("WebSocket connection error from {addr}: {e}");
                        }
                    })
                    .detach();
                }
                Err(e) => {
                    tracing::warn!("Failed to accept connection: {e}");
                }
            }
        }
    }

    /// Handle a single WebSocket connection.
    async fn handle_connection(
        stream: TcpStream,
        addr: SocketAddr,
        state: Arc<Mutex<ServerState>>,
    ) -> Result<(), async_tungstenite::tungstenite::Error> {
        // smol's TcpStream implements futures::AsyncRead/Write
        let mut ws_stream = async_tungstenite::accept_async(stream).await?;
        tracing::info!("Hot reload client connected from {addr}");

        // Create a channel for this client to receive broadcasts
        let (client_tx, client_rx) = channel::unbounded::<Vec<u8>>();

        // Register this client
        {
            let mut state = state.lock().await;
            state.add_client(client_tx);
        }

        // Handle the WebSocket connection - interleave sending and receiving
        loop {
            futures::select! {
                // Check for data to send to client
                data = client_rx.recv().fuse() => {
                    match data {
                        Ok(data) => {
                            if ws_stream.send(Message::Binary(data.into())).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break, // Channel closed
                    }
                }
                // Check for messages from client
                msg = ws_stream.next().fuse() => {
                    match msg {
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Ok(Message::Ping(data))) => {
                            // Respond with pong
                            if ws_stream.send(Message::Pong(data)).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(_)) => {
                            // Ignore other messages from client for now
                        }
                        Some(Err(_)) => break,
                    }
                }
            }
        }

        tracing::info!("Hot reload client disconnected from {addr}");
        Ok(())
    }

    /// Get the port the server is listening on.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Get the address the server is listening on.
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Get the host string for environment variable.
    #[must_use]
    pub fn host(&self) -> String {
        self.addr.ip().to_string()
    }

    /// Broadcast a library binary to all connected clients.
    ///
    /// Returns immediately; the broadcast happens asynchronously.
    pub fn send_library(&self, data: Vec<u8>) {
        let _ = self.broadcast_tx.try_send(data);
    }

    /// Broadcast a library file to all connected clients.
    ///
    /// Reads the file and sends its contents to all connected apps.
    ///
    /// # Errors
    /// Returns an error if the file cannot be read.
    pub async fn send_library_file(&self, path: &PathBuf) -> std::io::Result<()> {
        let data = smol::fs::read(path).await?;
        self.send_library(data);
        Ok(())
    }
}

/// Manages hot reload builds with debouncing and cancellation.
#[derive(Debug)]
pub struct BuildManager {
    /// Currently running build task (can be cancelled by dropping).
    current_build: Option<Task<Result<PathBuf, crate::build::RustBuildError>>>,
    /// Debounce timer task.
    debounce_task: Option<Task<()>>,
    /// Channel to signal debounce completion.
    debounce_rx: Option<Receiver<()>>,
}

impl BuildManager {
    /// Create a new build manager.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            current_build: None,
            debounce_task: None,
            debounce_rx: None,
        }
    }

    /// Request a rebuild, cancelling any in-flight build and resetting debounce.
    ///
    /// This method should be called when a file change is detected.
    /// The actual build will start after `DEBOUNCE_DURATION` of no further changes.
    pub fn request_rebuild(&mut self) {
        // Cancel any in-flight build by dropping
        self.current_build.take();

        // Cancel previous debounce timer by dropping
        self.debounce_task.take();
        self.debounce_rx.take();

        // Start new debounce timer
        let (tx, rx) = channel::bounded(1);
        self.debounce_task = Some(smol::spawn(async move {
            smol::Timer::after(DEBOUNCE_DURATION).await;
            let _ = tx.send(()).await;
        }));
        self.debounce_rx = Some(rx);
    }

    /// Check if the debounce timer has fired and a build should start.
    ///
    /// Returns `true` if a build should be started.
    pub fn should_start_build(&mut self) -> bool {
        if let Some(rx) = &self.debounce_rx {
            if rx.try_recv().is_ok() {
                self.debounce_task.take();
                self.debounce_rx.take();
                return true;
            }
        }
        false
    }

    /// Start a build for the given rust build configuration.
    pub fn start_build(&mut self, rust_build: crate::build::RustBuild) {
        self.current_build = Some(smol::spawn(async move {
            rust_build.build_hot_reload_lib().await
        }));
    }

    /// Check if the current build has completed.
    ///
    /// Returns `Some(path)` if the build completed successfully,
    /// `None` if the build is still running or failed.
    pub fn poll_build(&mut self) -> Option<PathBuf> {
        if let Some(task) = &self.current_build {
            // Check if task is done without blocking
            if task.is_finished() {
                if let Some(task) = self.current_build.take() {
                    // Use blocking to get the result since we know it's done
                    return smol::block_on(task).ok();
                }
            }
        }
        None
    }

    /// Check if a build is currently in progress.
    #[must_use]
    pub fn is_building(&self) -> bool {
        self.current_build.is_some()
    }

    /// Check if we're waiting for debounce.
    #[must_use]
    pub fn is_debouncing(&self) -> bool {
        self.debounce_rx.is_some()
    }
}

impl Default for BuildManager {
    fn default() -> Self {
        Self::new()
    }
}
