//! Cancellation token utilities for structured async task cancellation.
//!
//! This module provides a thin wrapper around `tokio_util::sync::CancellationToken`
//! with additional utilities for CLI use cases.

use std::future::Future;

use color_eyre::eyre::{Result, bail};
use tokio::process::Child;
pub use tokio_util::sync::CancellationToken;

/// Error type for cancelled operations.
#[derive(Debug, thiserror::Error)]
#[error("Operation cancelled")]
pub struct Cancelled;

/// Extension trait for `CancellationToken` with CLI-specific utilities.
pub trait CancellationTokenExt {
    /// Run a future with cancellation support.
    ///
    /// Returns `Err(Cancelled)` if the token is cancelled before the future completes.
    fn run<F, T>(&self, future: F) -> impl Future<Output = Result<T, Cancelled>> + Send
    where
        F: Future<Output = T> + Send,
        T: Send;

    /// Run a fallible future with cancellation support.
    ///
    /// Returns `Err(Cancelled)` if the token is cancelled before the future completes,
    /// otherwise returns the result of the future.
    fn run_result<F, T, E>(&self, future: F) -> impl Future<Output = Result<T, CancelledOr<E>>> + Send
    where
        F: Future<Output = Result<T, E>> + Send,
        T: Send,
        E: Send;
}

impl CancellationTokenExt for CancellationToken {
    fn run<F, T>(&self, future: F) -> impl Future<Output = Result<T, Cancelled>> + Send
    where
        F: Future<Output = T> + Send,
        T: Send,
    {
        let token = self.clone();
        async move {
            tokio::select! {
                biased;
                () = token.cancelled() => Err(Cancelled),
                result = future => Ok(result),
            }
        }
    }

    fn run_result<F, T, E>(&self, future: F) -> impl Future<Output = Result<T, CancelledOr<E>>> + Send
    where
        F: Future<Output = Result<T, E>> + Send,
        T: Send,
        E: Send,
    {
        let token = self.clone();
        async move {
            tokio::select! {
                biased;
                () = token.cancelled() => Err(CancelledOr::Cancelled),
                result = future => result.map_err(CancelledOr::Error),
            }
        }
    }
}

/// Error type that represents either a cancellation or another error.
#[derive(Debug)]
pub enum CancelledOr<E> {
    Cancelled,
    Error(E),
}

impl<E: std::fmt::Display> std::fmt::Display for CancelledOr<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "Operation cancelled"),
            Self::Error(e) => write!(f, "{e}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for CancelledOr<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Cancelled => None,
            Self::Error(e) => Some(e),
        }
    }
}

impl<E> CancelledOr<E> {
    /// Returns `true` if this is a cancellation error.
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// Returns the inner error if this is not a cancellation.
    #[must_use]
    pub fn into_error(self) -> Option<E> {
        match self {
            Self::Cancelled => None,
            Self::Error(e) => Some(e),
        }
    }
}

/// Wait for an async child process with cancellation support.
///
/// If cancelled, kills the child process and returns an error.
pub async fn wait_child_cancellable(
    child: &mut Child,
    cancel: &CancellationToken,
) -> Result<std::process::ExitStatus> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => {
            child.kill().await?;
            bail!("Build interrupted by user");
        }
        status = child.wait() => {
            Ok(status?)
        }
    }
}

/// Create a root cancellation token and register Ctrl+C handler.
///
/// Returns a token that will be cancelled when the user presses Ctrl+C.
pub fn create_root_token() -> CancellationToken {
    let token = CancellationToken::new();
    let handler_token = token.clone();

    // Register Ctrl+C handler
    // Note: This uses ctrlc crate which is already a dependency
    let _ = ctrlc::set_handler(move || {
        handler_token.cancel();
    });

    token
}
