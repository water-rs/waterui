//! WebSocket server for hot reload communication.

use std::{
    io,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;
use serde_json::json;
use skyzen::{
    CreateRouteNode, Route, StaticDir,
    runtime::native as skyzen_runtime,
    websocket::{WebSocket, WebSocketMessage, WebSocketUpgrade},
};
use tokio::{fs as tokio_fs, sync::broadcast};
use tracing::{debug, info, warn};

use crate::output;

use super::{DisconnectReason, HotReloadError, HotReloadMessage};

/// WebSocket server for hot reload communication.
///
/// The server provides two endpoints:
/// - `/hot-reload-native` - For native apps (iOS/Android/macOS)
/// - `/hot-reload-web` - For web apps (browser)
///
/// Connection events are returned separately from `start()` to allow the caller
/// to own the receiver without requiring Clone.
#[derive(Debug)]
pub struct Server {
    address: SocketAddr,
    thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
    hot_reload_tx: broadcast::Sender<HotReloadMessage>,
    shutdown: Arc<AtomicBool>,
}

impl Server {
    /// Start the hot reload server on the specified port.
    ///
    /// Returns the server and a receiver for connection events. The server
    /// sends notifications to connected clients, while the receiver allows
    /// monitoring connection state.
    ///
    /// # Errors
    /// Returns an error if the server fails to start or bind to the port.
    pub fn start(
        port: u16,
        static_path: PathBuf,
        log_filter: Option<String>,
    ) -> Result<(Self, NativeConnectionEvents), HotReloadError> {
        let (hot_reload_tx, _) = broadcast::channel(16);
        let (connection_event_tx, connection_event_rx) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));

        let app_state = Arc::new(ServerState {
            hot_reload_tx: hot_reload_tx.clone(),
            connection_event_tx,
            log_filter,
            shutdown: shutdown.clone(),
        });

        let (startup_tx, startup_rx) = std::sync::mpsc::channel::<Result<SocketAddr, io::Error>>();

        let thread = thread::spawn(move || {
            skyzen_runtime::init_logging();
            let router = build_router(app_state, static_path);
            let address = SocketAddr::from(([127, 0, 0, 1], port));

            // SAFETY: The address string is well-formed and under our control.
            unsafe {
                std::env::set_var("SKYZEN_ADDRESS", address.to_string());
            }

            let _ = startup_tx.send(Ok(address));
            skyzen_runtime::launch(move || async { router });
        });

        let startup_result = startup_rx
            .recv()
            .map_err(|_| {
                HotReloadError::ServerStart(io::Error::new(
                    io::ErrorKind::Other,
                    "hot reload server failed to report its status",
                ))
            })?
            .map_err(HotReloadError::BindFailed)?;

        let address = startup_result;

        // Wait for server to be ready by probing the socket
        wait_for_server_ready(address)?;

        let connection_events = NativeConnectionEvents::new(connection_event_rx);

        Ok((
            Self {
                address,
                thread: Arc::new(Mutex::new(Some(thread))),
                hot_reload_tx,
                shutdown,
            },
            connection_events,
        ))
    }

    /// Get the server's bound address.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    /// Notify connected native clients to reload with a new library.
    pub fn notify_native_reload(&self, path: PathBuf) {
        info!("Hot reload: queueing native artifact {}", path.display());
        let _ = self.hot_reload_tx.send(HotReloadMessage::Native(path));
    }

    /// Notify connected web clients to reload.
    pub fn notify_web_reload(&self) {
        let _ = self.hot_reload_tx.send(HotReloadMessage::Web);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);

        let thread_handle = self.thread.lock().unwrap().take();

        if let Some(handle) = thread_handle {
            let start = Instant::now();
            let timeout = Duration::from_millis(500);

            loop {
                if handle.is_finished() {
                    let _ = handle.join();
                    break;
                }
                if start.elapsed() > timeout {
                    debug!("Hot reload server shutdown timed out, continuing...");
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

// =============================================================================
// Internal state and helpers
// =============================================================================

#[derive(Clone)]
struct ServerState {
    hot_reload_tx: broadcast::Sender<HotReloadMessage>,
    connection_event_tx: mpsc::Sender<NativeConnectionEvent>,
    log_filter: Option<String>,
    shutdown: Arc<AtomicBool>,
}

fn wait_for_server_ready(address: SocketAddr) -> Result<(), HotReloadError> {
    use std::net::TcpStream;

    let max_attempts = 50;
    let delay = Duration::from_millis(20);

    for attempt in 0..max_attempts {
        match TcpStream::connect_timeout(&address.into(), Duration::from_millis(100)) {
            Ok(_) => {
                debug!("Hot reload server ready after {} attempts", attempt + 1);
                return Ok(());
            }
            Err(_) => {
                thread::sleep(delay);
            }
        }
    }

    Err(HotReloadError::ServerNotReady {
        address: address.to_string(),
        attempts: max_attempts,
    })
}

fn build_router(state: Arc<ServerState>, static_path: PathBuf) -> skyzen::routing::Router {
    let native_state = state.clone();
    let web_state = state.clone();

    Route::new((
        "/hot-reload-native".at(move |ws: WebSocketUpgrade| {
            let state = native_state.clone();
            async move {
                ws.max_message_size(None)
                    .on_upgrade(move |socket| handle_native_socket(socket, state))
            }
        }),
        "/hot-reload-web".at(move |ws: WebSocketUpgrade| {
            let state = web_state.clone();
            async move { ws.on_upgrade(move |socket| handle_web_socket(socket, state)) }
        }),
        StaticDir::new("/", static_path),
    ))
    .build()
}

async fn handle_native_socket(mut socket: WebSocket, state: Arc<ServerState>) {
    use futures_util::StreamExt;

    // Send log filter configuration if set
    if let Some(filter) = &state.log_filter {
        let message = json!({
            "type": "log_filter",
            "filter": filter,
        })
        .to_string();
        if let Err(err) = socket.send_text(message).await {
            warn!("Failed to send log filter to CLI: {err}");
        }
    }

    let mut rx = state.hot_reload_tx.subscribe();
    let _ = state
        .connection_event_tx
        .send(NativeConnectionEvent::Connected);

    let mut shutdown_check = tokio::time::interval(tokio::time::Duration::from_millis(100));

    loop {
        if state.shutdown.load(Ordering::Relaxed) {
            break;
        }

        tokio::select! {
            _ = shutdown_check.tick() => {}
            Some(msg) = socket.next() => {
                match msg {
                    Ok(WebSocketMessage::Text(payload)) => {
                        handle_native_client_message(&payload);
                    }
                    Ok(WebSocketMessage::Close) => {
                        let reason = DisconnectReason::Graceful {
                            code: None, // TODO: extract code from close frame
                        };
                        let _ = state.connection_event_tx.send(
                            NativeConnectionEvent::Disconnected(reason)
                        );
                        break;
                    }
                    Ok(WebSocketMessage::Ping(payload)) => {
                        let _ = socket.send_message(WebSocketMessage::Pong(payload)).await;
                    }
                    Ok(WebSocketMessage::Binary(_) | WebSocketMessage::Pong(_)) => {}
                    Err(err) => {
                        let reason = DisconnectReason::Abnormal {
                            details: err.to_string(),
                        };
                        let _ = state.connection_event_tx.send(
                            NativeConnectionEvent::Disconnected(reason)
                        );
                        break;
                    }
                }
            }
            msg = rx.recv() => {
                match msg {
                    Ok(HotReloadMessage::Native(path)) => {
                        match tokio_fs::read(&path).await {
                            Ok(data) => {
                                info!(
                                    "Hot reload: sending {} ({} bytes)",
                                    path.display(),
                                    data.len()
                                );
                                if let Err(err) = socket.send_message(WebSocketMessage::Binary(data.into())).await {
                                    let reason = DisconnectReason::Abnormal {
                                        details: err.to_string(),
                                    };
                                    tracing::error!("Failed to send hot reload artifact: {err}");
                                    let _ = state.connection_event_tx.send(
                                        NativeConnectionEvent::Disconnected(reason)
                                    );
                                    break;
                                }
                            }
                            Err(err) => {
                                let exists = path.exists();
                                warn!(
                                    "Failed to read hot reload artifact at {} (exists: {}): {err:?}",
                                    path.display(),
                                    exists
                                );
                            }
                        }
                    }
                    Ok(HotReloadMessage::Web) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("Missed {skipped} hot reload updates (CLI lagged behind)");
                    }
                }
            }
            else => break,
        }
    }
}

async fn handle_web_socket(mut socket: WebSocket, state: Arc<ServerState>) {
    let mut rx = state.hot_reload_tx.subscribe();
    let mut shutdown_check = tokio::time::interval(tokio::time::Duration::from_millis(100));

    loop {
        if state.shutdown.load(Ordering::Relaxed) {
            break;
        }

        tokio::select! {
            _ = shutdown_check.tick() => {}
            msg = rx.recv() => {
                match msg {
                    Ok(HotReloadMessage::Web) => {
                        if socket.send_text("reload").await.is_err() {
                            break;
                        }
                    }
                    Ok(HotReloadMessage::Native(_)) => {}
                    Err(_) => break,
                }
            }
        }
    }
}

// =============================================================================
// Native client message handling
// =============================================================================

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum NativeClientEvent {
    #[serde(rename = "panic")]
    Panic(NativePanicReport),
    #[serde(rename = "log")]
    Log(NativeLogEvent),
}

#[derive(Debug, Deserialize)]
struct NativePanicReport {
    message: String,
    location: Option<NativePanicLocation>,
    thread: Option<String>,
    backtrace: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NativeLogEvent {
    message: String,
    level: String,
    target: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NativePanicLocation {
    file: String,
    line: u32,
    column: u32,
}

fn handle_native_client_message(payload: &str) {
    match serde_json::from_str::<NativeClientEvent>(payload) {
        Ok(NativeClientEvent::Panic(report)) => emit_remote_panic(report),
        Ok(NativeClientEvent::Log(event)) => emit_remote_log(event),
        Err(err) => {
            warn!("Failed to parse native client message ({err}): {payload}");
        }
    }
}

fn emit_remote_panic(report: NativePanicReport) {
    use console::style;

    if output::global_output_format().is_json() {
        warn!("App panic: {:?}", report);
        return;
    }

    println!();
    println!(
        "{}",
        style("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
            .red()
            .dim()
    );
    eprintln!("{} PANIC in app", style("error:").red().bold());
    println!(
        "{}",
        style("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
            .red()
            .dim()
    );
    println!();

    println!(
        "  {} {}",
        style("Message:").bold(),
        style(&report.message).red()
    );

    if let Some(location) = &report.location {
        let location_str = format!("{}:{}:{}", location.file, location.line, location.column);
        println!(
            "  {} {}",
            style("Location:").bold(),
            style(&location_str).cyan().underlined()
        );
    }

    if let Some(thread) = &report.thread {
        println!("  {} {}", style("Thread:").bold(), thread);
    }

    if let Some(backtrace) = &report.backtrace {
        let backtrace = backtrace.trim();
        if !backtrace.is_empty() && backtrace != "disabled backtrace" {
            println!();
            println!("  {}", style("Backtrace:").bold());
            for line in backtrace.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                let is_user_frame = !line.contains("std::")
                    && !line.contains("core::")
                    && !line.contains("alloc::")
                    && !line.contains("<unknown>")
                    && !line.contains("rust_begin_unwind");

                if is_user_frame && (line.contains("::") || line.contains(" at ")) {
                    println!("    {}", style(line).yellow());
                } else {
                    println!("    {}", style(line).dim());
                }
            }
        }
    }

    println!();
    println!(
        "{}",
        style("────────────────────────────────────────────────────────────────────────────────")
            .dim()
    );
    println!(
        "  {} Fix the panic above, save, and WaterUI will rebuild automatically.",
        style("hint:").cyan()
    );
    println!();
}

fn emit_remote_log(event: NativeLogEvent) {
    use crate::WATERUI_TRACING_PREFIX;

    if output::global_output_format().is_json() {
        return;
    }

    let target = event.target.unwrap_or_default();
    let message = event
        .message
        .trim()
        .trim_start_matches(WATERUI_TRACING_PREFIX)
        .trim_start();

    if target.is_empty() {
        println!("{} [{}] {}", WATERUI_TRACING_PREFIX, event.level, message);
    } else {
        println!(
            "{} [{}] {} ({})",
            WATERUI_TRACING_PREFIX, event.level, message, target
        );
    }
}

// =============================================================================
// Connection events
// =============================================================================

/// Events from native app connections.
#[derive(Clone, Debug)]
pub enum NativeConnectionEvent {
    /// App connected to hot reload server
    Connected,
    /// App disconnected from hot reload server
    Disconnected(DisconnectReason),
}

/// Receiver for native connection events.
///
/// This is a simple wrapper around `mpsc::Receiver` that provides
/// the same interface. It is not cloneable - there should be only
/// one consumer of connection events.
#[derive(Debug)]
pub struct NativeConnectionEvents {
    receiver: mpsc::Receiver<NativeConnectionEvent>,
}

impl NativeConnectionEvents {
    fn new(receiver: mpsc::Receiver<NativeConnectionEvent>) -> Self {
        Self { receiver }
    }

    /// Wait for a connection event with timeout.
    ///
    /// # Errors
    /// Returns an error if the timeout expires or the channel is disconnected.
    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<NativeConnectionEvent, mpsc::RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)
    }

    /// Try to receive a connection event without blocking.
    ///
    /// # Errors
    /// Returns an error if no event is available or the channel is disconnected.
    pub fn try_recv(&self) -> Result<NativeConnectionEvent, TryRecvError> {
        self.receiver.try_recv()
    }
}
