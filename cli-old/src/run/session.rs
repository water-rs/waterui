//! Run session persistence for `water run again`.
//!
//! This module handles saving and loading the last run configuration,
//! allowing users to quickly re-run their previous command with `water run again`.

use std::{
    fs::{self, File},
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use color_eyre::eyre::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{platform::PlatformKind, project::Project};

const LAST_RUN_FILE: &str = "last-run.json";

/// Snapshot of a run configuration that can be replayed.
#[derive(Debug, Clone)]
pub struct LastRunSnapshot {
    /// Target platform
    pub platform: PlatformKind,
    /// Device name or identifier
    pub device: Option<String>,
    /// Whether release mode was used
    pub release: bool,
    /// Whether sccache was enabled
    pub enable_sccache: bool,
    /// Whether mold linker was requested
    pub mold: bool,
    /// Unix timestamp when the run occurred
    pub timestamp: u64,
}

impl LastRunSnapshot {
    /// Create a new snapshot with the current timestamp.
    #[must_use]
    pub fn new(
        platform: PlatformKind,
        device: Option<String>,
        release: bool,
        enable_sccache: bool,
        mold: bool,
    ) -> Self {
        Self {
            platform,
            device,
            release,
            enable_sccache,
            mold,
            timestamp: current_timestamp(),
        }
    }
}

/// Save the last run configuration to disk.
///
/// # Errors
/// Returns an error if the file cannot be written.
pub fn persist(project: &Project, snapshot: &LastRunSnapshot) -> Result<()> {
    let path = last_run_path(project);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = File::create(&path)?;
    let record = SerializableSnapshot::from(snapshot.clone());
    serde_json::to_writer_pretty(file, &record)?;
    Ok(())
}

/// Load the last run configuration from disk.
///
/// # Errors
/// Returns an error if no previous run exists or the file cannot be parsed.
pub fn load(project: &Project) -> Result<LastRunSnapshot> {
    let path = last_run_path(project);
    let file = File::open(&path).with_context(|| {
        format!(
            "No previous run recorded for {}. Run `water run` first.",
            project.root().display()
        )
    })?;

    let record: SerializableSnapshot = serde_json::from_reader(file).with_context(|| {
        format!(
            "Failed to parse last run configuration at {}",
            path.display()
        )
    })?;

    Ok(record.into())
}

fn last_run_path(project: &Project) -> PathBuf {
    project.root().join(".water").join(LAST_RUN_FILE)
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

// =============================================================================
// Serialization
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializableSnapshot {
    platform: StoredPlatform,
    device: Option<String>,
    release: bool,
    enable_sccache: bool,
    mold: bool,
    timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredPlatform {
    Web,
    Macos,
    Ios,
    Ipados,
    Watchos,
    Tvos,
    Visionos,
    Android,
}

impl From<PlatformKind> for StoredPlatform {
    fn from(platform: PlatformKind) -> Self {
        match platform {
            PlatformKind::Web => Self::Web,
            PlatformKind::Macos => Self::Macos,
            PlatformKind::Ios => Self::Ios,
            PlatformKind::Ipados => Self::Ipados,
            PlatformKind::Watchos => Self::Watchos,
            PlatformKind::Tvos => Self::Tvos,
            PlatformKind::Visionos => Self::Visionos,
            PlatformKind::Android => Self::Android,
        }
    }
}

impl From<StoredPlatform> for PlatformKind {
    fn from(value: StoredPlatform) -> Self {
        match value {
            StoredPlatform::Web => Self::Web,
            StoredPlatform::Macos => Self::Macos,
            StoredPlatform::Ios => Self::Ios,
            StoredPlatform::Ipados => Self::Ipados,
            StoredPlatform::Watchos => Self::Watchos,
            StoredPlatform::Tvos => Self::Tvos,
            StoredPlatform::Visionos => Self::Visionos,
            StoredPlatform::Android => Self::Android,
        }
    }
}

impl From<LastRunSnapshot> for SerializableSnapshot {
    fn from(value: LastRunSnapshot) -> Self {
        Self {
            platform: value.platform.into(),
            device: value.device,
            release: value.release,
            enable_sccache: value.enable_sccache,
            mold: value.mold,
            timestamp: value.timestamp,
        }
    }
}

impl From<SerializableSnapshot> for LastRunSnapshot {
    fn from(value: SerializableSnapshot) -> Self {
        Self {
            platform: value.platform.into(),
            device: value.device,
            release: value.release,
            enable_sccache: value.enable_sccache,
            mold: value.mold,
            timestamp: value.timestamp,
        }
    }
}
