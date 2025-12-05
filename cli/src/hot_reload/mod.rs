//! Hot reload server and file watcher infrastructure.
//!
//! This module provides:
//! - `Server` - WebSocket server for hot reload communication
//! - `RebuildWatcher` - File system watcher that triggers rebuilds
//! - `NativeConnectionEvents` - Channel for tracking app connections
//!
//! ## Architecture
//!
//! The hot reload system has two modes:
//! - **Native** (iOS/Android/macOS): Sends compiled dylib over WebSocket
//! - **Web**: Sends reload signal, browser refetches WASM
//!
//! ```text
//! ┌─────────────────┐       ┌──────────────────┐
//! │  File Watcher   │──────▶│  RebuildWatcher  │
//! └─────────────────┘       └────────┬─────────┘
//!                                    │ on_change
//!                                    ▼
//!                           ┌──────────────────┐
//!                           │   cargo build    │
//!                           └────────┬─────────┘
//!                                    │
//!                                    ▼
//!                           ┌──────────────────┐
//!                           │     Server       │
//!                           │  (WebSocket)     │
//!                           └────────┬─────────┘
//!                                    │
//!                      ┌─────────────┼─────────────┐
//!                      ▼             ▼             ▼
//!               ┌───────────┐ ┌───────────┐ ┌───────────┐
//!               │    App    │ │    App    │ │  Browser  │
//!               │  (iOS)    │ │ (Android) │ │   (Web)   │
//!               └───────────┘ └───────────┘ └───────────┘
//! ```

mod server;
mod watcher;

pub use server::{NativeConnectionEvent, NativeConnectionEvents, Server};
pub use watcher::{FileChanged, FileWatcher, poll_file_changes};

use std::{fmt, path::PathBuf};
use thiserror::Error;

/// Message types sent from server to connected clients.
#[derive(Debug, Clone)]
pub enum HotReloadMessage {
    /// Native hot reload: send compiled library binary
    Native(PathBuf),
    /// Web hot reload: signal browser to reload
    Web,
}

/// Errors that can occur during hot reload operations.
#[derive(Debug, Error)]
pub enum HotReloadError {
    /// Failed to start the hot reload server
    #[error("failed to start hot reload server: {0}")]
    ServerStart(#[source] std::io::Error),

    /// Failed to bind to a port
    #[error("failed to bind hot reload server socket: {0}")]
    BindFailed(#[source] std::io::Error),

    /// Server failed to become ready in time
    #[error("hot reload server failed to start listening on {address} after {attempts} attempts")]
    ServerNotReady { address: String, attempts: u32 },

    /// Connection was lost
    #[error("hot reload connection lost: {reason}")]
    ConnectionLost { reason: DisconnectReason },

    /// Interrupted while waiting for connection
    #[error("interrupted while waiting for hot reload connection")]
    Interrupted,

    /// App failed to connect in time
    #[error("app failed to establish hot reload connection within timeout")]
    ConnectionTimeout,

    /// Server shutdown unexpectedly
    #[error("hot reload server shut down before app connected")]
    ServerShutdown,
}

/// Reason why a hot reload connection was disconnected.
#[derive(Debug, Clone)]
pub enum DisconnectReason {
    /// App closed the connection gracefully
    Graceful {
        /// WebSocket close code, if provided
        code: Option<u16>,
    },
    /// Connection failed abnormally
    Abnormal {
        /// Error details
        details: String,
    },
}

impl fmt::Display for DisconnectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Graceful { code: Some(code) } => {
                write!(f, "connection closed by app (close code {code})")
            }
            Self::Graceful { code: None } => {
                write!(f, "connection closed by app")
            }
            Self::Abnormal { details } if details.trim().is_empty() => {
                write!(f, "connection failed unexpectedly (app likely crashed)")
            }
            Self::Abnormal { details } => {
                write!(
                    f,
                    "connection failed: {} (app likely crashed)",
                    details.trim()
                )
            }
        }
    }
}

/// Wait outcome when blocking for user interrupt or connection loss.
#[derive(Debug)]
pub enum WaitOutcome {
    /// User pressed Ctrl+C
    Interrupted,
    /// Hot reload connection was lost
    ConnectionLost(DisconnectReason),
}
