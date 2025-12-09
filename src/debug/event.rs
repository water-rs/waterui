//! Events from the CLI connection.

use alloc::string::String;
use std::path::PathBuf;

/// Events emitted by [`super::CliConnection`].
#[derive(Debug)]
pub enum CliEvent {
    /// Successfully connected to CLI.
    Connected,

    /// Connection lost, will attempt to reconnect.
    Disconnected,

    /// Attempting to reconnect.
    Reconnecting {
        /// Current reconnection attempt number.
        attempt: u32,
        /// Maximum number of reconnection attempts.
        max_attempts: u32
    },

    /// A new library is available for hot reload.
    LibraryReady(PathBuf),

    /// CLI sent a log filter update.
    LogFilterChanged(String),

    /// Connection failed permanently.
    Error(ConnectionError),
}

/// Connection errors.
#[derive(Debug, Clone)]
pub enum ConnectionError {
    /// No endpoint configured.
    NoEndpoint,

    /// Failed to connect after max attempts.
    MaxReconnectAttempts(u32),

    /// Connection is unstable (rapid disconnects).
    UnstableConnection(u32),

    /// WebSocket error.
    WebSocket(String),
}

impl core::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoEndpoint => write!(f, "Hot reload endpoint not configured"),
            Self::MaxReconnectAttempts(n) => {
                write!(f, "Failed to connect after {n} attempts")
            }
            Self::UnstableConnection(n) => {
                write!(f, "Connection unstable: {n} rapid disconnections")
            }
            Self::WebSocket(msg) => write!(f, "WebSocket error: {msg}"),
        }
    }
}

impl std::error::Error for ConnectionError {}
