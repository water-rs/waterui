//! WebSocket connection to the CLI hot reload server.

use alloc::vec::Vec;

use futures::FutureExt;
use serde::{Deserialize, Serialize};
use zenwave::websocket;

use super::event::ConnectionError;
use crate::debug::hot_reload::HotReloadConfig;

/// Connection to the CLI hot reload server.
#[derive(Debug)]
pub struct CliConnection {
    socket: zenwave::websocket::WebSocket,
}

/// Sender half of the CLI connection (for future app-to-CLI communication).
#[derive(Debug)]
pub struct CliSender {
    #[allow(dead_code)]
    socket: zenwave::websocket::WebSocket,
}

/// Receiver half of the CLI connection.
#[derive(Debug)]
pub struct CliReceiver {
    socket: zenwave::websocket::WebSocket,
}

/// Connection timeout in seconds.
const CONNECTION_TIMEOUT_SECS: u64 = 5;

impl CliConnection {
    /// Connect to the CLI hot reload server.
    ///
    /// Times out after 5 seconds if the connection cannot be established.
    ///
    /// # Errors
    /// Returns an error if the connection fails or times out.
    pub async fn connect(config: HotReloadConfig) -> Result<Self, ConnectionError> {
        use alloc::string::ToString;
        use core::time::Duration;

        let url = alloc::format!("ws://{}:{}", config.host(), config.port());

        // Race the connection against a timeout
        let timeout = native_executor::sleep(Duration::from_secs(CONNECTION_TIMEOUT_SECS));
        let connect = websocket::connect(&url);

        // Use select to race connection vs timeout
        futures::select! {
            result = Box::pin(connect).fuse() => {
                let socket = result.map_err(|e| ConnectionError::WebSocket(e.to_string()))?;
                Ok(Self { socket })
            }
            _ = Box::pin(timeout).fuse() => {
                Err(ConnectionError::Timeout)
            }
        }
    }

    /// Convert into a receiver (consumes the connection).
    ///
    /// For now, we don't need the sender since app-to-CLI communication is not yet implemented.
    #[must_use]
    pub fn into_receiver(self) -> CliReceiver {
        CliReceiver {
            socket: self.socket,
        }
    }
}

impl CliReceiver {
    /// Receive the next event from the CLI.
    ///
    /// Returns `None` if the connection is closed.
    pub async fn recv(&mut self) -> Option<CliEvent> {
        use zenwave::websocket::WebSocketMessage;

        loop {
            match self.socket.recv().await {
                Ok(Some(WebSocketMessage::Binary(data))) => {
                    return Some(CliEvent::HotReload {
                        binary: data.to_vec(),
                    });
                }
                Ok(Some(
                    WebSocketMessage::Text(_)
                    | WebSocketMessage::Ping(_)
                    | WebSocketMessage::Pong(_)
                    | WebSocketMessage::Close,
                )) => {
                    // Ignore text and control messages
                }
                Ok(None) | Err(_) => {
                    // Connection closed
                    return None;
                }
            }
        }
    }
}

/// Events received from the CLI.
#[derive(Debug, Serialize, Deserialize)]
pub enum CliEvent {
    /// A new hot reload library binary is ready.
    HotReload {
        /// The raw binary data of the dynamic library.
        binary: Vec<u8>,
    },
}

/// Panic report sent from app to CLI.
#[derive(Debug, Serialize, Deserialize)]
pub struct PanicReport {}

/// Events sent from app to CLI.
#[derive(Debug, Serialize, Deserialize)]
pub enum AppEvent {
    /// The app has crashed.
    Crashed {},
}
