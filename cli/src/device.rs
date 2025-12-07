use color_eyre::eyre::{self, Result};

use crate::{debug::crash::CrashReport, platform::Platform, project::Project};

/// Result of a device build operation.
#[derive(Debug, Clone)]
pub struct DeviceBuildResult {
    /// Path to the built library artifact (e.g., `libwaterui_app.a` or .so)
    pub library_path: std::path::PathBuf,
    /// Target triple that was built
    pub target_triple: &'static str,
}

pub struct RunOptions {}

#[derive(Debug, thiserror::Error)]
pub enum FailToRun {
    #[error("Failed to launch application: {0}")]
    Lanuch(eyre::Report),
    #[error("Application crashed {0}")]
    Crash(CrashReport),
}

pub trait Device: Send + Sync {
    type Platform: Platform + Clone;

    /// Prepare the device for building and running apps.
    fn prepare(&self) -> impl Future<Output = Result<(), eyre::Report>> + Send;

    fn run(
        &self,
        project: &Project,
        options: &RunOptions,
    ) -> impl Future<Output = Result<(), FailToRun>> + Send;

    fn platform(&self) -> &Self::Platform;
}
