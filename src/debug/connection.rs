//! WebSocket connection to CLI server.

use crate::debug::hot_reload::HotReloadConfig;

use super::event::{CliEvent, ConnectionError};
use super::library;
use alloc::string::String;
use async_channel::Sender;
use core::pin::Pin;
use core::task::{Context, Poll};
use futures::{FutureExt, Stream};
use serde::Deserialize;
use std::time::Instant;
use tracing::{debug, info, warn};
use zenwave::websocket::{WebSocketConfig, WebSocketMessage};

// ============================================================================
// Configuration
// ============================================================================

const MAX_RECONNECT_ATTEMPTS: u32 = 5;
const RECONNECT_DELAY_MS: u64 = 1000;
const MAX_RAPID_DISCONNECTS: u32 = 3;
const RAPID_DISCONNECT_WINDOW_MS: u64 = 5000;

// ============================================================================
// CliConnection
// ============================================================================

/// Connection to the CLI server.
///
/// Implements `Stream<Item = CliEvent>` for receiving events.
///
/// # Example
///
/// ```ignore
/// let (conn, outbound) = CliConnection::connect().await?;
/// while let Some(event) = conn.next().await {
///     match event {
///         CliEvent::LibraryReady(path) => { /* reload */ }
///         CliEvent::Connected => { /* hide overlay */ }
///         _ => {}
///     }
/// }
/// ```
#[derive(Debug)]
pub struct CliConnection {
    receiver: async_channel::Receiver<CliEvent>,
}

impl CliConnection {
    /// Create a new connection to the CLI.
    ///
    /// Returns `None` if hot reload is disabled or no endpoint is configured.
    /// Also returns a sender for outbound messages (logs, panic reports).
    #[must_use]
    pub fn connect(config: HotReloadConfig) -> (Self, Sender<String>) {
        let (event_tx, event_rx) = async_channel::unbounded();
        let (outbound_tx, outbound_rx) = async_channel::unbounded();
        executor_core::spawn_local(async move {
            run_connection_loop(config, event_tx, outbound_rx).await;
        })
        .detach();

        (Self { receiver: event_rx }, outbound_tx)
    }
}

impl Stream for CliConnection {
    type Item = CliEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // SAFETY: receiver is never moved after pinning the parent.
        let receiver = unsafe { self.map_unchecked_mut(|s| &mut s.receiver) };
        receiver.poll_next(cx)
    }
}

// ============================================================================
// Connection Loop
// ============================================================================

async fn run_connection_loop(
    config: HotReloadConfig,
    events: Sender<CliEvent>,
    outbound: async_channel::Receiver<String>,
) {
    let url = format!("ws://{}:{}/hot-reload-native", config.host(), config.port());
    let ws_config = WebSocketConfig::default()
        .with_max_message_size(None)
        .with_max_frame_size(None);

    let mut attempt = 0u32;
    let mut rapid_disconnects = 0u32;
    let mut last_connect: Option<Instant> = None;

    loop {
        attempt += 1;

        if attempt > 1 {
            let _ = events
                .send(CliEvent::Reconnecting {
                    attempt: attempt - 1,
                    max_attempts: MAX_RECONNECT_ATTEMPTS,
                })
                .await;

            #[cfg(not(target_arch = "wasm32"))]
            std::thread::sleep(std::time::Duration::from_millis(RECONNECT_DELAY_MS));
        }

        let socket =
            match zenwave::websocket::connect_with_config(&url, ws_config.clone()).await {
                Ok(socket) => socket,
                Err(err) => {
                    warn!(
                        "Connection attempt {}/{} failed: {}",
                        attempt, MAX_RECONNECT_ATTEMPTS, err
                    );

                    if attempt >= MAX_RECONNECT_ATTEMPTS {
                        let _ = events
                            .send(CliEvent::Error(ConnectionError::MaxReconnectAttempts(
                                MAX_RECONNECT_ATTEMPTS,
                            )))
                            .await;
                        return;
                    }

                    continue;
                }
            };

        info!("Hot reload connected to CLI");
        let _ = events.send(CliEvent::Connected).await;
        attempt = 0;
        last_connect = Some(Instant::now());

        let mut disconnect_reason: Option<String> = None;
        let mut outbound_closed = false;

        loop {
            if outbound_closed {
                match socket.recv().await {
                    Ok(Some(message)) => {
                        if let Some(event) = handle_incoming_message(message) {
                            let _ = events.send(event).await;
                        }
                    }
                    Ok(None) => {
                        debug!("Connection closed by server");
                        break;
                    }
                    Err(err) => {
                        disconnect_reason = Some(format!("Receive error: {err}"));
                        break;
                    }
                }

                continue;
            }

            futures::select_biased! {
                outbound_msg = outbound.recv().fuse() => {
                    match outbound_msg {
                        Ok(text) => {
                            if let Err(err) = socket.send_text(text).await {
                                disconnect_reason = Some(format!("Send error: {err}"));
                                break;
                            }
                        }
                        Err(_) => {
                            outbound_closed = true;
                        }
                    }
                }
                incoming = socket.recv().fuse() => {
                    match incoming {
                        Ok(Some(message)) => {
                            if let Some(event) = handle_incoming_message(message) {
                                let _ = events.send(event).await;
                            }
                        }
                        Ok(None) => {
                            debug!("Connection closed by server");
                            break;
                        }
                        Err(err) => {
                            disconnect_reason = Some(format!("Receive error: {err}"));
                            break;
                        }
                    }
                }
            }
        }

        if let Some(connect_time) = last_connect {
            if connect_time.elapsed().as_millis() < u128::from(RAPID_DISCONNECT_WINDOW_MS) {
                rapid_disconnects += 1;
                warn!(
                    "Rapid disconnection ({}/{})",
                    rapid_disconnects, MAX_RAPID_DISCONNECTS
                );

                if rapid_disconnects >= MAX_RAPID_DISCONNECTS {
                    let _ = events
                        .send(CliEvent::Error(ConnectionError::UnstableConnection(
                            rapid_disconnects,
                        )))
                        .await;
                    return;
                }
            } else {
                rapid_disconnects = 0;
            }
        }

        let _ = events.send(CliEvent::Disconnected).await;

        if let Some(err) = disconnect_reason {
            warn!("Disconnected: {}", err);
        }
    }
}

fn handle_incoming_message(message: WebSocketMessage) -> Option<CliEvent> {
    match message {
        WebSocketMessage::Binary(data) => {
            let path = library::create_library(&data);
            Some(CliEvent::LibraryReady(path))
        }
        WebSocketMessage::Text(text) => parse_server_message(&text),
        _ => None,
    }
}

#[derive(Deserialize)]
struct ServerMessage {
    #[serde(rename = "type")]
    kind: String,
    filter: Option<String>,
}

fn parse_server_message(text: &str) -> Option<CliEvent> {
    let msg: ServerMessage = serde_json::from_str(text).ok()?;
    match msg.kind.as_str() {
        "log_filter" => msg.filter.map(CliEvent::LogFilterChanged),
        _ => None,
    }
}
