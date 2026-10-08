//! Management of the global Water home and per-project managed backend build cache.
//!
//! Projects store generated backends under
//! `~/.water/build_cache/<absolute-project-path>/managed_backends/` instead of
//! scattering `.water` directories into user projects. Compiled Cargo artifacts
//! live in the sibling `~/.water/build_cache/target/` — one directory shared by
//! every project, so a machine compiles each dependency revision once no matter
//! how many projects use it. The price of that sharing is serialization: Cargo
//! takes one build-directory lock per target, so concurrent `water` invocations
//! on different projects build one at a time — the second reports `Blocking
//! waiting for file lock on build directory`, which the piped progress render
//! surfaces rather than swallowing — instead of each compiling the same
//! dependency graph in parallel. Sharing wins overall for the workflows the
//! CLI targets, where the alternative is N cold framework compiles.

use std::{
    ffi::OsStr,
    io::Write as _,
    path::{Component, Path, PathBuf, Prefix, PrefixComponent},
    process::Stdio,
    time::{SystemTime, UNIX_EPOCH},
};

use eyre::WrapErr;
use fs4::{FileExt, TryLockError};
use serde::{Deserialize, Serialize};
use smol::fs;
use tracing::{info, warn};
use walkdir::WalkDir;

/// The CLI commit hash embedded at build time.
pub const CLI_COMMIT: &str = env!("WATERUI_CLI_COMMIT");

const BUILD_CACHE_DIR_NAME: &str = "build_cache";
const MANAGED_BACKENDS_DIR_NAME: &str = "managed_backends";
const SHARED_TARGET_DIR_NAME: &str = "target";
const CONFIG_FILE_NAME: &str = "config.toml";
const METADATA_FILE_NAME: &str = "metadata.toml";
const METADATA_LOCK_FILE_NAME: &str = ".metadata.lock";
const CLEANUP_LOCK_FILE_NAME: &str = ".cleanup.lock";
const SHARED_TARGET_LEASE_FILE_NAME: &str = ".build-lease";
const LEGACY_LOCAL_WATER_DIR_NAME: &str = ".water";
const DEFAULT_BUILD_CACHE_CLEANUP_AFTER_UNUSED_DAYS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
/// Global Water CLI configuration persisted to `~/.water/config.toml`.
pub struct WaterConfig {
    /// Managed build-cache policy.
    #[serde(default)]
    pub build_cache: BuildCacheConfig,
    /// The device `water run` last used per target, keyed by
    /// `"<backend>/<platform>"` (for example `"apple/ios"`). The value is the
    /// device's stable identifier — a simulator UDID, a physical-device
    /// identifier, an Android serial, or an AVD name.
    #[serde(default)]
    pub last_used_device: std::collections::BTreeMap<String, String>,
    /// Unix seconds of the last passive `water update` check, which runs at
    /// most once per 24 hours.
    #[serde(default)]
    pub last_update_check_unix_seconds: Option<u64>,
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

/// The current user's home directory could not be determined.
#[derive(Debug, thiserror::Error)]
#[error("Could not determine home directory")]
pub struct HomeDirError;

/// Return the Water home directory root at `~/.water`.
///
/// # Errors
/// Returns an error if the current user's home directory cannot be determined.
pub fn water_home_dir() -> Result<PathBuf, HomeDirError> {
    let home = dirs::home_dir().ok_or(HomeDirError)?;
    Ok(home.join(".water"))
}

/// Return the Water home directory root at `~/.water` for `host`.
///
/// # Errors
/// Returns an error if the host's home directory cannot be determined.
pub fn water_home_dir_in(host: &crate::toolchain::Host) -> Result<PathBuf, HomeDirError> {
    let home = host.home_dir().ok_or(HomeDirError)?;
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

/// Return the Cargo target directory every project's CLI-managed builds share,
/// creating it if needed.
///
/// Compiled units are fingerprint-keyed, so one `~/.water/build_cache/target`
/// serves every project on the machine: a second `water run` reuses the
/// dependency graph the first one compiled instead of cold-building it per
/// project. The directory sits beside the per-project `managed_backends`
/// containers rather than inside one — generated sources are wiped when the
/// CLI's scaffold templates change, while compiled artifacts do not go stale
/// for that reason.
///
/// The directory is a first-class cache entry: it carries the same
/// `metadata.toml` the per-project containers do, so the garbage collector
/// reports it in usage surveys and reclaims it once it has been unused for
/// the configured window — but only while holding the exclusive build
/// lease, which stays out of reach for as long as any build compiling into
/// the tree holds a shared one (see `shared_target_lease_path`). One target also
/// means one Cargo build-directory lock: builds of different projects
/// serialize, and a waiting build prints `Blocking waiting for file lock on
/// build directory` — visible through the piped progress render — for the
/// holder's duration.
///
/// # Errors
/// Returns an error if the Water home cannot be determined, the global config
/// cannot be loaded, or the cache directory cannot be created.
pub async fn shared_target_dir() -> eyre::Result<PathBuf> {
    let cache_root = build_cache_root().await?;
    ensure_shared_target_dir_in(&cache_root).await
}

/// The path of the shared Cargo target directory, without creating it or
/// touching its metadata — for messages that name it.
///
/// # Errors
/// Returns an error if the Water home or the global config cannot be resolved.
pub async fn shared_target_dir_path() -> eyre::Result<PathBuf> {
    let water_home = water_home_dir()?;
    let (_, cache_root) = resolved_build_cache_root_in(&water_home).await?;
    Ok(cache_root.join(SHARED_TARGET_DIR_NAME))
}

async fn ensure_shared_target_dir_in(cache_root: &Path) -> eyre::Result<PathBuf> {
    let target_dir = cache_root.join(SHARED_TARGET_DIR_NAME);
    loop {
        fs::create_dir_all(&target_dir).await.wrap_err_with(|| {
            format!(
                "Failed to create shared target dir {}",
                target_dir.display()
            )
        })?;
        // Stamp under a shared build lease: the marker inside the tree is
        // the lease's own record of use, so a remover — which needs the
        // exclusive lease — cannot drop the directory between the create
        // and the write. The lease is released when this returns; a build
        // that then compiles into the tree takes its own share, and the
        // lease loop rebuilds the root if it was removed in between.
        let stamped = {
            let dir = target_dir.clone();
            smol::unblock(move || -> eyre::Result<Option<std::fs::File>> {
                let lease_path = shared_target_lease_path(&dir);
                let file = match open_shared_target_lease_file(&dir) {
                    Ok(file) => file,
                    // The tree was removed between the create and the open —
                    // rebuild and lease it again.
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Ok(None);
                    }
                    Err(error) => {
                        return Err(error).wrap_err_with(|| {
                            format!(
                                "Failed to open the shared target lease in {}",
                                dir.display()
                            )
                        });
                    }
                };
                FileExt::lock_shared(&file).wrap_err_with(|| {
                    format!("Failed to take a shared lease on {}", dir.display())
                })?;
                if locked_file_is_live(&file, &lease_path)? {
                    Ok(Some(file))
                } else {
                    Ok(None)
                }
            })
            .await?
        };
        let Some(_lease) = stamped else {
            continue;
        };
        // `project_root` is the entry itself: it exists exactly as long as
        // the cache does, so only the unused-days policy can collect it.
        write_metadata(
            &target_dir,
            &CacheMetadata {
                project_root: target_dir.display().to_string(),
                cli_commit: CLI_COMMIT.to_string(),
                last_used_unix_seconds: now_unix_seconds()?,
            },
        )
        .await?;
        return Ok(target_dir);
    }
}

/// The lock file serializing `metadata.toml` writes inside a cache dir.
///
/// Independent `water` processes may stamp the same cache entry
/// concurrently — a build resolves the shared target while another package
/// runs — and two overlapping writers can fail the persist step on
/// platforms that replace files less permissively (Windows cannot replace
/// a path another process has open for writing). Locking the sibling file
/// for the write's duration gives every writer a completed `metadata.toml`
/// regardless of overlap; like `.build-lease` it is never deleted.
fn metadata_lock_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(METADATA_LOCK_FILE_NAME)
}

/// Remove the shared Cargo target directory every project's builds write
/// into, returning the disk space it held — `None` when no shared target
/// exists.
///
/// The garbage collector only reclaims the directory once it has been unused
/// for the configured window; this is the explicit drop, for when a user
/// wants the space back now. Unlike `cargo clean`, it refuses while a Cargo
/// build is in flight — it must first take the exclusive lease, which any
/// build compiling into the tree keeps out of reach through its shared
/// lease — since deleting a target mid-build leaves the survivor's own
/// project with a half-written graph.
///
/// # Errors
/// Returns an error if the cache root cannot be resolved, a Cargo build is
/// using the directory, or the directory cannot be removed.
pub async fn remove_shared_target_dir() -> eyre::Result<Option<u64>> {
    let water_home = water_home_dir()?;
    let (_, cache_root) = resolved_build_cache_root_in(&water_home).await?;
    remove_shared_target_dir_in(&cache_root).await
}

async fn remove_shared_target_dir_in(cache_root: &Path) -> eyre::Result<Option<u64>> {
    let target_dir = cache_root.join(SHARED_TARGET_DIR_NAME);
    if !target_dir.exists() {
        return Ok(None);
    }
    // Held until the tree is gone: the exclusive lease is what proves no
    // build is compiling into it.
    let _lease = require_exclusive_shared_target_lease(&target_dir).await?;
    let bytes = directory_disk_usage(target_dir.clone()).await?;
    fs::remove_dir_all(&target_dir).await.wrap_err_with(|| {
        format!(
            "Failed to remove shared target dir {}",
            target_dir.display()
        )
    })?;
    Ok(Some(bytes))
}

/// Remove one project's own units from the shared Cargo target directory,
/// returning the paths removed.
///
/// `packages` are the project's generated crate names — tagged with the
/// project root's hash (see `generated_crate_name`), so nothing another
/// project compiled carries them. Every profile directory under the shared
/// root (`<variant>/<triple>/<profile>`, at most three levels deep and marked
/// by Cargo's `.cargo-lock`) is swept: the uplifted outputs in the profile
/// directory itself and the entries in `deps/`, `.fingerprint/`, `build/` and
/// `incremental/` that Cargo names after those packages. Dependency artifacts
/// — the framework, the shared runtime, everything a package by another name
/// produced — stay for the other projects that resolve them identically;
/// `remove_shared_target_dir` is the only operation that drops those.
///
/// This is the filesystem effect of `cargo clean --package` for each name, done
/// without Cargo because a project's generated manifests are removed by
/// the same clean and Cargo needs them — and their resolved dependency graph —
/// to compute the units.
///
/// # Errors
/// Returns an error if the cache root cannot be resolved, a Cargo build is
/// using the directory, or an entry cannot be removed.
pub async fn remove_project_units_from_shared_target(
    packages: &[String],
) -> eyre::Result<Vec<PathBuf>> {
    remove_project_units_in(&shared_target_dir_path().await?, packages).await
}

async fn remove_project_units_in(
    target_dir: &Path,
    packages: &[String],
) -> eyre::Result<Vec<PathBuf>> {
    if !target_dir.exists() || packages.is_empty() {
        return Ok(Vec::new());
    }
    let _lease = require_exclusive_shared_target_lease(target_dir).await?;
    let target_dir = target_dir.to_path_buf();
    let packages = packages.to_vec();
    smol::unblock(move || -> eyre::Result<Vec<PathBuf>> {
        let mut removed = Vec::new();
        let mut pending = vec![(target_dir, 0usize)];
        while let Some((dir, depth)) = pending.pop() {
            if dir.join(".cargo-lock").is_file() {
                removed.extend(remove_package_units_in_profile(&dir, &packages)?);
                continue;
            }
            if depth == 3 {
                continue;
            }
            for entry in std::fs::read_dir(&dir)
                .wrap_err_with(|| format!("Failed to read {}", dir.display()))?
            {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    pending.push((entry.path(), depth + 1));
                }
            }
        }
        removed.sort();
        Ok(removed)
    })
    .await
}

/// Cargo's per-profile subdirectories whose direct children are named after
/// the unit's package or target.
const PROFILE_UNIT_DIRS: [&str; 4] = ["deps", ".fingerprint", "build", "incremental"];

fn remove_package_units_in_profile(
    profile_dir: &Path,
    packages: &[String],
) -> eyre::Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    let mut dirs = vec![profile_dir.to_path_buf()];
    dirs.extend(
        PROFILE_UNIT_DIRS
            .iter()
            .map(|name| profile_dir.join(name))
            .filter(|dir| dir.is_dir()),
    );
    for dir in dirs {
        for entry in
            std::fs::read_dir(&dir).wrap_err_with(|| format!("Failed to read {}", dir.display()))?
        {
            let entry = entry?;
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            if !packages
                .iter()
                .any(|package| unit_entry_belongs_to(name, package))
            {
                continue;
            }
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            }
            .wrap_err_with(|| format!("Failed to remove {}", path.display()))?;
            removed.push(path);
        }
    }
    Ok(removed)
}

/// Whether a Cargo target-directory entry belongs to `package`.
///
/// Cargo spells a package `foo-bar` as `foo-bar-<hash>` in `.fingerprint/`
/// and `build/`, and its targets as `foo_bar`, `foo_bar-<hash>` or
/// `libfoo_bar-<hash>.<ext>` elsewhere; an uplifted output may carry an
/// extension straight after the name (`foo_bar.exe`, `foo_bar.pdb`), and a
/// further target of the same package extends the name with `_`
/// (`foo_bar_cef_helper`). Any other continuation is a different name.
fn unit_entry_belongs_to(entry_name: &str, package: &str) -> bool {
    let target_name = package.replace('-', "_");
    let stem = entry_name.strip_prefix("lib").unwrap_or(entry_name);
    [entry_name, stem].into_iter().any(|candidate| {
        [package, target_name.as_str()].into_iter().any(|name| {
            candidate
                .strip_prefix(name)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '.', '_']))
        })
    })
}

/// The lease file every build compiling into the shared Cargo target locks.
///
/// One file at the target root covers the whole tree — variant and
/// toolchain subdirectories alike — because a removal drops the root. Like
/// `.cleanup.lock` it is never deleted deliberately: it is the thing being
/// locked, not a signal, and a new lease file is simply created if the
/// directory is rebuilt after a drop.
fn shared_target_lease_path(target_dir: &Path) -> PathBuf {
    target_dir.join(SHARED_TARGET_LEASE_FILE_NAME)
}

/// Whether `file` is the inode `path` names right now.
///
/// A lock is granted on the inode the open file addresses, not on the path:
/// a remover can delete the tree — and the lease file with it — while a
/// lock waits, and the granted lock then guards nothing. Only when the
/// locked file and the path name the same inode does the lease cover the
/// live tree.
fn locked_file_is_live(file: &std::fs::File, path: &Path) -> eyre::Result<bool> {
    let locked = same_file::Handle::from_file(file.try_clone()?)
        .wrap_err_with(|| format!("Failed to stat the lease file {}", path.display()))?;
    match same_file::Handle::from_path(path) {
        Ok(named) => Ok(locked == named),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => {
            Err(error).wrap_err_with(|| format!("Failed to stat the lease file {}", path.display()))
        }
    }
}

fn open_shared_target_lease_file(target_dir: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(shared_target_lease_path(target_dir))
}

/// Take a shared lease on the shared Cargo target rooted at `target_dir`,
/// waiting until the lock is granted.
///
/// `target_dir` is the directory [`shared_target_dir`] returns — never a
/// variant subdirectory — so the lease covers the tree a removal would drop.
/// Removers — the build-cache garbage collector and the explicit drop paths
/// — must take the exclusive lease before deleting, and they can only take
/// it when no build holds a share, so a live compile never loses the
/// directory it writes into. Builds hold the returned file for as long as
/// their Cargo invocation runs — dropping or crashing releases the lock —
/// and that is what keeps the target out of a collector's reach, where a
/// one-time probe would lose every build that starts after the check.
///
/// A remover can delete the tree while the lock waits; when the granted
/// lock turns out to sit on a deleted inode — or the lease file was already
/// gone at open — the target is re-ensured so the root and a fresh marker
/// exist, and the lease is retaken on the live inode.
///
/// # Errors
/// Returns an error if the lease file cannot be opened, locked or checked,
/// or the target directory cannot be rebuilt after a removal.
pub async fn lease_shared_target_dir(target_dir: &Path) -> eyre::Result<std::fs::File> {
    loop {
        let dir = target_dir.to_path_buf();
        let leased = smol::unblock(move || -> eyre::Result<Option<std::fs::File>> {
            let lease_path = shared_target_lease_path(&dir);
            let file = match open_shared_target_lease_file(&dir) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => {
                    return Err(error).wrap_err_with(|| {
                        format!(
                            "Failed to open the shared target lease in {}",
                            dir.display()
                        )
                    });
                }
            };
            FileExt::lock_shared(&file)
                .wrap_err_with(|| format!("Failed to take a shared lease on {}", dir.display()))?;
            if locked_file_is_live(&file, &lease_path)? {
                return Ok(Some(file));
            }
            Ok(None)
        })
        .await?;
        if let Some(lease) = leased {
            return Ok(lease);
        }
        // The tree was removed around the lock: rebuild the root and its
        // fresh marker, then lease whatever the path names afterwards.
        let cache_root = target_dir.parent().ok_or_else(|| {
            eyre::eyre!(
                "the shared Cargo target {} has no parent directory",
                target_dir.display()
            )
        })?;
        ensure_shared_target_dir_in(cache_root).await?;
    }
}

/// Try to take the exclusive lease on the shared Cargo target rooted at
/// `target_dir`, returning `None` while any build holds a shared lease.
///
/// The granted lock must sit on the inode the lease path still names: when
/// another remover deleted and the path was re-created in between, the lock
/// guards a dead inode and is dropped for a retry on the live file, so two
/// removers never both believe they own the tree.
async fn try_exclusive_shared_target_lease(
    target_dir: &Path,
) -> eyre::Result<Option<std::fs::File>> {
    let target_dir = target_dir.to_path_buf();
    smol::unblock(move || -> eyre::Result<Option<std::fs::File>> {
        let lease_path = shared_target_lease_path(&target_dir);
        loop {
            let file = open_shared_target_lease_file(&target_dir).wrap_err_with(|| {
                format!(
                    "Failed to open the shared target lease in {}",
                    target_dir.display()
                )
            })?;
            match FileExt::try_lock(&file) {
                Ok(()) => {
                    if locked_file_is_live(&file, &lease_path)? {
                        return Ok(Some(file));
                    }
                    // The lock sits on a deleted inode and guards nothing —
                    // drop it and take the file the path names now.
                }
                Err(TryLockError::WouldBlock) => return Ok(None),
                Err(TryLockError::Error(error)) => {
                    return Err(eyre::Report::from(error)).wrap_err_with(|| {
                        format!(
                            "Failed to lock the shared target lease in {}",
                            target_dir.display()
                        )
                    });
                }
            }
        }
    })
    .await
}

/// The exclusive lease a remover must hold before dropping any part of the
/// shared Cargo target.
///
/// The error names the running build the way the explicit drops report it;
/// the returned file keeps the lease — and every build out of the tree —
/// until it is dropped.
async fn require_exclusive_shared_target_lease(target_dir: &Path) -> eyre::Result<std::fs::File> {
    try_exclusive_shared_target_lease(target_dir)
        .await?
        .ok_or_else(|| {
            eyre::eyre!(
                "the shared Cargo target {} is in use by a running build — drop it once the build finishes",
                target_dir.display()
            )
        })
}

/// Return the managed build-cache directory for a project.
///
/// # Errors
/// Returns an error if the project root cannot be canonicalized or the global cache root cannot be resolved.
pub async fn project_build_cache_dir(project_root: &Path) -> eyre::Result<PathBuf> {
    let water_home = water_home_dir()?;
    project_build_cache_dir_from_home(project_root, &water_home).await
}

/// Return the managed build-cache directory for a project under `host`'s Water home.
///
/// # Errors
/// Returns an error if the host has no home directory, the project root cannot
/// be canonicalized, or the global cache root cannot be resolved.
pub async fn project_build_cache_dir_on(
    host: &crate::toolchain::Host,
    project_root: &Path,
) -> eyre::Result<PathBuf> {
    let water_home = water_home_dir_in(host)?;
    project_build_cache_dir_from_home(project_root, &water_home).await
}

async fn project_build_cache_dir_from_home(
    project_root: &Path,
    water_home: &Path,
) -> eyre::Result<PathBuf> {
    let project_root = canonicalize_project_root(project_root)?;
    let (_, cache_root) = resolved_build_cache_root_in(water_home).await?;
    Ok(project_build_cache_dir_in(&project_root, &cache_root))
}

/// Return the whole managed cache container for a project directory, which need
/// not exist.
///
/// A generated backend workspace lives in the cache rather than in the project,
/// so a project that has been thrown away leaves its cache behind — and that
/// cache is what the next build reads. Canonicalizing the project root is not
/// available in that case, so the nearest existing ancestor is canonicalized
/// and the rest of the path appended as written.
///
/// # Errors
/// Returns an error if no ancestor of `project_root` can be canonicalized or the
/// global cache root cannot be resolved.
pub async fn build_cache_container_for(project_root: &Path) -> eyre::Result<PathBuf> {
    let cache_root = build_cache_root().await?;
    build_cache_container_for_in(project_root, &cache_root)
}

/// Resolve the managed cache container path for a project under `host`'s Water home.
///
/// This resolves the path only; it creates nothing.
///
/// # Errors
/// Returns an error if the host has no home directory or no ancestor of
/// `project_root` can be canonicalized.
pub fn build_cache_container_for_on(
    host: &crate::toolchain::Host,
    project_root: &Path,
) -> eyre::Result<PathBuf> {
    let water_home = water_home_dir_in(host)?;
    let cache_root = water_home.join(BUILD_CACHE_DIR_NAME);
    build_cache_container_for_in(project_root, &cache_root)
}

fn build_cache_container_for_in(project_root: &Path, cache_root: &Path) -> eyre::Result<PathBuf> {
    let mut trailing = Vec::new();
    let mut existing = project_root.to_path_buf();
    let resolved = loop {
        if let Ok(resolved) = existing.canonicalize() {
            break resolved;
        }
        let name = existing.file_name().map(std::ffi::OsString::from);
        let parent = existing.parent().map(Path::to_path_buf);
        let (Some(name), Some(parent)) = (name, parent) else {
            return Err(eyre::eyre!(
                "Failed to resolve any existing ancestor of {}",
                project_root.display()
            ));
        };
        trailing.push(name);
        existing = parent;
    };
    let mut project_root = resolved;
    for name in trailing.iter().rev() {
        project_root.push(name);
    }
    Ok(project_cache_container_in(&project_root, cache_root))
}

/// Ensure the managed build-cache directory exists for a project and return it.
///
/// # Errors
/// Returns an error if the project root cannot be canonicalized, config loading fails, or cache directories cannot be created.
pub async fn ensure_project_build_cache(
    host: &crate::toolchain::Host,
    project_root: &Path,
) -> eyre::Result<PathBuf> {
    let project_root = canonicalize_project_root(project_root)?;
    let water_home = water_home_dir_in(host)?;
    let (config, cache_root) = resolved_build_cache_root_in(&water_home).await?;
    if let Err(error) = spawn_build_cache_cleanup_process(host, &project_root).await {
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

async fn spawn_build_cache_cleanup_process(
    host: &crate::toolchain::Host,
    project_root: &Path,
) -> eyre::Result<()> {
    let current_executable = std::env::current_exe()
        .wrap_err("Failed to resolve current water executable for build-cache cleanup")?;
    let project_root = project_root.to_path_buf();
    let mut command = host.std_command(&current_executable);
    command
        .arg("gc")
        .arg("build-cache")
        .arg("--path")
        .arg(&project_root)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    smol::unblock(move || -> eyre::Result<()> {
        command
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

pub(crate) async fn ensure_global_config_in(water_home: &Path) -> eyre::Result<WaterConfig> {
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
    write_global_config_in(water_home, &config).await?;
    Ok(config)
}

/// Persist `config` to `~/.water/config.toml`.
///
/// # Errors
/// Returns an error if the config cannot be serialized or written.
pub async fn write_global_config(config: &WaterConfig) -> eyre::Result<()> {
    let water_home = water_home_dir()?;
    write_global_config_in(&water_home, config).await
}

pub(crate) async fn write_global_config_in(
    water_home: &Path,
    config: &WaterConfig,
) -> eyre::Result<()> {
    let config_path = water_home.join(CONFIG_FILE_NAME);
    let contents = toml::to_string_pretty(config).wrap_err("Failed to serialize Water config")?;
    fs::write(&config_path, contents)
        .await
        .wrap_err_with(|| format!("Failed to write Water config {}", config_path.display()))
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
    let lock_path = metadata_lock_path(cache_dir);
    let contents = toml::to_string(metadata).wrap_err("Failed to serialize cache metadata")?;
    let cache_dir = cache_dir.to_path_buf();
    smol::unblock(move || -> eyre::Result<()> {
        // Hold the cache dir's metadata lock across stage+rename so
        // concurrent processes stamp the marker one at a time.
        let lock = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .wrap_err_with(|| {
                format!("Failed to open cache metadata lock {}", lock_path.display())
            })?;
        FileExt::lock(&lock)
            .wrap_err_with(|| format!("Failed to lock cache metadata {}", lock_path.display()))?;
        // Stage the marker beside its name and persist it in one step: a
        // reader never observes a truncated document, and a crash only
        // leaves the unnamed staging file `NamedTempFile` cleans up on drop.
        let mut staging = tempfile::NamedTempFile::new_in(&cache_dir).wrap_err_with(|| {
            format!("Failed to stage cache metadata {}", metadata_path.display())
        })?;
        staging.write_all(contents.as_bytes()).wrap_err_with(|| {
            format!("Failed to write cache metadata {}", metadata_path.display())
        })?;
        staging.persist(&metadata_path).wrap_err_with(|| {
            format!("Failed to write cache metadata {}", metadata_path.display())
        })?;
        Ok(())
    })
    .await
}

async fn remove_legacy_local_water_dir(project_root: &Path) -> eyre::Result<()> {
    let legacy_dir = project_root.join(LEGACY_LOCAL_WATER_DIR_NAME);
    if legacy_dir.exists() {
        info!(
            "Removing legacy local build cache at {}",
            legacy_dir.display()
        );
        fs::remove_dir_all(&legacy_dir).await?;
    }
    Ok(())
}

/// On-disk usage of one managed build-cache entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildCacheEntryUsage {
    /// Project the cache entry belongs to.
    pub project_root: PathBuf,
    /// Managed cache directory holding the entry.
    pub cache_dir: PathBuf,
    /// Disk space the entry occupies, in bytes.
    pub bytes: u64,
    /// Whether the entry is currently eligible for removal.
    pub stale: bool,
    /// Whether the entry belongs to the project the survey was run from.
    pub active: bool,
}

/// What the managed build cache is currently spending disk on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildCacheUsageReport {
    /// Entries, largest first.
    pub entries: Vec<BuildCacheEntryUsage>,
    /// Total disk space across every entry, in bytes.
    pub total_bytes: u64,
    /// Disk space held by entries eligible for removal, in bytes.
    pub reclaimable_bytes: u64,
}

/// Survey the managed build cache without removing anything.
///
/// Sizes are measured in allocated blocks and each inode is counted once, so
/// copy-on-write clones and hard links are reported at what they actually cost
/// rather than at the sum of their apparent lengths.
///
/// # Errors
/// Returns an error if the cache root cannot be read or the Water config cannot be loaded.
pub async fn survey_build_cache_usage(
    current_project_root: &Path,
) -> eyre::Result<BuildCacheUsageReport> {
    let water_home = water_home_dir()?;
    let (config, cache_root) = resolved_build_cache_root_in(&water_home).await?;
    let current_cache_dir = project_build_cache_dir_in(current_project_root, &cache_root);
    let max_unused_seconds = config
        .build_cache
        .cleanup_after_unused_days
        .saturating_mul(24 * 60 * 60);
    let now = now_unix_seconds()?;

    let mut entries = Vec::new();
    for cache_dir in discover_managed_build_cache_dirs(&cache_root).await? {
        let metadata = read_metadata(&cache_dir).await.ok();
        let project_root = metadata.as_ref().map_or_else(
            || cache_dir.clone(),
            |metadata| PathBuf::from(&metadata.project_root),
        );
        let active = cache_dir == current_cache_dir;
        // An unreadable marker is not proof of staleness: the entry reports
        // its size but is never counted as reclaimable.
        let stale = !active
            && metadata.as_ref().is_some_and(|metadata| {
                !PathBuf::from(&metadata.project_root).exists()
                    || now.saturating_sub(metadata.last_used_unix_seconds) > max_unused_seconds
            });
        let bytes = directory_disk_usage(cache_dir.clone()).await?;
        entries.push(BuildCacheEntryUsage {
            project_root,
            cache_dir,
            bytes,
            stale,
            active,
        });
    }

    entries.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.cache_dir.cmp(&right.cache_dir))
    });
    let total_bytes = entries.iter().map(|entry| entry.bytes).sum();
    let reclaimable_bytes = entries
        .iter()
        .filter(|entry| entry.stale)
        .map(|entry| entry.bytes)
        .sum();

    Ok(BuildCacheUsageReport {
        entries,
        total_bytes,
        reclaimable_bytes,
    })
}

/// Measure how much disk a directory tree actually occupies.
async fn directory_disk_usage(root: PathBuf) -> eyre::Result<u64> {
    smol::unblock(move || {
        let mut seen_inodes = std::collections::HashSet::new();
        let mut total = 0u64;
        for entry in WalkDir::new(&root).follow_links(false) {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            total = total.saturating_add(file_disk_usage(&metadata, &mut seen_inodes));
        }
        Ok(total)
    })
    .await
}

#[cfg(unix)]
fn file_disk_usage(
    metadata: &std::fs::Metadata,
    seen_inodes: &mut std::collections::HashSet<(u64, u64)>,
) -> u64 {
    use std::os::unix::fs::MetadataExt as _;

    if !seen_inodes.insert((metadata.dev(), metadata.ino())) {
        return 0;
    }
    // `blocks` is always in 512-byte units, independent of the filesystem's own
    // block size, and already excludes extents shared with a clone source.
    metadata.blocks().saturating_mul(512)
}

#[cfg(not(unix))]
fn file_disk_usage(
    metadata: &std::fs::Metadata,
    _seen_inodes: &mut std::collections::HashSet<(u64, u64)>,
) -> u64 {
    metadata.len()
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

    let shared_target_dir = cache_root.join(SHARED_TARGET_DIR_NAME);
    let mut scanned_entries = 0usize;
    let mut removed_entries = 0usize;

    for cache_dir in discover_managed_build_cache_dirs(cache_root).await? {
        if cache_dir == current_cache_dir {
            continue;
        }

        scanned_entries += 1;

        // The shared Cargo target is not a project entry: no project-root
        // liveness rule applies to it, only the unused-days policy — and even
        // a proven-stale marker cannot overrule a running build, so removal
        // needs the exclusive lease every build holds a share of.
        let is_shared_target = cache_dir == shared_target_dir;
        let should_remove = match read_metadata(&cache_dir).await {
            Ok(metadata) if is_shared_target => {
                now.saturating_sub(metadata.last_used_unix_seconds) > max_unused_seconds
            }
            Ok(metadata) => {
                let project_root = PathBuf::from(&metadata.project_root);
                !project_root.exists()
                    || now.saturating_sub(metadata.last_used_unix_seconds) > max_unused_seconds
            }
            // A marker that cannot be read proves nothing: every project open
            // rewrites it, so an unreadable file is as likely a build in
            // flight as garbage. Removal requires proof of staleness.
            Err(error) => {
                warn!(
                    "Keeping build cache entry with unreadable metadata at {}: {error}",
                    cache_dir.display()
                );
                false
            }
        };

        if !should_remove {
            continue;
        }

        // Held through the removal below: releasing the file releases the
        // exclusive lease only once the directory is gone.
        let mut removal_lease = None;
        if is_shared_target {
            match try_exclusive_shared_target_lease(&cache_dir).await {
                Ok(Some(lease)) => removal_lease = Some(lease),
                Ok(None) => {
                    warn!(
                        "Keeping shared Cargo target {}: a running build holds its lease",
                        cache_dir.display()
                    );
                    continue;
                }
                Err(error) => {
                    warn!(
                        "Keeping shared Cargo target {}: {error}",
                        cache_dir.display()
                    );
                    continue;
                }
            }
            // A build that opened the target after the marker was read above
            // has already refreshed it, so under the exclusive lease only a
            // still-stale marker may be removed.
            match read_metadata(&cache_dir).await {
                Ok(metadata)
                    if now.saturating_sub(metadata.last_used_unix_seconds) > max_unused_seconds => {
                }
                Ok(_) => continue,
                Err(error) => {
                    warn!(
                        "Keeping shared Cargo target with unreadable metadata at {}: {error}",
                        cache_dir.display()
                    );
                    continue;
                }
            }
        }

        if let Err(error) = fs::remove_dir_all(&cache_dir).await {
            warn!(
                "Failed to remove stale build cache {}: {error}",
                cache_dir.display()
            );
            continue;
        }
        drop(removal_lease);
        prune_empty_build_cache_ancestors(
            cache_root,
            cache_dir
                .parent()
                .expect("managed build cache dir should always have a parent"),
        )
        .await?;
        removed_entries += 1;
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
    // Closing the descriptor releases the lock. The file stays behind on
    // purpose: it is what the next sweep locks, and leaving it means no exit
    // path — including one that never runs — can stop cleanup happening again.
    drop(lock_file);

    cleanup_result.map(BuildCacheGcOutcome::Ran)
}

/// Takes the cleanup lock, or reports that another sweep already holds it.
///
/// The lock has to be released even when the process holding it dies, and
/// creating the file exclusively is not that: a sweep that is killed leaves the
/// file behind, and since then every run takes the "already running" path
/// against a process that no longer exists. Cleanup is spawned detached with its
/// output discarded, so nothing says so — one interrupted sweep in May left the
/// cache unswept until August, by which point it had grown to 149 GB, most of it
/// belonging to workspaces deleted months earlier.
///
/// `flock` is released by the kernel when the descriptor closes, which a crash
/// does too. The file itself stays: it is the thing being locked, not the
/// signal, so nothing has to remove it and nothing is stranded if a process
/// dies before it can.
async fn try_acquire_cleanup_lock(lock_path: &Path) -> eyre::Result<Option<std::fs::File>> {
    let lock_path = lock_path.to_path_buf();
    smol::unblock(move || {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .wrap_err_with(|| format!("Failed to open cleanup lock {}", lock_path.display()))?;
        match FileExt::try_lock(&file) {
            Ok(()) => Ok(Some(file)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(eyre::Report::from(error))
                .wrap_err_with(|| format!("Failed to lock {}", lock_path.display())),
        }
    })
    .await
}

/// Every managed cache entry under `cache_root`.
///
/// Entries are per-project `managed_backends` containers plus the shared
/// Cargo `target/` every project's builds write into. Neither interior is
/// walked: the shared target's marker is read directly — descending into a
/// cargo target costs as much as the GC itself — and a `managed_backends`
/// marker is checked the moment its directory surfaces, so the walk only
/// ever sees the container path hierarchy.
async fn discover_managed_build_cache_dirs(cache_root: &Path) -> eyre::Result<Vec<PathBuf>> {
    let cache_root = cache_root.to_path_buf();
    smol::unblock(move || -> eyre::Result<Vec<PathBuf>> {
        let shared_target_dir = cache_root.join(SHARED_TARGET_DIR_NAME);
        let mut cache_dirs = Vec::new();
        if shared_target_dir.join(METADATA_FILE_NAME).is_file() {
            cache_dirs.push(shared_target_dir.clone());
        }
        for entry in WalkDir::new(&cache_root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                // A project path can itself end in `target`, so only the
                // shared target at the root is pruned.
                if entry.depth() == 1 && entry.path() == shared_target_dir {
                    return false;
                }
                if entry.file_type().is_dir()
                    && entry.file_name() == OsStr::new(MANAGED_BACKENDS_DIR_NAME)
                {
                    if entry.path().join(METADATA_FILE_NAME).is_file() {
                        cache_dirs.push(entry.path().to_path_buf());
                    }
                    return false;
                }
                true
            })
        {
            entry.map_err(eyre::Report::from)?;
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
        write_global_config_in,
    };

    #[test]
    fn global_config_round_trips_last_used_devices() {
        smol::block_on(async {
            let water_home = tempdir().expect("water home");

            let mut config = WaterConfig::default();
            config.last_used_device.insert(
                "apple/ios".to_owned(),
                "00008140-00011C210CF3001C".to_owned(),
            );
            config
                .last_used_device
                .insert("android/android".to_owned(), "emulator-5554".to_owned());
            write_global_config_in(water_home.path(), &config)
                .await
                .expect("write config");

            let loaded = ensure_global_config_in(water_home.path())
                .await
                .expect("read config back");
            assert_eq!(
                loaded.last_used_device.get("apple/ios").map(String::as_str),
                Some("00008140-00011C210CF3001C")
            );
            assert_eq!(
                loaded
                    .last_used_device
                    .get("android/android")
                    .map(String::as_str),
                Some("emulator-5554")
            );
        });
    }

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

    /// A sweep that dies leaves the lock file behind, because only the happy
    /// path could ever remove it. Cleanup must still run afterwards: when it did
    /// not, one interrupted sweep stopped every later one for three months, and
    /// silently, since cleanup is spawned with its output discarded.
    #[test]
    fn cleanup_runs_again_after_a_sweep_dies_holding_the_lock() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let water_home = tempdir().expect("water home");
            let config = ensure_global_config_in(water_home.path())
                .await
                .expect("ensure config");
            let cache_root = water_home.path().join("build_cache");
            smol::fs::create_dir_all(&cache_root)
                .await
                .expect("create cache root");

            // What a killed sweep leaves on disk: the lock file, with no live
            // process behind it.
            let lock_path = cache_root.join(super::CLEANUP_LOCK_FILE_NAME);
            smol::fs::write(&lock_path, [])
                .await
                .expect("leave a lock file behind");

            let stale_project = tempdir().expect("stale project");
            let stale_cache =
                ensure_project_build_cache_in(stale_project.path(), &cache_root, &config)
                    .await
                    .expect("ensure stale cache");
            drop(stale_project);

            let outcome = super::cleanup_stale_caches_if_idle(&cache_root, project.path(), &config)
                .await
                .expect("cleanup");

            assert!(
                matches!(outcome, super::BuildCacheGcOutcome::Ran(_)),
                "an abandoned lock file must not be read as a running sweep, got {outcome:?}"
            );
            assert!(!stale_cache.exists());
        });
    }

    /// The other half: while a sweep really is holding the lock, a second one
    /// stands down instead of walking the same tree.
    #[test]
    fn a_second_sweep_stands_down_while_the_first_holds_the_lock() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let water_home = tempdir().expect("water home");
            let config = ensure_global_config_in(water_home.path())
                .await
                .expect("ensure config");
            let cache_root = water_home.path().join("build_cache");
            smol::fs::create_dir_all(&cache_root)
                .await
                .expect("create cache root");

            let lock_path = cache_root.join(super::CLEANUP_LOCK_FILE_NAME);
            let held = super::try_acquire_cleanup_lock(&lock_path)
                .await
                .expect("acquire lock")
                .expect("lock is free");

            let outcome = super::cleanup_stale_caches_if_idle(&cache_root, project.path(), &config)
                .await
                .expect("cleanup");
            assert_eq!(outcome, super::BuildCacheGcOutcome::SkippedAlreadyRunning);

            drop(held);
        });
    }

    /// Every `water` process stamps the shared target's marker, so callers
    /// on different projects routinely overlap the write. The stamp is
    /// serialized on a lock file: overlapping ensures all succeed and the
    /// surviving marker parses — no writer may be left with a failed
    /// rename or a torn document.
    #[test]
    fn concurrent_shared_target_stamps_serialize() {
        smol::block_on(async {
            let cache_root = tempdir().expect("cache root");
            let tasks: Vec<_> = (0..8)
                .map(|_| {
                    let root = cache_root.path().to_path_buf();
                    smol::spawn(async move { super::ensure_shared_target_dir_in(&root).await })
                })
                .collect();
            for task in tasks {
                task.await.expect("ensure shared target dir");
            }

            let metadata =
                smol::fs::read_to_string(metadata_path(&cache_root.path().join("target")))
                    .await
                    .expect("read metadata");
            let metadata: super::CacheMetadata =
                toml::from_str(&metadata).expect("marker parses after concurrent writes");
            assert_eq!(metadata.cli_commit, CLI_COMMIT);
        });
    }

    /// Discovery reads the shared target's marker directly and never descends
    /// into the tree — a `managed_backends` directory inside `target/` is not
    /// a cache entry and must not surface, while the real ones still do.
    #[test]
    fn discovery_never_walks_inside_the_shared_target() {
        smol::block_on(async {
            let cache_root = tempdir().expect("cache root");
            let target_dir = super::ensure_shared_target_dir_in(cache_root.path())
                .await
                .expect("ensure shared target dir");
            // A marker-shaped dir buried in the target tree — the shape a
            // recursive walk would wrongly report.
            let buried = target_dir.join("debug/managed_backends");
            smol::fs::create_dir_all(&buried)
                .await
                .expect("create buried dir");
            smol::fs::write(
                buried.join(super::METADATA_FILE_NAME),
                "project_root = \"x\"",
            )
            .await
            .expect("write buried marker");

            let project = tempdir().expect("project dir");
            let config = WaterConfig::default();
            let managed =
                super::ensure_project_build_cache_in(project.path(), cache_root.path(), &config)
                    .await
                    .expect("ensure managed cache");

            let mut discovered = super::discover_managed_build_cache_dirs(cache_root.path())
                .await
                .expect("discover cache dirs");
            let mut expected = vec![managed, target_dir];
            discovered.sort();
            expected.sort();
            assert_eq!(discovered, expected);
        });
    }

    /// Dropping the shared target while a build holds a shared lease on it
    /// refuses rather than deleting a live build's tree; once the lease is
    /// released the drop goes through.
    #[test]
    fn shared_target_dir_refuses_while_a_build_holds_its_lease() {
        smol::block_on(async {
            let cache_root = tempdir().expect("cache root");
            let target_dir = super::ensure_shared_target_dir_in(cache_root.path())
                .await
                .expect("ensure shared target dir");
            let lease = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(super::shared_target_lease_path(&target_dir))
                .expect("open the build lease");
            fs4::FileExt::lock_shared(&lease).expect("hold a shared lease");

            let error = super::remove_shared_target_dir_in(cache_root.path())
                .await
                .expect_err("a held build lease must refuse the drop");
            assert!(
                error.to_string().contains("in use"),
                "the error says why: {error}"
            );
            assert!(target_dir.exists());

            drop(lease);
            super::remove_shared_target_dir_in(cache_root.path())
                .await
                .expect("a released target drops")
                .expect("the target existed");
        });
    }

    /// A build whose `lock_shared` waits out an in-progress removal must not
    /// come back holding a lock on a deleted inode: the granted lock is
    /// verified against the path, the tree is re-ensured, and the returned
    /// lease is the file the path names now.
    ///
    /// Unix only: the scenario needs a tree deleted while a lease file
    /// inside it is still open, which Windows' delete sharing does not
    /// allow a test to stage.
    #[cfg(unix)]
    #[test]
    fn a_lease_waiting_out_a_removal_returns_the_live_lease_file() {
        smol::block_on(async {
            let cache_root = tempdir().expect("cache root");
            let target_dir = super::ensure_shared_target_dir_in(cache_root.path())
                .await
                .expect("ensure shared target dir");
            let lease_path = super::shared_target_lease_path(&target_dir);

            // A remover holds the exclusive lease the build will wait on.
            let remover =
                super::open_shared_target_lease_file(&target_dir).expect("open the lease file");
            fs4::FileExt::lock(&remover).expect("hold the exclusive lease");

            let waiting_dir = target_dir.clone();
            let (entered, wait) = std::sync::mpsc::channel::<()>();
            let build = std::thread::spawn(move || {
                entered
                    .send(())
                    .expect("report the build entering its lease");
                smol::block_on(super::lease_shared_target_dir(&waiting_dir))
            });
            // The build has entered the lease call by the time the removal
            // runs; whether it is already parked in `lock_shared` or opens a
            // path that just vanished, the outcome is the same.
            wait.recv().expect("the build thread is running");

            // Move the tree aside in one step, then delete it: an open in
            // flight either lands inside the moved tree — which the removal
            // then takes with it — or misses the path outright, so the walk
            // can never race a re-created `.build-lease` into ENOTEMPTY.
            let aside = cache_root.path().join("removing");
            std::fs::rename(&target_dir, &aside).expect("move the tree aside");
            std::fs::remove_dir_all(&aside).expect("remove the tree");
            drop(remover);

            let lease = build
                .join()
                .expect("join the build thread")
                .expect("the waiting lease resolves");
            let locked =
                same_file::Handle::from_file(lease.try_clone().expect("clone the lease fd"))
                    .expect("stat the held lease file");
            let named =
                same_file::Handle::from_path(&lease_path).expect("stat the path's lease file");
            assert_eq!(locked, named, "the held lease is the file the path names");
            assert!(
                metadata_path(&target_dir).is_file(),
                "the rebuilt target carries its marker"
            );
        });
    }

    /// A project clean removes exactly the units Cargo named after this
    /// project's generated crates — uplifted outputs, `deps/`, `.fingerprint/`,
    /// `build/` and `incremental/` entries, in every variant and profile — and
    /// leaves every other project's units and the shared dependency artifacts,
    /// the uplifted runtime dylib included, where they are.
    #[test]
    fn project_clean_removes_only_this_projects_units_from_the_shared_target() {
        smol::block_on(async {
            let cache_root = tempdir().expect("cache root");
            let target_dir = super::ensure_shared_target_dir_in(cache_root.path())
                .await
                .expect("ensure shared target dir");
            let ours = "e2eapp-hydrolysis-1a2b3c4d";
            let theirs = "e2eapp-hydrolysis-9f8e7d6c";
            let mut kept = Vec::new();
            let mut gone = Vec::new();
            for profile in [
                "shared/x86_64-pc-windows-msvc/debug",
                "static/x86_64-pc-windows-msvc/release",
                "host/debug",
            ] {
                let profile = target_dir.join(profile);
                for dir in ["deps", ".fingerprint", "build", "incremental"] {
                    std::fs::create_dir_all(profile.join(dir)).expect("profile dir");
                }
                std::fs::write(profile.join(".cargo-lock"), []).expect("cargo lock");
                // (relative path, is a unit directory, belongs to `ours`)
                let entries = [
                    ("e2eapp_hydrolysis_1a2b3c4d.exe", false, true),
                    ("e2eapp_hydrolysis_1a2b3c4d.pdb", false, true),
                    ("e2eapp_hydrolysis_1a2b3c4d.d", false, true),
                    ("e2eapp_hydrolysis_1a2b3c4d_cef_helper.exe", false, true),
                    ("deps/e2eapp_hydrolysis_1a2b3c4d-0011.exe", false, true),
                    ("deps/libe2eapp_hydrolysis_1a2b3c4d-0022.rlib", false, true),
                    ("deps/e2eapp_hydrolysis_1a2b3c4d-0022.d", false, true),
                    (".fingerprint/e2eapp-hydrolysis-1a2b3c4d-0011", true, true),
                    ("build/e2eapp-hydrolysis-1a2b3c4d-0033", true, true),
                    ("incremental/e2eapp_hydrolysis_1a2b3c4d-abc", true, true),
                    ("e2eapp_hydrolysis_9f8e7d6c.exe", false, false),
                    ("deps/libe2eapp_hydrolysis_9f8e7d6c-0044.rlib", false, false),
                    (".fingerprint/e2eapp-hydrolysis-9f8e7d6c-0044", true, false),
                    ("waterui_dylib.dll", false, false),
                    ("deps/waterui_dylib-0055.dll", false, false),
                    ("deps/libwaterui-0066.rlib", false, false),
                    (".fingerprint/waterui-dylib-0055", true, false),
                    ("build/waterui-chromium-0077", true, false),
                ];
                for (relative, is_dir, is_ours) in entries {
                    let path = profile.join(relative);
                    if is_dir {
                        std::fs::create_dir_all(path.join("output")).expect("unit dir");
                    } else {
                        std::fs::write(&path, []).expect("unit file");
                    }
                    if is_ours { &mut gone } else { &mut kept }.push(path);
                }
            }
            // A directory outside any profile is never inspected.
            let metadata = target_dir.join("metadata.toml");
            assert!(metadata.is_file(), "the shared target keeps its metadata");

            let removed = super::remove_project_units_in(&target_dir, &[ours.to_owned()])
                .await
                .expect("clean this project's units");

            gone.sort();
            assert_eq!(removed, gone, "exactly this project's units are reported");
            for path in &gone {
                assert!(!path.exists(), "{} was removed", path.display());
            }
            for path in &kept {
                assert!(path.exists(), "{} stays", path.display());
            }
            assert!(metadata.is_file());
            assert!(
                super::remove_project_units_in(&target_dir, &[theirs.to_owned()])
                    .await
                    .expect("clean the other project")
                    .iter()
                    .all(|path| kept.contains(path)),
                "the other project's clean removes only what this one kept"
            );
            assert!(
                super::remove_project_units_in(&target_dir, &[ours.to_owned()])
                    .await
                    .expect("a second clean")
                    .is_empty(),
                "a second clean finds nothing"
            );
        });
    }

    /// A project clean of the shared target refuses while a build holds a
    /// shared lease on it, like the full drop does.
    #[test]
    fn project_clean_of_the_shared_target_refuses_while_a_build_holds_its_lease() {
        smol::block_on(async {
            let cache_root = tempdir().expect("cache root");
            let target_dir = super::ensure_shared_target_dir_in(cache_root.path())
                .await
                .expect("ensure shared target dir");
            let profile = target_dir.join("shared/aarch64-apple-darwin/debug");
            std::fs::create_dir_all(&profile).expect("profile dir");
            std::fs::write(profile.join("demo_hydrolysis_0000abcd"), []).expect("unit");
            std::fs::write(profile.join(".cargo-lock"), []).expect("cargo lock");
            let lease = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(super::shared_target_lease_path(&target_dir))
                .expect("open the build lease");
            fs4::FileExt::lock_shared(&lease).expect("hold a shared lease");

            let packages = ["demo-hydrolysis-0000abcd".to_owned()];
            let error = super::remove_project_units_in(&target_dir, &packages)
                .await
                .expect_err("a held build lease must refuse the clean");
            assert!(error.to_string().contains("in use"), "{error}");
            assert!(profile.join("demo_hydrolysis_0000abcd").is_file());

            drop(lease);
            let removed = super::remove_project_units_in(&target_dir, &packages)
                .await
                .expect("a released target cleans");
            assert_eq!(removed, [profile.join("demo_hydrolysis_0000abcd")]);
        });
    }

    #[test]
    fn unit_entries_belong_to_their_package_only() {
        let package = "demo-hydrolysis-0000abcd";
        for name in [
            "demo-hydrolysis-0000abcd-1234",
            "demo_hydrolysis_0000abcd",
            "demo_hydrolysis_0000abcd.exe",
            "demo_hydrolysis_0000abcd-1234.d",
            "libdemo_hydrolysis_0000abcd-1234.rlib",
            "demo_hydrolysis_0000abcd_cef_helper.pdb",
        ] {
            assert!(super::unit_entry_belongs_to(name, package), "{name}");
        }
        for name in [
            "demo-hydrolysis-0000abcd1-1234",
            "demo_hydrolysis_0000abce",
            "demo_hydrolysis",
            "waterui_dylib.dll",
            "libdemo-0000.rlib",
        ] {
            assert!(!super::unit_entry_belongs_to(name, package), "{name}");
        }
    }

    /// The survey reports the shared target, and an explicit drop removes it
    /// immediately instead of waiting out the unused-days policy.
    #[test]
    fn shared_target_dir_can_be_dropped_on_demand() {
        smol::block_on(async {
            let cache_root = tempdir().expect("cache root");
            let target_dir = super::ensure_shared_target_dir_in(cache_root.path())
                .await
                .expect("ensure shared target dir");
            smol::fs::create_dir_all(target_dir.join("debug"))
                .await
                .expect("create a unit dir");
            smol::fs::write(target_dir.join("debug/unit.rlib"), [0u8; 1024])
                .await
                .expect("write a unit");

            let freed = super::remove_shared_target_dir_in(cache_root.path())
                .await
                .expect("drop the shared target");
            assert!(
                freed.is_some_and(|bytes| bytes > 0),
                "the drop reports the space it held: {freed:?}"
            );
            assert!(!target_dir.exists());
            assert_eq!(
                super::remove_shared_target_dir_in(cache_root.path())
                    .await
                    .expect("a second drop"),
                None,
                "dropping an absent shared target is a no-op"
            );
        });
    }

    #[test]
    fn shared_target_dir_is_discovered_and_kept_while_in_use() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let config = WaterConfig::default();
            let cache_root = tempdir().expect("cache root");

            let target_dir = super::ensure_shared_target_dir_in(cache_root.path())
                .await
                .expect("ensure shared target dir");

            assert_eq!(target_dir, cache_root.path().join("target"));

            let discovered = super::discover_managed_build_cache_dirs(cache_root.path())
                .await
                .expect("discover cache dirs");
            assert_eq!(discovered, vec![target_dir.clone()]);

            let outcome =
                super::cleanup_stale_caches_if_idle(cache_root.path(), project.path(), &config)
                    .await
                    .expect("cleanup caches");
            assert_eq!(
                outcome,
                super::BuildCacheGcOutcome::Ran(super::BuildCacheGcSummary {
                    scanned_entries: 1,
                    removed_entries: 0,
                })
            );
            assert!(target_dir.exists());
        });
    }

    #[test]
    fn stale_shared_target_dir_is_collected() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let config = WaterConfig::default();
            let cache_root = tempdir().expect("cache root");

            let target_dir = super::ensure_shared_target_dir_in(cache_root.path())
                .await
                .expect("ensure shared target dir");
            let stale_metadata = super::CacheMetadata {
                project_root: target_dir.display().to_string(),
                cli_commit: CLI_COMMIT.to_string(),
                last_used_unix_seconds: 1,
            };
            smol::fs::write(
                metadata_path(&target_dir),
                toml::to_string(&stale_metadata).expect("serialize stale metadata"),
            )
            .await
            .expect("write stale metadata");

            let outcome =
                super::cleanup_stale_caches_if_idle(cache_root.path(), project.path(), &config)
                    .await
                    .expect("cleanup caches");
            assert_eq!(
                outcome,
                super::BuildCacheGcOutcome::Ran(super::BuildCacheGcSummary {
                    scanned_entries: 1,
                    removed_entries: 1,
                })
            );
            assert!(!target_dir.exists());
        });
    }

    /// Staleness alone is not enough to drop the shared Cargo target: a
    /// shared lease held by a running build keeps the exclusive lease — and
    /// with it removal — out of the sweep's reach, and once the build lets
    /// go the stale target is collected again.
    #[test]
    fn cleanup_keeps_the_shared_target_while_a_build_holds_its_lease() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let config = WaterConfig::default();
            let cache_root = tempdir().expect("cache root");

            let target_dir = super::ensure_shared_target_dir_in(cache_root.path())
                .await
                .expect("ensure shared target dir");
            let lease = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(super::shared_target_lease_path(&target_dir))
                .expect("open the build lease");
            fs4::FileExt::lock_shared(&lease).expect("hold a shared lease");

            // Stale enough that the unused-days policy would collect the
            // target were no build holding it.
            let stale_metadata = super::CacheMetadata {
                project_root: target_dir.display().to_string(),
                cli_commit: CLI_COMMIT.to_string(),
                last_used_unix_seconds: 1,
            };
            smol::fs::write(
                metadata_path(&target_dir),
                toml::to_string(&stale_metadata).expect("serialize stale metadata"),
            )
            .await
            .expect("write stale metadata");

            let outcome =
                super::cleanup_stale_caches_if_idle(cache_root.path(), project.path(), &config)
                    .await
                    .expect("cleanup caches");

            assert_eq!(
                outcome,
                super::BuildCacheGcOutcome::Ran(super::BuildCacheGcSummary {
                    scanned_entries: 1,
                    removed_entries: 0,
                })
            );
            assert!(target_dir.exists(), "a live build's target must survive");

            drop(lease);
            let outcome =
                super::cleanup_stale_caches_if_idle(cache_root.path(), project.path(), &config)
                    .await
                    .expect("cleanup once the lease is released");
            assert_eq!(
                outcome,
                super::BuildCacheGcOutcome::Ran(super::BuildCacheGcSummary {
                    scanned_entries: 1,
                    removed_entries: 1,
                })
            );
            assert!(!target_dir.exists());
        });
    }

    /// An entry whose marker cannot be read is not proven stale. The marker
    /// is rewritten every time the owning project opens, so an unreadable
    /// file is just as likely a build in flight as garbage — the sweep keeps
    /// it either way.
    #[test]
    fn cleanup_keeps_an_entry_whose_metadata_cannot_be_read() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let config = WaterConfig::default();
            let cache_root = tempdir().expect("cache root");

            let entry = cache_root.path().join("Users/lexo/demo/managed_backends");
            smol::fs::create_dir_all(&entry)
                .await
                .expect("create cache entry");
            smol::fs::write(entry.join("marker.bin"), [0u8; 16])
                .await
                .expect("write entry contents");
            // The shape a concurrent read sees while another process rewrites
            // the marker: the file exists, its contents are not a document.
            smol::fs::write(metadata_path(&entry), "project_root = [")
                .await
                .expect("write partial metadata");

            let outcome =
                super::cleanup_stale_caches_if_idle(cache_root.path(), project.path(), &config)
                    .await
                    .expect("cleanup caches");

            assert_eq!(
                outcome,
                super::BuildCacheGcOutcome::Ran(super::BuildCacheGcSummary {
                    scanned_entries: 1,
                    removed_entries: 0,
                })
            );
            assert!(entry.join("marker.bin").exists());
        });
    }

    /// The shared target gets the same benefit of the doubt a project entry
    /// does: an unreadable marker is not proof it is stale.
    #[test]
    fn cleanup_keeps_the_shared_target_whose_metadata_cannot_be_read() {
        smol::block_on(async {
            let project = tempdir().expect("project dir");
            let config = WaterConfig::default();
            let cache_root = tempdir().expect("cache root");

            let target_dir = super::ensure_shared_target_dir_in(cache_root.path())
                .await
                .expect("ensure shared target dir");
            smol::fs::write(metadata_path(&target_dir), "project_root = [")
                .await
                .expect("write partial metadata");

            let outcome =
                super::cleanup_stale_caches_if_idle(cache_root.path(), project.path(), &config)
                    .await
                    .expect("cleanup caches");

            assert_eq!(
                outcome,
                super::BuildCacheGcOutcome::Ran(super::BuildCacheGcSummary {
                    scanned_entries: 1,
                    removed_entries: 0,
                })
            );
            assert!(target_dir.exists());
        });
    }
}