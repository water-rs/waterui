//! Composable installation system with parallel execution support.
//!
//! # Example
//!
//! ```ignore
//! use waterui_cli::toolchain::installation::*;
//!
//! let install = Rustup.then(
//!     RustTarget::new("aarch64-linux-android")
//!         .and(RustTarget::new("aarch64-apple-ios"))
//! );
//!
//! // With progress tracking
//! let progress = Progress::new(|name, status| {
//!     println!("{name}: {status:?}");
//! });
//! let report = install.install(progress).await?;
//!
//! // Or without progress tracking
//! let report = install.install(Progress::noop()).await?;
//! ```

use std::{
    fmt::{self, Display},
    future::Future,
    marker::PhantomData,
    sync::Arc,
};

use serde::Serialize;

use super::ToolchainError;

// ============================================================================
// Progress Tracking
// ============================================================================

/// Progress tracker for installations.
///
/// Reports real-time progress updates (start, percentage, done, failed).
/// Clone to share across parallel tasks.
#[derive(Clone)]
pub struct Progress {
    inner: Arc<dyn Fn(&str, &Status) + Send + Sync>,
}

/// Status update for a task.
#[derive(Debug, Clone)]
pub enum Status {
    /// Task started.
    Started,
    /// Progress percentage (0-100) with message.
    Progress { percent: u8, message: String },
    /// Task completed.
    Done(String),
    /// Task failed.
    Failed(String),
}

impl Progress {
    /// Create a progress tracker with a callback.
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(&str, &Status) + Send + Sync + 'static,
    {
        Self {
            inner: Arc::new(callback),
        }
    }

    /// Create a no-op progress tracker.
    pub fn noop() -> Self {
        Self::new(|_, _| {})
    }

    /// Report task started.
    pub fn start(&self, name: &str) {
        (self.inner)(name, &Status::Started);
    }

    /// Report progress (0-100%).
    pub fn update(&self, name: &str, percent: u8, message: impl Into<String>) {
        (self.inner)(name, &Status::Progress {
            percent: percent.min(100),
            message: message.into(),
        });
    }

    /// Report task done.
    pub fn done(&self, name: &str, message: impl Into<String>) {
        (self.inner)(name, &Status::Done(message.into()));
    }

    /// Report task failed.
    pub fn fail(&self, name: &str, error: impl Into<String>) {
        (self.inner)(name, &Status::Failed(error.into()));
    }
}

impl std::fmt::Debug for Progress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Progress").finish_non_exhaustive()
    }
}

// ============================================================================
// Installation Trait
// ============================================================================

/// A pending installation.
pub trait Installation: Display + Send + Sized {
    /// Future type returned by `install()`.
    type Future: Future<Output = Result<InstallationReport, ToolchainError>> + Send;

    /// Execute the installation.
    fn install(self, progress: Progress) -> Self::Future;

    /// Description of what will be installed.
    fn description(&self) -> &str;

    /// Run another installation after this one.
    fn then<I: Installation>(self, next: I) -> Sequence<Self, I> {
        Sequence { first: self, second: next }
    }

    /// Run another installation in parallel.
    fn and<I: Installation>(self, other: I) -> Parallel<Self, I> {
        Parallel { left: self, right: other }
    }
}

// ============================================================================
// Combinators
// ============================================================================

/// Sequential: run `first` then `second`.
#[derive(Debug)]
pub struct Sequence<A, B> {
    pub first: A,
    pub second: B,
}

impl<A: Display, B: Display> Display for Sequence<A, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.first)?;
        write!(f, "{}", self.second)
    }
}

impl<A: Installation, B: Installation> Installation for Sequence<A, B> {
    type Future = impl Future<Output = Result<InstallationReport, ToolchainError>> + Send;

    fn description(&self) -> &str {
        "sequential installation"
    }

    fn install(self, progress: Progress) -> Self::Future {
        async move {
            let mut report = self.first.install(progress.clone()).await?;
            report.merge(self.second.install(progress).await?);
            Ok(report)
        }
    }
}

/// Parallel: run `left` and `right` concurrently.
#[derive(Debug)]
pub struct Parallel<A, B> {
    pub left: A,
    pub right: B,
}

impl<A: Display, B: Display> Display for Parallel<A, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "┬ {}", self.left)?;
        write!(f, "└ {}", self.right)
    }
}

impl<A: Installation, B: Installation> Installation for Parallel<A, B> {
    type Future = impl Future<Output = Result<InstallationReport, ToolchainError>> + Send;

    fn description(&self) -> &str {
        "parallel installation"
    }

    fn install(self, progress: Progress) -> Self::Future {
        async move {
            let (left, right) = tokio::join!(
                self.left.install(progress.clone()),
                self.right.install(progress)
            );
            let mut report = left?;
            report.merge(right?);
            Ok(report)
        }
    }
}

/// Run multiple installations in parallel.
#[derive(Debug)]
pub struct Many<I> {
    pub installations: Vec<I>,
}

impl<I> Many<I> {
    pub fn new(iter: impl IntoIterator<Item = I>) -> Self {
        Self { installations: iter.into_iter().collect() }
    }

    pub fn is_empty(&self) -> bool {
        self.installations.is_empty()
    }

    pub fn len(&self) -> usize {
        self.installations.len()
    }
}

impl<I: Display> Display for Many<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let len = self.installations.len();
        for (i, inst) in self.installations.iter().enumerate() {
            if i == len - 1 {
                write!(f, "└ {inst}")?;
            } else {
                writeln!(f, "├ {inst}")?;
            }
        }
        Ok(())
    }
}

impl<I: Installation> Installation for Many<I> {
    type Future = impl Future<Output = Result<InstallationReport, ToolchainError>> + Send;

    fn description(&self) -> &str {
        "parallel installations"
    }

    fn install(self, progress: Progress) -> Self::Future {
        async move {
            use futures_util::future::try_join_all;

            let futures: Vec<_> = self.installations
                .into_iter()
                .map(|inst| inst.install(progress.clone()))
                .collect();

            let results = try_join_all(futures).await?;
            let mut report = InstallationReport::empty();
            for r in results {
                report.merge(r);
            }
            Ok(report)
        }
    }
}

// ============================================================================
// Empty
// ============================================================================

/// An installation that does nothing.
#[derive(Debug)]
pub struct Empty<T = ()>(PhantomData<T>);

impl<T> Empty<T> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T> Default for Empty<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Display for Empty<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(nothing to install)")
    }
}

impl<T: Send> Installation for Empty<T> {
    type Future = std::future::Ready<Result<InstallationReport, ToolchainError>>;

    fn description(&self) -> &str {
        "nothing"
    }

    fn install(self, _: Progress) -> Self::Future {
        std::future::ready(Ok(InstallationReport::empty()))
    }
}

// ============================================================================
// Report
// ============================================================================

/// Report of a completed installation.
#[derive(Debug, Clone, Serialize)]
pub struct InstallationReport {
    pub completed: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl InstallationReport {
    pub fn empty() -> Self {
        Self { completed: Vec::new(), warnings: Vec::new() }
    }

    pub fn completed(step: impl Into<String>) -> Self {
        Self { completed: vec![step.into()], warnings: Vec::new() }
    }

    pub fn add_completed(&mut self, step: impl Into<String>) {
        self.completed.push(step.into());
    }

    pub fn add_warning(&mut self, warning: impl Into<String>) {
        self.warnings.push(warning.into());
    }

    pub fn merge(&mut self, other: Self) {
        self.completed.extend(other.completed);
        self.warnings.extend(other.warnings);
    }
}

impl Display for InstallationReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.completed.is_empty() && self.warnings.is_empty() {
            return write!(f, "Nothing installed");
        }
        for step in &self.completed {
            writeln!(f, "  ✓ {step}")?;
        }
        for warning in &self.warnings {
            writeln!(f, "  ⚠ {warning}")?;
        }
        Ok(())
    }
}

impl crate::output::Report for InstallationReport {}
