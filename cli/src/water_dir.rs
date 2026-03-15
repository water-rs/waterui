//! Management of the playground managed backend cache.
//!
//! Playground projects store generated backends in a per-project cache under
//! `~/.water/project_cache/<project-path-hash>/` instead of scattering `.water`
//! directories into user projects.

use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use color_eyre::eyre::{self, WrapErr};
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use smol::fs;
use smol::stream::StreamExt;
use tracing::{info, warn};

/// The CLI commit hash embedded at build time.
pub const CLI_COMMIT: &str = env!("WATERUI_CLI_COMMIT");

const PROJECT_CACHE_DIR_NAME: &str = "project_cache";
const LEGACY_LOCAL_WATER_DIR_NAME: &str = ".water";
const METADATA_FILE_NAME: &str = "metadata.toml";
const STALE_CACHE_RETENTION_SECS: u64 = 30 * 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CacheMetadata {
    project_root: String,
    cli_commit: String,
    last_used_unix_seconds: u64,
}

/// Return the global root used for playground managed backend caches.
pub fn playground_cache_root() -> eyre::Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| eyre::eyre!("Could not determine home directory"))?;
    Ok(home.join(".water").join(PROJECT_CACHE_DIR_NAME))
}

/// Return the managed backend cache directory for a playground project.
pub fn playground_cache_dir(project_root: &Path) -> eyre::Result<PathBuf> {
    let project_root = canonicalize_project_root(project_root)?;
    let cache_root = playground_cache_root()?;
    Ok(playground_cache_dir_in(&project_root, &cache_root))
}

/// Ensure the playground managed backend cache is valid for the current CLI version.
///
/// Returns the absolute cache directory path for the project.
pub async fn ensure_valid(project_root: &Path) -> eyre::Result<PathBuf> {
    let project_root = canonicalize_project_root(project_root)?;
    let cache_root = playground_cache_root()?;
    ensure_valid_in(&project_root, &cache_root).await
}

/// Remove the managed backend cache for a playground project.
pub async fn remove_playground_cache(project_root: &Path) -> eyre::Result<()> {
    let project_root = canonicalize_project_root(project_root)?;
    let cache_root = playground_cache_root()?;
    remove_playground_cache_in(&project_root, &cache_root).await
}

async fn ensure_valid_in(project_root: &Path, cache_root: &Path) -> eyre::Result<PathBuf> {
    cleanup_stale_caches(cache_root, project_root).await?;
    let water_dir = playground_cache_dir_in(project_root, cache_root);
    remove_legacy_local_water_dir(project_root).await?;

    if water_dir.exists() {
        let should_clean = match read_metadata(&water_dir).await {
            Ok(metadata) => {
                metadata.project_root != project_root.display().to_string()
                    || metadata.cli_commit != CLI_COMMIT
            }
            Err(_) => true,
        };

        if should_clean {
            info!(
                "Playground managed cache changed shape, cleaning {}",
                water_dir.display()
            );
            fs::remove_dir_all(&water_dir).await?;
        }
    }

    fs::create_dir_all(&water_dir).await?;
    write_metadata(
        &water_dir,
        &CacheMetadata {
            project_root: project_root.display().to_string(),
            cli_commit: CLI_COMMIT.to_string(),
            last_used_unix_seconds: now_unix_seconds()?,
        },
    )
    .await?;

    Ok(water_dir)
}

async fn remove_playground_cache_in(project_root: &Path, cache_root: &Path) -> eyre::Result<()> {
    let water_dir = playground_cache_dir_in(project_root, cache_root);
    if water_dir.exists() {
        fs::remove_dir_all(&water_dir).await?;
    }
    remove_legacy_local_water_dir(project_root).await
}

fn playground_cache_dir_in(project_root: &Path, cache_root: &Path) -> PathBuf {
    cache_root.join(project_root_hash(project_root))
}

fn project_root_hash(project_root: &Path) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(project_root.display().to_string().as_bytes());
    format!("{:x}", hasher.finalize())
}

fn canonicalize_project_root(project_root: &Path) -> eyre::Result<PathBuf> {
    project_root.canonicalize().wrap_err_with(|| {
        format!(
            "Failed to canonicalize project root {}",
            project_root.display()
        )
    })
}

fn metadata_path(water_dir: &Path) -> PathBuf {
    water_dir.join(METADATA_FILE_NAME)
}

async fn read_metadata(water_dir: &Path) -> eyre::Result<CacheMetadata> {
    let metadata_path = metadata_path(water_dir);
    let contents = fs::read_to_string(&metadata_path)
        .await
        .wrap_err_with(|| format!("Failed to read cache metadata {}", metadata_path.display()))?;
    toml::from_str(&contents)
        .wrap_err_with(|| format!("Failed to parse cache metadata {}", metadata_path.display()))
}

async fn write_metadata(water_dir: &Path, metadata: &CacheMetadata) -> eyre::Result<()> {
    let metadata_path = metadata_path(water_dir);
    let contents = toml::to_string(metadata).wrap_err("Failed to serialize cache metadata")?;
    fs::write(&metadata_path, contents)
        .await
        .wrap_err_with(|| format!("Failed to write cache metadata {}", metadata_path.display()))
}

async fn remove_legacy_local_water_dir(project_root: &Path) -> eyre::Result<()> {
    let legacy_dir = project_root.join(LEGACY_LOCAL_WATER_DIR_NAME);
    if legacy_dir.exists() {
        info!(
            "Removing legacy local playground cache at {}",
            legacy_dir.display()
        );
        fs::remove_dir_all(&legacy_dir).await?;
    }
    Ok(())
}

async fn cleanup_stale_caches(cache_root: &Path, current_project_root: &Path) -> eyre::Result<()> {
    fs::create_dir_all(cache_root).await?;
    let current_cache_dir = playground_cache_dir_in(current_project_root, cache_root);
    let now = now_unix_seconds()?;
    let mut entries = fs::read_dir(cache_root).await?;

    while let Some(entry) = entries.next().await {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type().await?;
        if !file_type.is_dir() || path == current_cache_dir {
            continue;
        }

        let should_remove = match read_metadata(&path).await {
            Ok(metadata) => {
                let project_root = PathBuf::from(&metadata.project_root);
                !project_root.exists()
                    || now.saturating_sub(metadata.last_used_unix_seconds)
                        > STALE_CACHE_RETENTION_SECS
            }
            Err(error) => {
                warn!(
                    "Removing stale playground cache with invalid metadata at {}: {error}",
                    path.display()
                );
                true
            }
        };

        if should_remove {
            if let Err(error) = fs::remove_dir_all(&path).await {
                warn!(
                    "Failed to remove stale playground cache {}: {error}",
                    path.display()
                );
            }
        }
    }

    Ok(())
}

fn now_unix_seconds() -> eyre::Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .wrap_err("System clock is before UNIX_EPOCH")?
        .as_secs())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::tempdir;

    use super::{
        CLI_COMMIT, CacheMetadata, ensure_valid_in, metadata_path, now_unix_seconds,
        playground_cache_dir_in, remove_playground_cache_in,
    };

    #[test]
    fn ensure_valid_uses_global_project_cache_dir() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let cache_root = tempdir().expect("cache root");

            let water_dir = ensure_valid_in(project.path(), cache_root.path())
                .await
                .expect("ensure cache");

            assert!(water_dir.starts_with(cache_root.path()));
            assert_ne!(water_dir, project.path().join(".water"));
            assert!(water_dir.exists());
        });
    }

    #[test]
    fn ensure_valid_removes_legacy_local_water_dir() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let cache_root = tempdir().expect("cache root");
            let legacy_dir = project.path().join(".water");
            smol::fs::create_dir_all(&legacy_dir)
                .await
                .expect("create legacy dir");
            smol::fs::write(legacy_dir.join("stale"), b"stale")
                .await
                .expect("write legacy file");

            ensure_valid_in(project.path(), cache_root.path())
                .await
                .expect("ensure cache");

            assert!(!legacy_dir.exists());
        });
    }

    #[test]
    fn ensure_valid_cleans_cache_when_cli_commit_changes() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let cache_root = tempdir().expect("cache root");
            let water_dir = playground_cache_dir_in(project.path(), cache_root.path());
            smol::fs::create_dir_all(&water_dir)
                .await
                .expect("create cache dir");
            smol::fs::write(water_dir.join("stale"), b"stale")
                .await
                .expect("write stale file");
            let stale_metadata = CacheMetadata {
                project_root: project.path().display().to_string(),
                cli_commit: String::from("old-commit"),
                last_used_unix_seconds: 1,
            };
            let stale_contents =
                toml::to_string(&stale_metadata).expect("serialize stale metadata");
            smol::fs::write(metadata_path(&water_dir), stale_contents)
                .await
                .expect("write stale metadata");

            ensure_valid_in(project.path(), cache_root.path())
                .await
                .expect("ensure cache");

            assert!(!water_dir.join("stale").exists());
            let fresh_metadata = super::read_metadata(&water_dir)
                .await
                .expect("read metadata");
            assert_eq!(fresh_metadata.cli_commit, CLI_COMMIT);
        });
    }

    #[test]
    fn ensure_valid_cleans_stale_orphaned_caches() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let cache_root = tempdir().expect("cache root");
            let stale_cache = cache_root.path().join("stale-cache");
            smol::fs::create_dir_all(&stale_cache)
                .await
                .expect("create stale cache");
            let stale_metadata = CacheMetadata {
                project_root: Path::new("/definitely/missing/project")
                    .display()
                    .to_string(),
                cli_commit: CLI_COMMIT.to_string(),
                last_used_unix_seconds: now_unix_seconds().expect("now"),
            };
            let stale_contents =
                toml::to_string(&stale_metadata).expect("serialize stale metadata");
            smol::fs::write(metadata_path(&stale_cache), stale_contents)
                .await
                .expect("write stale metadata");

            ensure_valid_in(project.path(), cache_root.path())
                .await
                .expect("ensure cache");

            assert!(!stale_cache.exists());
        });
    }

    #[test]
    fn remove_playground_cache_removes_global_cache_dir() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let cache_root = tempdir().expect("cache root");
            let water_dir = ensure_valid_in(project.path(), cache_root.path())
                .await
                .expect("ensure cache");

            remove_playground_cache_in(project.path(), cache_root.path())
                .await
                .expect("remove cache");

            assert!(!water_dir.exists());
        });
    }
}
