//! Management of the global Water home and per-project managed backend build cache.
//!
//! Playground projects store generated backends under
//! `~/.water/build_cache/<absolute-project-path>/managed_backends/` instead of
//! scattering `.water` directories into user projects.

use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf, Prefix, PrefixComponent},
    process::Stdio,
    time::{SystemTime, UNIX_EPOCH},
};

use color_eyre::eyre::{self, WrapErr};
use serde::{Deserialize, Serialize};
use smol::fs;
use tracing::{info, warn};
use walkdir::WalkDir;

/// The CLI commit hash embedded at build time.
pub const CLI_COMMIT: &str = env!("WATERUI_CLI_COMMIT");

const BUILD_CACHE_DIR_NAME: &str = "build_cache";
const MANAGED_BACKENDS_DIR_NAME: &str = "managed_backends";
const CONFIG_FILE_NAME: &str = "config.toml";
const METADATA_FILE_NAME: &str = "metadata.toml";
const CLEANUP_LOCK_FILE_NAME: &str = ".cleanup.lock";
const LEGACY_LOCAL_WATER_DIR_NAME: &str = ".water";
const DEFAULT_BUILD_CACHE_CLEANUP_AFTER_UNUSED_DAYS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
/// Global Water CLI configuration persisted to `~/.water/config.toml`.
pub struct WaterConfig {
    /// Managed build-cache policy.
    #[serde(default)]
    pub build_cache: BuildCacheConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Cleanup policy for the global managed build cache.
pub struct BuildCacheConfig {
    /// Remove build-cache entries that have been unused for more than this many days.
    #[serde(default = "default_build_cache_cleanup_after_unused_days")]
    pub cleanup_after_unused_days: u64,
}

impl Default for BuildCacheConfig {
    fn default() -> Self {
        Self {
            cleanup_after_unused_days: default_build_cache_cleanup_after_unused_days(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CacheMetadata {
    project_root: String,
    cli_commit: String,
    last_used_unix_seconds: u64,
}

/// Summary of one managed build-cache garbage-collection pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildCacheGcSummary {
    /// Number of managed cache entries inspected, excluding the active project cache.
    pub scanned_entries: usize,
    /// Number of stale managed cache entries removed during this pass.
    pub removed_entries: usize,
}

/// Result of attempting to garbage-collect stale managed build-cache entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildCacheGcOutcome {
    /// Cleanup ran to completion and produced a removal summary.
    Ran(BuildCacheGcSummary),
    /// Cleanup did not run because another `water gc build-cache` process already holds the lock.
    SkippedAlreadyRunning,
}

const fn default_build_cache_cleanup_after_unused_days() -> u64 {
    DEFAULT_BUILD_CACHE_CLEANUP_AFTER_UNUSED_DAYS
}

/// Return the Water home directory root at `~/.water`.
///
/// # Errors
/// Returns an error if the current user's home directory cannot be determined.
pub fn water_home_dir() -> eyre::Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| eyre::eyre!("Could not determine home directory"))?;
    Ok(home.join(".water"))
}

/// Ensure `~/.water/config.toml` exists and return the parsed configuration.
///
/// # Errors
/// Returns an error if the Water home cannot be created or the config cannot be read or written.
pub async fn ensure_global_config() -> eyre::Result<WaterConfig> {
    let water_home = water_home_dir()?;
    ensure_global_config_in(&water_home).await
}

/// Return the global managed build-cache root at `~/.water/build_cache`.
///
/// # Errors
/// Returns an error if the Water config cannot be loaded or the cache root cannot be created.
pub async fn build_cache_root() -> eyre::Result<PathBuf> {
    let water_home = water_home_dir()?;
    let (_, cache_root) = resolved_build_cache_root_in(&water_home).await?;
    Ok(cache_root)
}

/// Return the managed build-cache directory for a project.
///
/// # Errors
/// Returns an error if the project root cannot be canonicalized or the global cache root cannot be resolved.
pub async fn project_build_cache_dir(project_root: &Path) -> eyre::Result<PathBuf> {
    let project_root = canonicalize_project_root(project_root)?;
    let cache_root = build_cache_root().await?;
    Ok(project_build_cache_dir_in(&project_root, &cache_root))
}

/// Ensure the managed build-cache directory exists for a project and return it.
///
/// # Errors
/// Returns an error if the project root cannot be canonicalized, config loading fails, or cache directories cannot be created.
pub async fn ensure_project_build_cache(project_root: &Path) -> eyre::Result<PathBuf> {
    let project_root = canonicalize_project_root(project_root)?;
    let water_home = water_home_dir()?;
    let (config, cache_root) = resolved_build_cache_root_in(&water_home).await?;
    if let Err(error) = spawn_build_cache_cleanup_process(&project_root).await {
        warn!(
            current_project_root = %project_root.display(),
            "Failed to spawn build-cache cleanup process: {error}"
        );
    }
    ensure_project_build_cache_in(&project_root, &cache_root, &config).await
}

/// Garbage-collect stale managed build-cache entries while preserving the current project's cache.
///
/// # Errors
/// Returns an error if the project root cannot be canonicalized, config loading fails,
/// or stale cache removal fails.
pub async fn cleanup_stale_build_caches_for_project(
    project_root: &Path,
) -> eyre::Result<BuildCacheGcOutcome> {
    let project_root = canonicalize_project_root(project_root)?;
    let water_home = water_home_dir()?;
    let (config, cache_root) = resolved_build_cache_root_in(&water_home).await?;
    cleanup_stale_caches_if_idle(&cache_root, &project_root, &config).await
}

async fn spawn_build_cache_cleanup_process(project_root: &Path) -> eyre::Result<()> {
    let current_executable = std::env::current_exe()
        .wrap_err("Failed to resolve current water executable for build-cache cleanup")?;
    let project_root = project_root.to_path_buf();

    smol::unblock(move || -> eyre::Result<()> {
        std::process::Command::new(&current_executable)
            .arg("gc")
            .arg("build-cache")
            .arg("--path")
            .arg(&project_root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(eyre::Report::from)
            .wrap_err_with(|| {
                format!(
                    "Failed to spawn build-cache cleanup process for {}",
                    project_root.display()
                )
            })
    })
    .await
}

/// Remove the managed build cache for a project.
///
/// # Errors
/// Returns an error if the project root cannot be canonicalized or cache entries cannot be removed.
pub async fn remove_project_build_cache(project_root: &Path) -> eyre::Result<()> {
    let project_root = canonicalize_project_root(project_root)?;
    let cache_root = build_cache_root().await?;
    remove_project_build_cache_in(&project_root, &cache_root).await
}

async fn resolved_build_cache_root_in(water_home: &Path) -> eyre::Result<(WaterConfig, PathBuf)> {
    let config = ensure_global_config_in(water_home).await?;
    let cache_root = water_home.join(BUILD_CACHE_DIR_NAME);
    fs::create_dir_all(&cache_root)
        .await
        .wrap_err_with(|| format!("Failed to create build cache root {}", cache_root.display()))?;
    Ok((config, cache_root))
}

async fn ensure_global_config_in(water_home: &Path) -> eyre::Result<WaterConfig> {
    fs::create_dir_all(water_home)
        .await
        .wrap_err_with(|| format!("Failed to create Water home {}", water_home.display()))?;

    let config_path = water_home.join(CONFIG_FILE_NAME);
    if config_path.exists() {
        let contents = fs::read_to_string(&config_path)
            .await
            .wrap_err_with(|| format!("Failed to read Water config {}", config_path.display()))?;
        return toml::from_str(&contents)
            .wrap_err_with(|| format!("Failed to parse Water config {}", config_path.display()));
    }

    let config = WaterConfig::default();
    let contents =
        toml::to_string_pretty(&config).wrap_err("Failed to serialize default Water config")?;
    fs::write(&config_path, contents)
        .await
        .wrap_err_with(|| format!("Failed to write Water config {}", config_path.display()))?;
    Ok(config)
}

async fn ensure_project_build_cache_in(
    project_root: &Path,
    cache_root: &Path,
    _config: &WaterConfig,
) -> eyre::Result<PathBuf> {
    remove_legacy_local_water_dir(project_root).await?;

    let cache_dir = project_build_cache_dir_in(project_root, cache_root);
    if cache_dir.exists() {
        let should_clean = match read_metadata(&cache_dir).await {
            Ok(metadata) => {
                metadata.project_root != project_root.display().to_string()
                    || metadata.cli_commit != CLI_COMMIT
            }
            Err(_) => true,
        };

        if should_clean {
            info!(
                "Managed build cache changed shape, cleaning {}",
                cache_dir.display()
            );
            fs::remove_dir_all(&cache_dir).await?;
            prune_empty_build_cache_ancestors(
                cache_root,
                cache_dir
                    .parent()
                    .expect("managed build cache dir should always have a parent"),
            )
            .await?;
        }
    }

    fs::create_dir_all(&cache_dir)
        .await
        .wrap_err_with(|| format!("Failed to create build cache dir {}", cache_dir.display()))?;
    write_metadata(
        &cache_dir,
        &CacheMetadata {
            project_root: project_root.display().to_string(),
            cli_commit: CLI_COMMIT.to_string(),
            last_used_unix_seconds: now_unix_seconds()?,
        },
    )
    .await?;

    Ok(cache_dir)
}

async fn remove_project_build_cache_in(project_root: &Path, cache_root: &Path) -> eyre::Result<()> {
    let cache_dir = project_build_cache_dir_in(project_root, cache_root);
    if cache_dir.exists() {
        fs::remove_dir_all(&cache_dir).await?;
        prune_empty_build_cache_ancestors(
            cache_root,
            cache_dir
                .parent()
                .expect("managed build cache dir should always have a parent"),
        )
        .await?;
    }
    remove_legacy_local_water_dir(project_root).await
}

fn project_build_cache_dir_in(project_root: &Path, cache_root: &Path) -> PathBuf {
    project_cache_container_in(project_root, cache_root).join(MANAGED_BACKENDS_DIR_NAME)
}

fn project_cache_container_in(project_root: &Path, cache_root: &Path) -> PathBuf {
    let mut path = cache_root.to_path_buf();
    for component in project_root.components() {
        match component {
            Component::Prefix(prefix) => path.push(normalize_prefix_component(prefix)),
            Component::RootDir => {}
            Component::Normal(segment) => path.push(segment),
            Component::CurDir | Component::ParentDir => {
                panic!(
                    "Canonical project root {} must not contain relative path components",
                    project_root.display()
                );
            }
        }
    }
    path
}

fn normalize_prefix_component(prefix: PrefixComponent<'_>) -> String {
    match prefix.kind() {
        Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
            format!("drive-{}", char::from(letter).to_ascii_uppercase())
        }
        Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
            format!("unc-{}-{}", sanitize_os_str(server), sanitize_os_str(share))
        }
        Prefix::DeviceNS(device) => format!("device-{}", sanitize_os_str(device)),
        Prefix::Verbatim(component) => format!("verbatim-{}", sanitize_os_str(component)),
    }
}

fn sanitize_os_str(value: &OsStr) -> String {
    let sanitized = value
        .to_string_lossy()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        return String::from("empty");
    }
    sanitized
}

fn canonicalize_project_root(project_root: &Path) -> eyre::Result<PathBuf> {
    project_root.canonicalize().wrap_err_with(|| {
        format!(
            "Failed to canonicalize project root {}",
            project_root.display()
        )
    })
}

fn metadata_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(METADATA_FILE_NAME)
}

async fn read_metadata(cache_dir: &Path) -> eyre::Result<CacheMetadata> {
    let metadata_path = metadata_path(cache_dir);
    let contents = fs::read_to_string(&metadata_path)
        .await
        .wrap_err_with(|| format!("Failed to read cache metadata {}", metadata_path.display()))?;
    toml::from_str(&contents)
        .wrap_err_with(|| format!("Failed to parse cache metadata {}", metadata_path.display()))
}

async fn write_metadata(cache_dir: &Path, metadata: &CacheMetadata) -> eyre::Result<()> {
    let metadata_path = metadata_path(cache_dir);
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

async fn cleanup_stale_caches(
    cache_root: &Path,
    current_project_root: &Path,
    config: &WaterConfig,
) -> eyre::Result<BuildCacheGcSummary> {
    fs::create_dir_all(cache_root).await?;

    let current_cache_dir = project_build_cache_dir_in(current_project_root, cache_root);
    let max_unused_seconds = config
        .build_cache
        .cleanup_after_unused_days
        .saturating_mul(24 * 60 * 60);
    let now = now_unix_seconds()?;

    let mut scanned_entries = 0usize;
    let mut removed_entries = 0usize;

    for cache_dir in discover_managed_build_cache_dirs(cache_root).await? {
        if cache_dir == current_cache_dir {
            continue;
        }

        scanned_entries += 1;

        let should_remove = match read_metadata(&cache_dir).await {
            Ok(metadata) => {
                let project_root = PathBuf::from(&metadata.project_root);
                !project_root.exists()
                    || now.saturating_sub(metadata.last_used_unix_seconds) > max_unused_seconds
            }
            Err(error) => {
                warn!(
                    "Removing stale build cache with invalid metadata at {}: {error}",
                    cache_dir.display()
                );
                true
            }
        };

        if should_remove {
            if let Err(error) = fs::remove_dir_all(&cache_dir).await {
                warn!(
                    "Failed to remove stale build cache {}: {error}",
                    cache_dir.display()
                );
                continue;
            }
            prune_empty_build_cache_ancestors(
                cache_root,
                cache_dir
                    .parent()
                    .expect("managed build cache dir should always have a parent"),
            )
            .await?;
            removed_entries += 1;
        }
    }

    Ok(BuildCacheGcSummary {
        scanned_entries,
        removed_entries,
    })
}

async fn cleanup_stale_caches_if_idle(
    cache_root: &Path,
    current_project_root: &Path,
    config: &WaterConfig,
) -> eyre::Result<BuildCacheGcOutcome> {
    let lock_path = cache_root.join(CLEANUP_LOCK_FILE_NAME);
    let Some(lock_file) = try_acquire_cleanup_lock(&lock_path).await? else {
        return Ok(BuildCacheGcOutcome::SkippedAlreadyRunning);
    };

    let cleanup_result = cleanup_stale_caches(cache_root, current_project_root, config).await;
    drop(lock_file);

    if let Err(error) = fs::remove_file(&lock_path).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        warn!(
            lock_path = %lock_path.display(),
            "Failed to remove build-cache cleanup lock: {error}"
        );
    }

    cleanup_result.map(BuildCacheGcOutcome::Ran)
}

async fn try_acquire_cleanup_lock(lock_path: &Path) -> eyre::Result<Option<fs::File>> {
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
        .await
    {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(error) => Err(eyre::Report::from(error))
            .wrap_err_with(|| format!("Failed to create cleanup lock {}", lock_path.display())),
    }
}

async fn discover_managed_build_cache_dirs(cache_root: &Path) -> eyre::Result<Vec<PathBuf>> {
    let cache_root = cache_root.to_path_buf();
    smol::unblock(move || -> eyre::Result<Vec<PathBuf>> {
        let mut cache_dirs = Vec::new();
        for entry in WalkDir::new(&cache_root).follow_links(false) {
            let entry = entry.map_err(eyre::Report::from)?;
            if !entry.file_type().is_file() || entry.file_name() != OsStr::new(METADATA_FILE_NAME) {
                continue;
            }

            let cache_dir = entry.path().parent().ok_or_else(|| {
                eyre::eyre!("Cache metadata {} has no parent", entry.path().display())
            })?;
            if cache_dir.file_name() != Some(OsStr::new(MANAGED_BACKENDS_DIR_NAME)) {
                continue;
            }
            cache_dirs.push(cache_dir.to_path_buf());
        }
        Ok(cache_dirs)
    })
    .await
}

async fn prune_empty_build_cache_ancestors(
    cache_root: &Path,
    starting_dir: &Path,
) -> eyre::Result<()> {
    let mut current = starting_dir.to_path_buf();
    while current.starts_with(cache_root) && current != cache_root {
        match fs::remove_dir(&current).await {
            Ok(()) => {
                let Some(parent) = current.parent() else {
                    break;
                };
                current = parent.to_path_buf();
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(parent) = current.parent() else {
                    break;
                };
                current = parent.to_path_buf();
            }
            Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => break,
            Err(error) => return Err(error.into()),
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
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::{
        CLI_COMMIT, WaterConfig, ensure_global_config_in, ensure_project_build_cache_in,
        metadata_path, now_unix_seconds, project_build_cache_dir_in, remove_project_build_cache_in,
    };

    #[test]
    fn ensure_global_config_writes_default_build_cache_policy() {
        smol::block_on(async {
            let water_home = tempdir().expect("water home");

            let config = ensure_global_config_in(water_home.path())
                .await
                .expect("ensure config");

            assert_eq!(
                config.build_cache.cleanup_after_unused_days,
                super::DEFAULT_BUILD_CACHE_CLEANUP_AFTER_UNUSED_DAYS
            );
            let saved = smol::fs::read_to_string(water_home.path().join("config.toml"))
                .await
                .expect("read config");
            assert!(saved.contains("[build_cache]"));
            assert!(saved.contains("cleanup_after_unused_days = 30"));
        });
    }

    #[test]
    fn project_build_cache_dir_uses_absolute_project_path_components() {
        let cache_root = Path::new("/tmp/water-cache-root");
        let project_root = if cfg!(windows) {
            PathBuf::from(r"C:\Users\lexo\demo")
        } else {
            PathBuf::from("/Users/lexo/demo")
        };

        let cache_dir = project_build_cache_dir_in(&project_root, cache_root);

        let expected = if cfg!(windows) {
            cache_root.join("drive-C/Users/lexo/demo/managed_backends")
        } else {
            cache_root.join("Users/lexo/demo/managed_backends")
        };
        assert_eq!(cache_dir, expected);
    }

    #[test]
    fn ensure_project_build_cache_uses_global_build_cache_dir() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let water_home = tempdir().expect("water home");
            let config = ensure_global_config_in(water_home.path())
                .await
                .expect("ensure config");
            let cache_root = water_home.path().join("build_cache");

            let cache_dir = ensure_project_build_cache_in(project.path(), &cache_root, &config)
                .await
                .expect("ensure cache");

            assert!(cache_dir.starts_with(&cache_root));
            assert_ne!(cache_dir, project.path().join(".water"));
            assert!(cache_dir.ends_with("managed_backends"));
            assert!(cache_dir.exists());
        });
    }

    #[test]
    fn ensure_project_build_cache_removes_legacy_local_water_dir() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let water_home = tempdir().expect("water home");
            let config = ensure_global_config_in(water_home.path())
                .await
                .expect("ensure config");
            let cache_root = water_home.path().join("build_cache");
            let legacy_dir = project.path().join(".water");
            smol::fs::create_dir_all(&legacy_dir)
                .await
                .expect("create legacy dir");
            smol::fs::write(legacy_dir.join("stale"), b"stale")
                .await
                .expect("write legacy file");

            ensure_project_build_cache_in(project.path(), &cache_root, &config)
                .await
                .expect("ensure cache");

            assert!(!legacy_dir.exists());
        });
    }

    #[test]
    fn ensure_project_build_cache_cleans_cache_when_cli_commit_changes() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let water_home = tempdir().expect("water home");
            let config = ensure_global_config_in(water_home.path())
                .await
                .expect("ensure config");
            let cache_root = water_home.path().join("build_cache");
            let cache_dir = project_build_cache_dir_in(project.path(), &cache_root);
            smol::fs::create_dir_all(&cache_dir)
                .await
                .expect("create cache dir");
            smol::fs::write(cache_dir.join("stale"), b"stale")
                .await
                .expect("write stale file");
            let stale_metadata = super::CacheMetadata {
                project_root: project.path().display().to_string(),
                cli_commit: String::from("old-commit"),
                last_used_unix_seconds: 1,
            };
            let stale_contents =
                toml::to_string(&stale_metadata).expect("serialize stale metadata");
            smol::fs::write(metadata_path(&cache_dir), stale_contents)
                .await
                .expect("write stale metadata");

            ensure_project_build_cache_in(project.path(), &cache_root, &config)
                .await
                .expect("ensure cache");

            assert!(!cache_dir.join("stale").exists());
            let fresh_metadata = super::read_metadata(&cache_dir)
                .await
                .expect("read metadata");
            assert_eq!(fresh_metadata.cli_commit, CLI_COMMIT);
        });
    }

    #[test]
    fn cleanup_stale_caches_removes_stale_orphaned_caches() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let water_home = tempdir().expect("water home");
            let config = WaterConfig::default();
            let cache_root = water_home.path().join("build_cache");
            let stale_cache = cache_root.join("definitely/missing/project/managed_backends");
            smol::fs::create_dir_all(&stale_cache)
                .await
                .expect("create stale cache");
            let stale_metadata = super::CacheMetadata {
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

            let outcome = super::cleanup_stale_caches_if_idle(&cache_root, project.path(), &config)
                .await
                .expect("cleanup caches");

            assert_eq!(
                outcome,
                super::BuildCacheGcOutcome::Ran(super::BuildCacheGcSummary {
                    scanned_entries: 1,
                    removed_entries: 1,
                })
            );
            assert!(!stale_cache.exists());
        });
    }

    #[test]
    fn remove_project_build_cache_deletes_only_managed_backends_leaf() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let child_project = project.path().join("nested/child");
            smol::fs::create_dir_all(&child_project)
                .await
                .expect("create child project");
            let water_home = tempdir().expect("water home");
            let config = ensure_global_config_in(water_home.path())
                .await
                .expect("ensure config");
            let cache_root = water_home.path().join("build_cache");

            let parent_cache = ensure_project_build_cache_in(project.path(), &cache_root, &config)
                .await
                .expect("ensure parent cache");
            let child_cache = ensure_project_build_cache_in(&child_project, &cache_root, &config)
                .await
                .expect("ensure child cache");

            remove_project_build_cache_in(project.path(), &cache_root)
                .await
                .expect("remove parent cache");

            assert!(!parent_cache.exists());
            assert!(child_cache.exists());
        });
    }
}
