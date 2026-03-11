use std::path::{Path, PathBuf};

use crate::Snapshot;

/// Centralized artifact output helper for WaterUI tests.
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

    /// Saves one snapshot using WaterUI's canonical artifact layout.
    pub fn save_snapshot(
        &self,
        case: impl AsRef<str>,
        stage: impl AsRef<str>,
        snapshot: &Snapshot,
    ) -> PathBuf {
        let path = self.snapshot_path(case, stage);
        snapshot
            .save_png(&path)
            .expect("TestArtifacts::save_snapshot: snapshot PNG should be writable");
        path
    }
}

/// Returns the global artifact root used by WaterUI tests.
#[must_use]
pub fn artifact_root() -> PathBuf {
    std::env::var_os("WATERUI_TEST_ARTIFACTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("waterui-testing-artifacts"))
}
