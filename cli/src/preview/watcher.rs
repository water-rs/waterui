//! File change detection for preview caching.
//!
//! Watches project directories and tracks modification times to determine
//! when a rebuild is needed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use color_eyre::eyre::Result;
use color_eyre::eyre::Context as _;

/// Watches project directories for changes.
#[derive(Debug, Default)]
pub struct ProjectWatcher {
    /// Cached modification times per project path.
    mtimes: HashMap<PathBuf, SystemTime>,
}

#[derive(Debug, Clone, Copy)]
/// Snapshot of a project's last-known modification time.
pub struct ProjectStamp {
    /// Latest modification time across relevant project files.
    pub mtime: SystemTime,
    /// Whether the project changed since the last stamp.
    pub changed: bool,
}

impl ProjectWatcher {
    /// Create a new project watcher.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if a project has changed since the last check.
    ///
    /// Returns `true` if the project has been modified or is being checked for the first time.
    ///
    /// # Errors
    /// Returns an error if the project directory cannot be scanned.
    pub async fn stamp(&mut self, project_path: &Path) -> Result<ProjectStamp> {
        let path = project_path.to_path_buf();
        let current_mtime = smol::unblock(move || get_latest_mtime(&path))
            .await
            .wrap_err("Failed to scan project modification time")?;
        let cached_mtime = self.mtimes.get(project_path);

        let changed = cached_mtime != Some(&current_mtime);
        self.mtimes
            .insert(project_path.to_path_buf(), current_mtime);

        Ok(ProjectStamp {
            mtime: current_mtime,
            changed,
        })
    }

    /// Check if a project has changed since the last check.
    ///
    /// Returns `true` if the project has been modified or is being checked for the first time.
    ///
    /// # Errors
    /// Returns an error if the project directory cannot be scanned.
    pub async fn has_changed(&mut self, project_path: &Path) -> Result<bool> {
        Ok(self.stamp(project_path).await?.changed)
    }

    /// Clear cached state for a project.
    pub fn invalidate(&mut self, project_path: &Path) {
        self.mtimes.remove(project_path);
    }

    /// Clear all cached state.
    pub fn clear(&mut self) {
        self.mtimes.clear();
    }
}

/// Get the latest modification time from a project directory.
///
/// Scans:
/// - `src/` directory recursively
/// - `Cargo.toml`
/// - `Water.toml`
fn get_latest_mtime(project_path: &Path) -> Result<SystemTime> {
    let mut latest = SystemTime::UNIX_EPOCH;

    // Check Cargo.toml
    let cargo_toml = project_path.join("Cargo.toml");
    if cargo_toml.exists() {
        if let Ok(metadata) = std::fs::metadata(&cargo_toml) {
            if let Ok(mtime) = metadata.modified() {
                if mtime > latest {
                    latest = mtime;
                }
            }
        }
    }

    // Check Water.toml
    let water_toml = project_path.join("Water.toml");
    if water_toml.exists() {
        if let Ok(metadata) = std::fs::metadata(&water_toml) {
            if let Ok(mtime) = metadata.modified() {
                if mtime > latest {
                    latest = mtime;
                }
            }
        }
    }

    // Scan src/ directory recursively
    let src_dir = project_path.join("src");
    if src_dir.exists() {
        latest = scan_directory_mtime(&src_dir, latest)?;
    }

    Ok(latest)
}

/// Recursively scan a directory for the latest modification time.
fn scan_directory_mtime(dir: &Path, mut latest: SystemTime) -> Result<SystemTime> {
    let entries = std::fs::read_dir(dir)?;

    for entry in entries.flatten() {
        let path = entry.path();
        let metadata = entry.metadata()?;

        if metadata.is_dir() {
            latest = scan_directory_mtime(&path, latest)?;
        } else if metadata.is_file() {
            // Only check Rust source files
            if path.extension().is_some_and(|ext| ext == "rs") {
                if let Ok(mtime) = metadata.modified() {
                    if mtime > latest {
                        latest = mtime;
                    }
                }
            }
        }
    }

    Ok(latest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_watcher_detects_changes() {
        use smol::block_on;

        let dir = tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        let file = src.join("lib.rs");
        fs::write(&file, "fn main() {}").unwrap();

        let mut watcher = ProjectWatcher::new();

        // First check should always return true (new project)
        assert!(block_on(watcher.has_changed(dir.path())).unwrap());

        // Second check without changes should return false
        assert!(!block_on(watcher.has_changed(dir.path())).unwrap());

        // Modify file
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&file, "fn main() { println!(\"hello\"); }").unwrap();

        // Should detect change
        assert!(block_on(watcher.has_changed(dir.path())).unwrap());
    }
}
