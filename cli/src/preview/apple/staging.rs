//! Completed runtime preparation, keyed by the Cargo outputs and prepared files.
//!
//! Cargo rewrites an output's mtime when it rebuilds it. Record the prepared
//! executable too: install-name changes are outputs, not changes to replay on
//! every invocation. Missing or replaced staged files invalidate the record.

use std::path::{Path, PathBuf};

use eyre::Result;
use serde::{Deserialize, Serialize};

use crate::project_model::facts::FileStamp;

/// Prepare only runtime files whose Cargo inputs or staged outputs changed.
pub(super) async fn prepare_runtime(
    project: &crate::project::Project,
    built: &crate::build::BuiltTarget,
    triple: &target_lexicon::Triple,
) -> Result<()> {
    use crate::apple::dynamic_runtime;
    use crate::build::RustDynamicLibraries;

    let executable = built.executable()?;
    let record_path = project
        .apple_preview_crate_path()
        .join(format!("runtime-{triple}.json"));
    if !is_current(&record_path, executable, built.shared_runtime()?).await? {
        let libraries = RustDynamicLibraries::resolve(built, triple, project).await?;
        let profile_dir = &built.profile_dir;
        libraries.stage(profile_dir).await?;
        let staged_runtime = libraries.stage_apple_canonical(profile_dir).await?;
        dynamic_runtime::retarget_module(project.host(), executable, &staged_runtime).await?;
        dynamic_runtime::prepare_host_runtime(project.host(), &staged_runtime).await?;
        let mut paths = vec![
            executable.to_path_buf(),
            built.shared_runtime()?.to_path_buf(),
            staged_runtime,
        ];
        paths.extend(libraries.staged_files(profile_dir));
        record(&record_path, paths).await?;
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct PreparedFile {
    path: PathBuf,
    stamp: FileStamp,
}

pub(super) async fn is_current(record: &Path, executable: &Path, runtime: &Path) -> Result<bool> {
    let bytes = match smol::fs::read(record).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let files: Vec<PreparedFile> = serde_json::from_slice(&bytes)?;
    if files.first().map(|file| file.path.as_path()) != Some(executable)
        || files.get(1).map(|file| file.path.as_path()) != Some(runtime)
    {
        return Ok(false);
    }
    smol::unblock(move || {
        for file in files {
            if FileStamp::of(&file.path)? != Some(file.stamp) {
                return Ok(false);
            }
        }
        Ok(true)
    })
    .await
}

pub(super) async fn record(record: &Path, paths: Vec<PathBuf>) -> Result<()> {
    let files = smol::unblock(move || {
        paths
            .into_iter()
            .map(|path| {
                let stamp = FileStamp::of(&path)?.ok_or_else(|| {
                    eyre::eyre!("prepared runtime file is missing: {}", path.display())
                })?;
                Ok(PreparedFile { path, stamp })
            })
            .collect::<Result<Vec<_>>>()
    })
    .await?;
    let bytes = serde_json::to_vec(&files)?;
    let pending = record.with_extension("json.pending");
    smol::fs::write(&pending, bytes).await?;
    smol::fs::rename(pending, record).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preparation_reuses_outputs_and_invalidates_changes() {
        smol::block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let executable = dir.path().join("preview");
            let runtime = dir.path().join("runtime");
            let staged = dir.path().join("staged");
            let record_path = dir.path().join("record.json");
            for path in [&executable, &runtime, &staged] {
                std::fs::write(path, b"artifact").unwrap();
            }
            assert!(
                !is_current(&record_path, &executable, &runtime)
                    .await
                    .unwrap()
            );
            let paths = vec![executable.clone(), runtime.clone(), staged.clone()];
            record(&record_path, paths.clone()).await.unwrap();
            assert!(
                is_current(&record_path, &executable, &runtime)
                    .await
                    .unwrap()
            );
            for path in [&executable, &runtime, &staged] {
                std::fs::write(path, b"rebuilt artifact").unwrap();
                assert!(
                    !is_current(&record_path, &executable, &runtime)
                        .await
                        .unwrap()
                );
                record(&record_path, paths.clone()).await.unwrap();
            }
            std::fs::remove_file(staged).unwrap();
            assert!(
                !is_current(&record_path, &executable, &runtime)
                    .await
                    .unwrap()
            );
        });
    }
}
