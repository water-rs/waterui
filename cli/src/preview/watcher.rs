//! File change detection for preview caching.
//!
//! Watches project directories and tracks modification times to determine
//! when a rebuild is needed.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use color_eyre::eyre::Context as _;
use color_eyre::eyre::Result;

use crate::runtime_compat::{is_preview_build_input_file, should_skip_scan_dir};

/// Watches project directories for changes.
#[derive(Debug, Default)]
pub struct ProjectWatcher {
    /// Cached project snapshots per project path.
    snapshots: HashMap<PathBuf, ProjectSnapshot>,
}

#[derive(Debug, Clone, Copy)]
/// Snapshot of a project's last-known modification time.
pub struct ProjectStamp {
    /// Latest modification time across relevant project files.
    pub mtime: SystemTime,
    /// Whether the project changed since the last stamp.
    pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProjectSnapshot {
    latest_mtime: SystemTime,
    fingerprint: u64,
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
        let current_snapshot = smol::unblock(move || get_project_snapshot(&path))
            .await
            .wrap_err("Failed to scan project snapshot")?;
        let cached_snapshot = self.snapshots.get(project_path);

        let changed = cached_snapshot != Some(&current_snapshot);
        self.snapshots
            .insert(project_path.to_path_buf(), current_snapshot);

        Ok(ProjectStamp {
            mtime: current_snapshot.latest_mtime,
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
        self.snapshots.remove(project_path);
    }

    /// Clear all cached state.
    pub fn clear(&mut self) {
        self.snapshots.clear();
    }
}

/// Compute a snapshot fingerprint from project files.
///
/// This tracks both the latest mtime and a structural fingerprint so additions/removals
/// with older mtimes are still detected.
fn get_project_snapshot(project_path: &Path) -> Result<ProjectSnapshot> {
    let mut latest_mtime = SystemTime::UNIX_EPOCH;
    let mut hasher = DefaultHasher::new();

    if project_path.exists() {
        scan_directory_snapshot(project_path, project_path, &mut latest_mtime, &mut hasher)?;
    }

    Ok(ProjectSnapshot {
        latest_mtime,
        fingerprint: hasher.finish(),
    })
}

/// Recursively scan a directory for snapshot fingerprinting.
fn scan_directory_snapshot(
    root: &Path,
    dir: &Path,
    latest_mtime: &mut SystemTime,
    hasher: &mut DefaultHasher,
) -> Result<()> {
    let entries = std::fs::read_dir(dir)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;
        let file_name = entry.file_name();

        if metadata.is_dir() {
            if should_skip_scan_dir(&file_name) {
                continue;
            }
            scan_directory_snapshot(root, &path, latest_mtime, hasher)?;
        } else if metadata.is_file() {
            if !is_preview_build_input_file(&path) {
                continue;
            }

            let mtime = metadata.modified()?;
            if mtime > *latest_mtime {
                *latest_mtime = mtime;
            }

            let relative = path
                .strip_prefix(root)
                .wrap_err_with(|| format!("Path {} was outside project root", path.display()))?;
            relative.hash(hasher);
            metadata.len().hash(hasher);
            match mtime.duration_since(SystemTime::UNIX_EPOCH) {
                Ok(duration) => {
                    duration.as_secs().hash(hasher);
                    duration.subsec_nanos().hash(hasher);
                }
                Err(err) => {
                    0u8.hash(hasher);
                    err.duration().as_secs().hash(hasher);
                    err.duration().subsec_nanos().hash(hasher);
                }
            }
        }
    }

    Ok(())
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
        fs::write(&file, "fn main() { let _ = 1; }").unwrap();

        // Should detect change
        assert!(block_on(watcher.has_changed(dir.path())).unwrap());
    }

    #[test]
    fn test_watcher_detects_non_rust_source_input() {
        use smol::block_on;

        let dir = tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        let rust_file = src.join("lib.rs");
        let shader = src.join("main.wgsl");
        fs::write(&rust_file, "fn main() {}").unwrap();
        fs::write(&shader, "// shader").unwrap();

        let mut watcher = ProjectWatcher::new();

        assert!(block_on(watcher.has_changed(dir.path())).unwrap());
        assert!(!block_on(watcher.has_changed(dir.path())).unwrap());

        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&shader, "// shader changed").unwrap();

        assert!(block_on(watcher.has_changed(dir.path())).unwrap());
    }

    #[test]
    fn test_watcher_ignores_preview_output_files() {
        use smol::block_on;

        let dir = tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        let rust_file = src.join("lib.rs");
        let output_file = dir.path().join("preview.png");
        fs::write(&rust_file, "fn main() {}").unwrap();

        let mut watcher = ProjectWatcher::new();

        assert!(block_on(watcher.has_changed(dir.path())).unwrap());
        assert!(!block_on(watcher.has_changed(dir.path())).unwrap());

        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(&output_file, "not a build input").unwrap();

        assert!(!block_on(watcher.has_changed(dir.path())).unwrap());
    }
}
