//! Stable fingerprints for preview build inputs.

use std::fmt;
use std::path::Path;

use color_eyre::eyre::{Context as _, Result, bail};
use sha2::{Digest as _, Sha256};
use walkdir::WalkDir;

use crate::runtime_compat::{is_preview_build_input_file, should_skip_scan_dir};

/// Stable content fingerprint for all inputs that can affect a preview build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProjectInputsFingerprint([u8; 32]);

impl fmt::Display for ProjectInputsFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

/// Fingerprint the current preview build inputs without blocking the async executor.
pub(super) async fn project_inputs_fingerprint(
    project_path: &Path,
) -> Result<ProjectInputsFingerprint> {
    let project_path = project_path.to_path_buf();
    smol::unblock(move || scan_project_inputs(&project_path))
        .await
        .wrap_err("failed to fingerprint preview build inputs")
}

fn scan_project_inputs(project_path: &Path) -> Result<ProjectInputsFingerprint> {
    if !project_path.is_dir() {
        bail!(
            "preview project directory does not exist: {}",
            project_path.display()
        );
    }

    let mut hasher = Sha256::new();
    let entries = WalkDir::new(project_path)
        .follow_links(true)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || !entry.file_type().is_dir()
                || !should_skip_scan_dir(entry.file_name())
        });

    for entry in entries {
        let entry = entry?;
        if entry.file_type().is_dir() || !is_preview_build_input_file(entry.path()) {
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }

        let relative = entry.path().strip_prefix(project_path).wrap_err_with(|| {
            format!(
                "preview input {} is outside project root {}",
                entry.path().display(),
                project_path.display()
            )
        })?;
        let relative = relative.to_str().ok_or_else(|| {
            color_eyre::eyre::eyre!(
                "preview build input path is not valid UTF-8: {}",
                relative.display()
            )
        })?;
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        let contents = std::fs::read(entry.path())?;
        hasher.update(contents.len().to_le_bytes());
        hasher.update(contents);
    }

    Ok(ProjectInputsFingerprint(hasher.finalize().into()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn fingerprint_tracks_inputs_and_ignores_preview_outputs() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        let source = src.join("lib.rs");
        fs::write(&source, "fn first() {}").unwrap();

        let original = scan_project_inputs(dir.path()).unwrap();

        fs::write(dir.path().join("preview.png"), "render output").unwrap();
        assert_eq!(scan_project_inputs(dir.path()).unwrap(), original);

        fs::write(&source, "fn other() {}").unwrap();
        assert_ne!(scan_project_inputs(dir.path()).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn fingerprint_tracks_symlinked_source_directories() {
        use std::os::unix::fs::symlink;

        let project = tempdir().unwrap();
        let sources = tempdir().unwrap();
        let source = sources.path().join("lib.rs");
        fs::write(&source, "fn first() {}").unwrap();
        symlink(sources.path(), project.path().join("src")).unwrap();

        let original = scan_project_inputs(project.path()).unwrap();
        fs::write(&source, "fn other() {}").unwrap();
        assert_ne!(scan_project_inputs(project.path()).unwrap(), original);
    }
}
