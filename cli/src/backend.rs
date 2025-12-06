pub mod android;
pub mod apple;

use self::{android::AndroidBackend, apple::Apple};

use core::fmt::{Debug, Display};
use std::future::Future;

use color_eyre::eyre;
use tokio_util::sync::CancellationToken;

use crate::{device::DeviceInfo, project::Project, toolchain::ToolchainError};

/// A backend for building and packaging `WaterUI` projects.
///
/// Implementors should provide methods for initializing, cleaning,
/// checking requirements, and scanning for available devices.
///
/// The trait uses native async fn (Rust 1.75+) without the `async_trait` crate.
pub trait Backend: Display + Debug + Send + Sync {
    /// Initialize the backend for the given project.
    ///
    /// If `dev` is true, initialize in development mode.
    /// This may include setting up debug configurations or
    /// installing development dependencies.
    ///
    /// # Errors
    /// Returns an error if initialization fails.
    fn init(&self, project: &Project, dev: bool) -> eyre::Result<()>;

    /// Check if the backend is already set up for the given project.
    fn is_existing(&self, project: &Project) -> bool;

    /// Clean up any files or configurations added by this backend
    /// for the given project.
    ///
    /// # Errors
    /// Returns an error if cleaning fails.
    fn clean(&self, project: &Project) -> eyre::Result<()>;

    /// Check if the required toolchain components are available
    /// for this backend to function correctly.
    ///
    /// # Errors
    /// Returns a list of toolchain errors if requirements are not met.
    fn check_requirements(&self, project: &Project) -> Result<(), Vec<ToolchainError>>;

    /// Scan for available devices for this backend.
    ///
    /// This is an async method that can be cancelled via `CancellationToken`.
    /// Returns a list of devices (simulators, emulators, physical devices).
    ///
    /// # Errors
    /// Returns an error if device scanning fails or is cancelled.
    fn scan_devices(
        &self,
        cancel: CancellationToken,
    ) -> impl Future<Output = eyre::Result<Vec<DeviceInfo>>> + Send;
}

// Note: AnyBackend/dyn Backend is not supported with async trait methods
// that return `impl Future`. Use enum dispatch or generics instead.
// The old `AnyBackend = Box<dyn Backend>` pattern has been removed.

/// Enum dispatch for runtime polymorphism over backends.
///
/// Use this when you need to store multiple backend types in a collection
/// or pass a backend whose type isn't known at compile time.
#[derive(Debug)]
pub enum AnyBackend {
    Apple(Apple),
    Android(AndroidBackend),
}

impl std::fmt::Display for AnyBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Apple(b) => std::fmt::Display::fmt(b, f),
            Self::Android(b) => std::fmt::Display::fmt(b, f),
        }
    }
}

impl AnyBackend {
    /// Initialize the backend.
    pub fn init(&self, project: &Project, dev: bool) -> eyre::Result<()> {
        match self {
            Self::Apple(b) => b.init(project, dev),
            Self::Android(b) => b.init(project, dev),
        }
    }

    /// Check if the backend is already set up.
    #[must_use] 
    pub fn is_existing(&self, project: &Project) -> bool {
        match self {
            Self::Apple(b) => b.is_existing(project),
            Self::Android(b) => b.is_existing(project),
        }
    }

    /// Clean up the backend.
    pub fn clean(&self, project: &Project) -> eyre::Result<()> {
        match self {
            Self::Apple(b) => b.clean(project),
            Self::Android(b) => b.clean(project),
        }
    }

    /// Check requirements.
    pub fn check_requirements(&self, project: &Project) -> Result<(), Vec<ToolchainError>> {
        match self {
            Self::Apple(b) => b.check_requirements(project),
            Self::Android(b) => b.check_requirements(project),
        }
    }

    /// Scan for devices.
    pub async fn scan_devices(&self, cancel: CancellationToken) -> eyre::Result<Vec<DeviceInfo>> {
        match self {
            Self::Apple(b) => b.scan_devices(cancel).await,
            Self::Android(b) => b.scan_devices(cancel).await,
        }
    }
}

/// Scan and return a list of available backends for a project.
#[must_use]
pub fn scan_backends(project: &Project) -> Vec<AnyBackend> {
    let mut backends: Vec<AnyBackend> = Vec::new();
    let config = project.config();

    if config.backends.swift.is_some() {
        backends.push(AnyBackend::Apple(Apple));
    }

    if config.backends.android.is_some() {
        backends.push(AnyBackend::Android(AndroidBackend));
    }

    backends
}

/// Scan all backends for devices in parallel.
///
/// Returns a combined list of all devices found across all backends.
pub async fn scan_all_devices(
    backends: &[AnyBackend],
    cancel: CancellationToken,
) -> Vec<DeviceInfo> {
    use futures_util::future::join_all;

    let futures: Vec<_> = backends
        .iter()
        .map(|b| {
            let child = cancel.child_token();
            async move { b.scan_devices(child).await }
        })
        .collect();

    let results = join_all(futures).await;
    results
        .into_iter()
        .filter_map(Result::ok)
        .flatten()
        .collect()
}
