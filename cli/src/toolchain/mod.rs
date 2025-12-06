//! Toolchain detection and installation system.
//!
//! This module provides a unified interface for checking and installing
//! development toolchain dependencies (Rust targets, Xcode, Android SDK, etc.).
//!
//! # Design
//!
//! The [`Toolchain`] trait defines a two-phase workflow:
//! 1. `check()` - Verify if the toolchain is properly installed
//! 2. `fix()` - Attempt to install missing components
//!
//! The `fix()` method returns an [`Installation`] which represents a pending
//! installation. The installation is only performed when `.install()` is called,
//! allowing the caller to:
//! - Display what will be installed via `Display` trait
//! - Confirm by calling: `installation.install().await`
//! - Reject by dropping: `drop(installation)`
//!
//! # Example
//!
//! ```ignore
//! use waterui_cli::toolchain::{Toolchain, Rust};
//! use waterui_cli::toolchain::installation::{Installation, Progress};
//!
//! let rust = Rust::new()
//!     .with_target("aarch64-linux-android")
//!     .with_target("aarch64-apple-ios");
//!
//! // Check if everything is installed
//! if let Err(missing) = rust.check().await {
//!     println!("Missing: {missing}");
//!
//!     // Try to fix
//!     match rust.fix() {
//!         Ok(installation) => {
//!             println!("Will install: {installation}");
//!
//!             // With progress tracking
//!             let progress = Progress::new(|name, status| {
//!                 println!("{name}: {status:?}");
//!             });
//!             installation.install(progress).await?;
//!         }
//!         Err(e) => {
//!             eprintln!("error: {e}");
//!         }
//!     }
//! }
//! ```

mod android;
mod error;
pub mod installation;
mod rust;
mod swift;

pub use android::Android;
pub use error::{ToolchainError, ToolchainErrorKind};
pub use installation::{Installation, InstallationReport, Progress, Status};
pub use rust::{Rust, RustTarget, Rustup};
pub use swift::Swift;

use std::future::Future;

/// Trait for toolchain dependencies that can be checked and installed.
///
/// Implementors represent a specific toolchain configuration (e.g., Rust with
/// certain targets, Android SDK with specific components).
///
/// The associated `Installation` type preserves full type information through
/// the composition, enabling zero-cost abstractions for parallel/sequential
/// installation plans.
pub trait Toolchain: Send + Sync {
    /// The installation type returned by `fix()`.
    ///
    /// This preserves the concrete type through composition, avoiding
    /// dynamic dispatch.
    type Installation: Installation;

    /// Check if the toolchain is properly installed.
    ///
    /// Returns `Ok(())` if all components are available, or `Err` describing
    /// what is missing.
    fn check(&self) -> impl Future<Output = Result<(), ToolchainError>> + Send;

    /// Attempt to create an installation plan for missing components.
    ///
    /// Returns `Ok(Installation)` if the missing components can be installed
    /// automatically, or `Err` if manual intervention is required.
    ///
    /// The returned [`Installation`] is a pending operation that must have
    /// `.install()` called to actually perform the installation.
    fn fix(&self) -> Result<Self::Installation, ToolchainError>;

    /// Human-readable name for this toolchain (e.g., "Rust", "Android SDK").
    fn name(&self) -> &'static str;
}
