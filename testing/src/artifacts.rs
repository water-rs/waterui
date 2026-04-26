use std::path::{Path, PathBuf};

use crate::Snapshot;

/// Snapshot captured to `WaterUI`'s canonical test artifact layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedSnapshot {
    snapshot: Snapshot,
    path: PathBuf,
}

impl CapturedSnapshot {
    /// Creates a captured snapshot record from pixels and output path.
    #[must_use]
    pub const fn new(snapshot: Snapshot, path: PathBuf) -> Self {
        Self { snapshot, path }
    }

    /// Returns the captured pixels.
    #[must_use]
    pub const fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// Returns the path where the snapshot was written.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Splits the captured snapshot into pixels and output path.
    #[must_use]
    pub fn into_parts(self) -> (Snapshot, PathBuf) {
        (self.snapshot, self.path)
    }
}

/// Centralized artifact output helper for `WaterUI` tests.
#[derive(Debug, Clone)]
pub struct TestArtifacts {
    root: PathBuf,
}

impl TestArtifacts {
    /// Creates an artifact store rooted under the given suite name.
    #[must_use]
    pub fn new(suite: impl AsRef<str>) -> Self {
        let root = artifact_root().join(suite.as_ref());
        Self { root }
    }

    /// Returns the root directory used by this suite.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the directory for one logical test case.
    #[must_use]
    pub fn case_dir(&self, case: impl AsRef<str>) -> PathBuf {
        self.root.join(case.as_ref())
    }

    /// Builds the canonical PNG path for a named snapshot stage.
    #[must_use]
    pub fn snapshot_path(&self, case: impl AsRef<str>, stage: impl AsRef<str>) -> PathBuf {
        self.case_dir(case).join(format!("{}.png", stage.as_ref()))
    }

    /// Saves one snapshot and returns both the pixels and canonical artifact path.
    ///
    /// # Panics
    ///
    /// Panics if the PNG cannot be written to the artifact directory.
    pub fn capture_snapshot(
        &self,
        case: impl AsRef<str>,
        stage: impl AsRef<str>,
        snapshot: Snapshot,
    ) -> CapturedSnapshot {
        let path = self.snapshot_path(case, stage);
        snapshot
            .save_png(&path)
            .expect("TestArtifacts::capture_snapshot: snapshot PNG should be writable");
        CapturedSnapshot::new(snapshot, path)
    }

    /// Saves one snapshot using `WaterUI`'s canonical artifact layout.
    pub fn save_snapshot(
        &self,
        case: impl AsRef<str>,
        stage: impl AsRef<str>,
        snapshot: &Snapshot,
    ) -> PathBuf {
        self.capture_snapshot(case, stage, snapshot.clone())
            .into_parts()
            .1
    }
}

/// Returns the global artifact root used by `WaterUI` tests.
#[must_use]
pub fn artifact_root() -> PathBuf {
    std::env::var_os("WATERUI_TEST_ARTIFACTS_DIR").map_or_else(
        || std::env::temp_dir().join("waterui-testing-artifacts"),
        PathBuf::from,
    )
}
