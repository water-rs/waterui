//! Async runtime for the CLI.
//!
//! This module provides a unified async runtime using tokio. All async operations
//! in the CLI should run on this runtime.

use std::future::Future;

/// Run a future on the async runtime.
///
/// This is the main entry point for async code in the CLI. It creates a
/// multi-threaded tokio runtime and blocks on the provided future.
pub fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime")
        .block_on(future)
}
