//! Build system

use std::path::{Path, PathBuf};

use smol::process::Command;
use target_lexicon::Triple;

pub struct RustBuild {
    path: PathBuf,
    triple: Triple,
}

pub enum RustBuildError {
    /// Failed to execute cargo build.
    FailToExecuteCargoBuild(std::io::Error),

    FailToBuildRustLibrary(std::io::Error),
}

impl RustBuild {
    pub fn new(path: impl AsRef<Path>, triple: Triple) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            triple,
        }
    }

    /// Build a `.a` or `.so` library for linking.
    ///
    /// Will produce debug symbols and less optimizations for faster builds.
    pub async fn dev_build(&self) -> Result<PathBuf, RustBuildError> {
        // cargo rustc --lib -- --crate-type staticlib

        let mut command = Command::new("cargo");

        let command = command
            .arg("rustc")
            .arg("--lib")
            .args(["--target", self.triple.to_string().as_str()])
            .arg("--lib")
            .args(["--", "--crate-type", "staticlib"]);

        command
            .status()
            .await
            .map_err(|e| RustBuildError::FailToExecuteCargoBuild(e))?;

        let path = self
            .path
            .join("target")
            .join(self.triple.to_string())
            .join("debug")
            .join(format!(
                "lib{}.a",
                self.path.file_name().unwrap().to_string_lossy()
            ));

        todo!()
    }

    /// Build a `.a` or `.so` library for linking.
    pub async fn release_build(&self) -> PathBuf {
        todo!()
    }

    /// Build a hot-reloadable `.dylib` library.
    pub async fn build_hot_reload_lib(&self) -> PathBuf {
        todo!()
    }
}
