//! Moving packaged artifacts into a project's output directory.

use std::{fs, io::ErrorKind, path::Path};

use eyre::{Result, WrapErr};

use crate::{device::Artifact, project::Project};

/// Move a packaged artifact out of the build cache into the project's
/// `target/package/` directory and return the artifact at its new path.
///
/// # Errors
/// Returns an error if the output directory cannot be created, the artifact
/// cannot be moved or copied, or its path has no file name.
pub async fn place_in_project(project: &Project, artifact: Artifact) -> Result<Artifact> {
    let dest_dir = project.root().join("target").join("package");
    place_in_dir(&dest_dir, artifact).await
}

pub(crate) async fn place_in_dir(dest_dir: &Path, artifact: Artifact) -> Result<Artifact> {
    let bundle_id = artifact.bundle_id().to_owned();
    let source = artifact.path().to_owned();
    let file_name = source
        .file_name()
        .ok_or_else(|| {
            eyre::eyre!(
                "packaged artifact path has no file name: {}",
                source.display()
            )
        })?
        .to_owned();
    let source_for_log = source.clone();
    let dest_dir = dest_dir.to_owned();
    let (destination, strategy) = smol::unblock(move || -> Result<(_, &'static str)> {
        fs::create_dir_all(&dest_dir).wrap_err_with(|| {
            format!("failed to create package directory {}", dest_dir.display())
        })?;
        let destination = dest_dir.join(file_name);
        if source == destination {
            return Ok((destination, "rename"));
        }
        if destination.exists() {
            remove_existing(&destination).wrap_err_with(|| {
                format!(
                    "failed to replace previous package output {}",
                    destination.display()
                )
            })?;
        }
        match fs::rename(&source, &destination) {
            Ok(()) => Ok((destination, "rename")),
            Err(error) if error.kind() == ErrorKind::CrossesDevices => {
                copy_recursively(&source, &destination).wrap_err_with(|| {
                    format!(
                        "failed to copy packaged artifact from {} to {}",
                        source.display(),
                        destination.display()
                    )
                })?;
                remove_existing(&source).wrap_err_with(|| {
                    format!("failed to remove packaged artifact {}", source.display())
                })?;
                Ok((destination, "copy"))
            }
            Err(error) => Err(error).wrap_err_with(|| {
                format!(
                    "failed to move packaged artifact from {} to {}",
                    source.display(),
                    destination.display()
                )
            }),
        }
    })
    .await?;
    tracing::info!(
        from = %source_for_log.display(),
        to = %destination.display(),
        strategy,
        "placed packaged artifact"
    );
    Ok(Artifact::new(bundle_id, destination))
}

fn remove_existing(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn copy_recursively(source: &Path, destination: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        // A macOS `.app` carries symlinks by construction — every embedded
        // framework has `Versions/Current` and the shims beside it — so a
        // bundle that crosses a device boundary must keep them as symlinks.
        // Following them would inflate the bundle and break code signing;
        // refusing them would fail every framework-bearing bundle.
        return copy_symlink(source, destination);
    }
    if metadata.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_recursively(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else {
        fs::copy(source, destination)?;
    }
    Ok(())
}

/// Recreates `source`'s symlink at `destination`, target text unchanged.
#[cfg(unix)]
fn copy_symlink(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(fs::read_link(source)?, destination)
}

/// Windows needs a privilege to create symlinks and its bundles do not rely
/// on them, so a symlink there is reported rather than silently dereferenced.
#[cfg(not(unix))]
fn copy_symlink(source: &Path, _destination: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        ErrorKind::InvalidInput,
        format!("cannot copy symlink packaged artifact {}", source.display()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// A `.app` that crosses a device boundary keeps the framework symlinks
    /// every macOS bundle is built from.
    #[cfg(unix)]
    #[test]
    fn copies_a_bundle_symlink_as_a_symlink() {
        let temp = tempdir().expect("tempdir");
        let versions = temp.path().join("App.app").join("Versions");
        fs::create_dir_all(versions.join("A")).expect("bundle");
        fs::write(versions.join("A").join("App"), b"binary").expect("bundle binary");
        std::os::unix::fs::symlink("A", versions.join("Current")).expect("framework symlink");

        let copied = temp.path().join("copied.app");
        copy_recursively(&temp.path().join("App.app"), &copied).expect("bundle must copy");

        let link = copied.join("Versions").join("Current");
        assert!(
            fs::symlink_metadata(&link)
                .expect("copied link")
                .file_type()
                .is_symlink(),
            "the copy must stay a symlink, not a second copy of the version directory"
        );
        assert_eq!(fs::read_link(&link).expect("link target"), Path::new("A"));
    }

    #[test]
    fn moves_file_and_replaces_stale_file() {
        smol::block_on(async {
            let temp = tempdir().expect("tempdir");
            let source = temp.path().join("cache").join("app");
            let destination_dir = temp.path().join("project").join("target").join("package");
            fs::create_dir_all(source.parent().expect("source parent")).expect("cache directory");
            fs::create_dir_all(&destination_dir).expect("destination directory");
            fs::write(&source, b"new").expect("source file");
            fs::write(destination_dir.join("app"), b"stale").expect("stale output");

            let artifact = Artifact::new("dev.example.app", source.clone());
            let placed = place_in_dir(&destination_dir, artifact)
                .await
                .expect("artifact must move");

            assert!(!source.exists());
            assert_eq!(placed.path(), destination_dir.join("app"));
            assert_eq!(fs::read(placed.path()).expect("destination file"), b"new");
        });
    }

    #[test]
    fn moves_directory_tree_and_replaces_stale_directory() {
        smol::block_on(async {
            let temp = tempdir().expect("tempdir");
            let source = temp.path().join("cache").join("app");
            let destination_dir = temp.path().join("project").join("target").join("package");
            fs::create_dir_all(source.join("nested")).expect("source directory");
            fs::create_dir_all(destination_dir.join("app").join("old")).expect("stale destination");
            fs::write(source.join("nested").join("data"), b"new").expect("source content");
            fs::write(
                destination_dir.join("app").join("old").join("data"),
                b"stale",
            )
            .expect("stale content");

            let artifact = Artifact::new("dev.example.app", source.clone());
            let placed = place_in_dir(&destination_dir, artifact)
                .await
                .expect("artifact must move");

            assert!(!source.exists());
            assert_eq!(
                fs::read(placed.path().join("nested").join("data")).expect("destination content"),
                b"new"
            );
            assert!(!placed.path().join("old").exists());
        });
    }
}
