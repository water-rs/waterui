//! The payload an Android preview run ships into the preview host's private
//! files: the stripped launcher libraries and the project's asset bundle.
//!
//! The payload is two [`PayloadPart`]s, each a directory under
//! [`PAYLOAD_DIR`] identified by its own content hash. On the device each
//! part's hash is recorded in the run directory as the part's
//! [`PayloadPart::stamp_file`], written only once the part's files are in
//! place, so a stamp vouches for the files beside it. A run ships only the
//! parts whose hash differs from the device's stamp.
//!
//! Locally, `<backend>/android-preview/` holds [`PAYLOAD_DIR`], laid out
//! exactly as it lands on the device so a changed part is pushed as it is,
//! and [`STRIP_RECORD_FILE`]. The libraries are a cache keyed by the
//! unstripped cdylib's hash, so `llvm-strip` and the library scans run only
//! when the compiled artifact changes; the asset bundle is staged
//! incrementally against its own stamp. Anything else there is removed.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use eyre::{Context as _, Result, bail};
use futures_util::TryStreamExt as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use smol::fs;

use crate::android::KotlinToolchain;
use crate::android::platform::{AndroidAbi, ndk_llvm_tool};
use crate::build::{BuildOptions, BuildProfile};
use crate::hydrolysis::android::{self as hydrolysis_android, HydrolysisAndroidBuild};
use crate::hydrolysis::backend::HydrolysisBackend;
use crate::preview::hydrolysis::HydrolysisPreviewRequest;
use crate::project::Project;
use crate::project_model::assets;
use crate::toolchain::Host;
use crate::utils::hash_file_into;

/// The payload directory: under `<backend>/android-preview/` locally, and
/// inside the device run directory.
pub(super) const PAYLOAD_DIR: &str = "payload";

/// The record beside [`PAYLOAD_DIR`] naming the artifact the cached
/// stripped libraries were made from. It lives outside the payload, so it
/// is never pushed.
const STRIP_RECORD_FILE: &str = "strip-source.json";

/// The subdirectory of the preview cache holding one [`InstallRecord`]
/// per device — `devices/<device key>.json` beside [`PAYLOAD_DIR`].
const DEVICES_DIR: &str = "devices";

/// The version of the payload's shape its content hashes do not cover:
/// the parts' layout inside the device run directory and the contract the
/// instrumentation reads. It is hashed into every part's stamp, so raising
/// it ships every part again; raise it with any change to those shapes that
/// leaves the hashed paths alone.
const PAYLOAD_FORMAT_VERSION: u32 = 1;

/// One directory of the payload, shipped and stamped on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PayloadPart {
    /// The stripped launcher libraries.
    Libraries,
    /// The project's asset bundle.
    Resources,
}

impl PayloadPart {
    /// Every part, in the order the device reports their stamps.
    pub(super) const ALL: [Self; 2] = [Self::Libraries, Self::Resources];

    /// The part's directory inside [`PAYLOAD_DIR`].
    pub(super) const fn dir(self) -> &'static str {
        match self {
            Self::Libraries => "lib",
            Self::Resources => "resources",
        }
    }

    /// The file in the device run directory recording the hash of the part
    /// in place there.
    pub(super) const fn stamp_file(self) -> &'static str {
        match self {
            Self::Libraries => "lib.stamp",
            Self::Resources => "resources.stamp",
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Libraries => 0,
            Self::Resources => 1,
        }
    }
}

/// The stamps a device holds, one per [`PayloadPart`], `None` for a part it
/// holds none of.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct HeldStamps([Option<String>; 2]);

impl HeldStamps {
    /// Read the stamps a run preparation printed: one line per part, in
    /// [`PayloadPart::ALL`] order, empty for a part without a stamp.
    ///
    /// # Errors
    /// Returns an error when the output is not one line per part.
    pub(super) fn parse(output: &str) -> Result<Self> {
        let lines: Vec<&str> = output.lines().collect();
        let [libraries, resources] = lines.as_slice() else {
            bail!(
                "the run preparation printed {} lines where one stamp line per payload part \
                 was expected: {output:?}",
                lines.len()
            );
        };
        let held = |line: &str| (!line.is_empty()).then(|| line.to_string());
        Ok(Self([held(libraries), held(resources)]))
    }

    /// The stamp the device holds for `part`.
    fn get(&self, part: PayloadPart) -> Option<&str> {
        self.0[part.index()].as_deref()
    }
}

/// The Mac-side record of the payload stamps one device's run directory
/// held when this project's last install there finished — the prediction
/// the overlapped push is built on. It lives in the preview cache as
/// `devices/<device key>.json` beside [`PAYLOAD_DIR`], is rewritten to
/// the state each completed install leaves, and is only ever a
/// prediction: the stamps the run preparation reads off the device stay
/// the authority.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct InstallRecord {
    /// The stamp each [`PayloadPart`] held, keyed by [`PayloadPart::dir`];
    /// a part without an entry is one the device held no stamp for.
    pub(super) held: BTreeMap<String, String>,
}

impl InstallRecord {
    /// The record of the state an install leaves: every part `installed`
    /// holds its payload stamp, every other part the stamp `held`
    /// reported.
    pub(super) fn after_install(
        held: &HeldStamps,
        installed: &[PayloadPart],
        payload: &DevicePayload,
    ) -> Self {
        Self {
            held: PayloadPart::ALL
                .into_iter()
                .filter_map(|part| {
                    let stamp = if installed.contains(&part) {
                        Some(payload.stamp(part))
                    } else {
                        held.get(part)
                    }?;
                    Some((part.dir().to_string(), stamp.to_string()))
                })
                .collect(),
        }
    }

    /// The device's stamps as the record predicts them.
    pub(super) fn held_stamps(&self) -> HeldStamps {
        HeldStamps(PayloadPart::ALL.map(|part| self.held.get(part.dir()).cloned()))
    }
}

/// One staged part: its content hash and the bytes its files hold.
#[derive(Debug)]
struct StagedPart {
    stamp: String,
    size: u64,
}

/// A staged preview payload: its preview cache root, its local directory
/// laid out as it lands on the device, the libraries in `System.load`
/// order, and each part's content hash and size.
#[derive(Debug)]
pub(super) struct DevicePayload {
    cache_root: PathBuf,
    dir: PathBuf,
    libraries: Vec<String>,
    parts: [StagedPart; 2],
}

/// What the cached stripped libraries were made from, and what they are.
#[derive(Debug, Serialize, Deserialize)]
struct StripRecord {
    /// SHA-256 of the unstripped launcher cdylib.
    source_sha256: String,
    /// The NDK whose `llvm-strip` and `libc++_shared.so` produced the cache.
    ndk: PathBuf,
    /// The staged libraries in `System.load` order.
    libraries: Vec<String>,
}

impl DevicePayload {
    /// Build the launcher's preview-mode cdylib for `abi`, bring the
    /// stripped-library cache up to date with it, stage the project's
    /// assets, and hash each part.
    ///
    /// # Errors
    /// Returns an error when the build, the strip, the asset staging or the
    /// hashing fails.
    pub(super) async fn stage(
        project: &Project,
        host: &Host,
        request: &HydrolysisPreviewRequest<'_>,
        abi: AndroidAbi,
        kotlin: &KotlinToolchain,
    ) -> Result<Self> {
        let root = project
            .backend_path::<HydrolysisBackend>()
            .join("android-preview");
        let dir = root.join(PAYLOAD_DIR);
        prune_to_layout(&root, &dir).await?;
        let mut options = BuildOptions::development(BuildProfile::Debug);
        if let Some(sccache_path) = request.sccache_path.clone() {
            options = options.with_sccache(sccache_path);
        }
        if let Some(progress) = request.progress.clone() {
            options = options.with_progress(progress);
        }
        let build = hydrolysis_android::build_with_features(
            project,
            abi,
            options,
            &["waterui-preview-mode"],
            kotlin,
        )
        .await?;

        // Asset mounts ship as the library layout's `waterui_assets`
        // directory, staged through the shared planner and read from there.
        let symbols = build.built.app_symbols().await?;
        let resources_dir = dir.join(PayloadPart::Resources.dir());
        let (libraries, bundle_stamp) = futures_util::try_join!(
            stage_stripped_libraries(project, host, abi, &build, &root),
            async {
                let manifest = assets::plan_library_resources(project, &symbols, false).await?;
                assets::write_library_resources(&manifest, &resources_dir).await
            },
        )?;
        Self::from_staged(root, libraries, bundle_stamp).await
    }

    /// The payload inside `cache_root`, the preview cache holding
    /// [`PAYLOAD_DIR`]: the libraries `libraries` in its
    /// [`PayloadPart::Libraries`] directory and the asset bundle in its
    /// [`PayloadPart::Resources`] directory, whose staging answered
    /// `bundle_stamp`, hashed.
    ///
    /// The libraries are made read-only where they are staged: `adb push`
    /// propagates the source file's mode and the install copies with
    /// `cp -Rp`, so the linker's read-only `System.load` requirement is
    /// met on this side of the push and the device never runs a `chmod`
    /// pass. The bundle's stamp already identifies its contents, so only
    /// the libraries are read: the resources' hash is that stamp's.
    ///
    /// # Errors
    /// Returns an error when a part cannot be walked or a library cannot
    /// be read or marked read-only.
    pub(super) async fn from_staged(
        cache_root: PathBuf,
        libraries: Vec<String>,
        bundle_stamp: String,
    ) -> Result<Self> {
        smol::unblock(move || {
            let dir = cache_root.join(PAYLOAD_DIR);
            let lib_dir = dir.join(PayloadPart::Libraries.dir());
            let mut hasher = Sha256::new();
            hasher.update(PAYLOAD_FORMAT_VERSION.to_le_bytes());
            for name in &libraries {
                let path = lib_dir.join(name);
                let mut permissions = std::fs::metadata(&path)
                    .wrap_err_with(|| format!("failed to stat {}", path.display()))?
                    .permissions();
                if !permissions.readonly() {
                    permissions.set_readonly(true);
                    std::fs::set_permissions(&path, permissions)
                        .wrap_err_with(|| format!("failed to make {} read-only", path.display()))?;
                }
                hasher.update(name.as_bytes());
                hasher.update([0]);
                hash_file_into(&mut hasher, &path)?;
            }
            let libraries_part = StagedPart {
                stamp: hex::encode(hasher.finalize()),
                size: tree_size(&lib_dir)?,
            };

            let mut hasher = Sha256::new();
            hasher.update(PAYLOAD_FORMAT_VERSION.to_le_bytes());
            hasher.update(bundle_stamp.as_bytes());
            let resources_part = StagedPart {
                stamp: hex::encode(hasher.finalize()),
                size: tree_size(&dir.join(PayloadPart::Resources.dir()))?,
            };
            Ok(Self {
                cache_root,
                dir,
                libraries,
                parts: [libraries_part, resources_part],
            })
        })
        .await
    }

    /// The local payload directory, laid out as it lands on the device.
    pub(super) fn dir(&self) -> &Path {
        &self.dir
    }

    /// The [`InstallRecord`] file for `device_key` —
    /// `devices/<device key>.json` inside the preview cache, beside
    /// [`PAYLOAD_DIR`]. The key is sanitized the same way the device lock
    /// names it.
    pub(super) fn install_record_path(&self, device_key: &str) -> PathBuf {
        self.cache_root.join(DEVICES_DIR).join(format!(
            "{}.json",
            crate::project_model::water_dir::sanitize_os_str(device_key.as_ref())
        ))
    }

    /// `part`'s content hash.
    pub(super) fn stamp(&self, part: PayloadPart) -> &str {
        &self.parts[part.index()].stamp
    }

    /// The parts whose stamp on the device is missing or differs from
    /// theirs, in [`PayloadPart::ALL`] order.
    pub(super) fn stale_parts(&self, held: &HeldStamps) -> Vec<PayloadPart> {
        PayloadPart::ALL
            .into_iter()
            .filter(|part| held.get(*part) != Some(self.stamp(*part)))
            .collect()
    }

    /// The bytes `parts`' files hold.
    pub(super) fn size(&self, parts: &[PayloadPart]) -> u64 {
        parts.iter().map(|part| self.parts[part.index()].size).sum()
    }

    /// The libraries' paths relative to the device run directory, in
    /// `System.load` order.
    pub(super) fn library_paths(&self) -> Vec<String> {
        self.libraries
            .iter()
            .map(|name| format!("{PAYLOAD_DIR}/{}/{name}", PayloadPart::Libraries.dir()))
            .collect()
    }

    /// The asset bundle's path relative to the device run directory.
    pub(super) fn assets_root() -> String {
        format!(
            "{PAYLOAD_DIR}/{}/{}",
            PayloadPart::Resources.dir(),
            assets::ANDROID_ASSET_BUNDLE_DIR
        )
    }
}

/// The bytes the regular files under `dir` hold.
fn tree_size(dir: &Path) -> io::Result<u64> {
    let mut size = 0;
    for entry in walkdir::WalkDir::new(dir) {
        let entry = entry?;
        if entry.file_type().is_file() {
            size += entry.metadata()?.len();
        }
    }
    Ok(size)
}

/// Remove every entry of `root` its layout does not name — everything but
/// `payload`, [`STRIP_RECORD_FILE`] and [`DEVICES_DIR`] — and every entry
/// of `payload` that is not a [`PayloadPart`] directory.
async fn prune_to_layout(root: &Path, payload: &Path) -> Result<()> {
    let payload_name = payload
        .file_name()
        .ok_or_else(|| eyre::eyre!("{} names no directory", payload.display()))?;
    remove_unlisted(
        root,
        &[
            payload_name,
            OsStr::new(STRIP_RECORD_FILE),
            OsStr::new(DEVICES_DIR),
        ],
    )
    .await?;
    remove_unlisted(
        payload,
        &PayloadPart::ALL.map(|part| OsStr::new(part.dir())),
    )
    .await
}

/// Remove every entry of `dir` whose name `keep` does not hold; a missing
/// `dir` holds nothing to remove.
async fn remove_unlisted(dir: &Path, keep: &[&OsStr]) -> Result<()> {
    let mut entries = match fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).wrap_err_with(|| format!("failed to list {}", dir.display()));
        }
    };
    while let Some(entry) = entries
        .try_next()
        .await
        .wrap_err_with(|| format!("failed to list {}", dir.display()))?
    {
        if keep.contains(&entry.file_name().as_os_str()) {
            continue;
        }
        let path = entry.path();
        let removed = if entry.file_type().await?.is_dir() {
            fs::remove_dir_all(&path).await
        } else {
            fs::remove_file(&path).await
        };
        removed.wrap_err_with(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

/// Remove the file at `path`, which may not exist.
async fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).wrap_err_with(|| format!("failed to remove {}", path.display())),
    }
}

/// Bring the stripped-library cache — the payload's
/// [`PayloadPart::Libraries`] directory under `root` — up to date with the
/// build's artifact and answer the staged libraries in `System.load` order.
///
/// The artifact is hashed in place: a [`StripRecord`] naming its hash and
/// the build's NDK means the cache already holds the strip of exactly
/// those bytes, so a hit costs only that one read. `water run` and
/// `water build` rewrite the same artifact without the preview's lock, so
/// only on a miss is the artifact snapshotted (a clone on APFS), the
/// snapshot re-hashed and the snapshot stripped — the record can never
/// name bytes other than the ones stripped.
///
/// The cache is current when its [`StripRecord`] names the artifact's
/// hash and the build's NDK and every library it lists is still in place;
/// on a miss it is rebuilt from scratch — the record removed with it
/// first, so an interrupted rebuild is never taken for a current one — and
/// the record written last.
async fn stage_stripped_libraries(
    project: &Project,
    host: &Host,
    abi: AndroidAbi,
    build: &HydrolysisAndroidBuild,
    root: &Path,
) -> Result<Vec<String>> {
    let lib_dir = root.join(PAYLOAD_DIR).join(PayloadPart::Libraries.dir());
    let record_path = root.join(STRIP_RECORD_FILE);
    let artifact_sha256 = smol::unblock({
        let artifact = build.built.artifact.clone();
        move || -> Result<String> {
            let mut hasher = Sha256::new();
            hash_file_into(&mut hasher, &artifact)
                .wrap_err_with(|| format!("failed to hash {}", artifact.display()))?;
            Ok(hex::encode(hasher.finalize()))
        }
    })
    .await?;
    let ndk = build.context.ndk_path.clone();
    if let Some(record) = read_strip_record(&record_path).await?
        && record.source_sha256 == artifact_sha256
        && record.ndk == ndk
        && libraries_present(&lib_dir, &record.libraries).await
    {
        return Ok(record.libraries);
    }

    let (snapshot_dir, snapshot, source_sha256) = smol::unblock({
        let artifact = build.built.artifact.clone();
        let root = root.to_path_buf();
        move || -> Result<_> {
            std::fs::create_dir_all(&root)
                .wrap_err_with(|| format!("failed to create {}", root.display()))?;
            // A temporary directory, not a file: the copy must create its
            // destination for the copy to be a clone.
            let snapshot_dir = tempfile::Builder::new()
                .prefix("artifact-snapshot-")
                .tempdir_in(&root)
                .wrap_err_with(|| {
                    format!("failed to create a snapshot dir in {}", root.display())
                })?;
            let snapshot = snapshot_dir.path().join(
                artifact
                    .file_name()
                    .ok_or_else(|| eyre::eyre!("{} names no file", artifact.display()))?,
            );
            std::fs::copy(&artifact, &snapshot)
                .wrap_err_with(|| format!("failed to snapshot {}", artifact.display()))?;
            let mut hasher = Sha256::new();
            hash_file_into(&mut hasher, &snapshot)
                .wrap_err_with(|| format!("failed to hash {}", artifact.display()))?;
            Ok((snapshot_dir, snapshot, hex::encode(hasher.finalize())))
        }
    })
    .await?;
    let libraries =
        refresh_stripped_libraries(project, host, abi, build, root, &snapshot, source_sha256).await;
    smol::unblock(move || snapshot_dir.close())
        .await
        .wrap_err("failed to remove the artifact snapshot")?;
    libraries
}

/// Whether every library a [`StripRecord`] names is still in `lib_dir`:
/// the record only vouches for a cache whose files are all still there —
/// a removed library must rebuild, not fail the payload's content hash on
/// every run.
async fn libraries_present(lib_dir: &Path, libraries: &[String]) -> bool {
    for name in libraries {
        match fs::metadata(lib_dir.join(name)).await {
            Ok(metadata) if metadata.is_file() => {}
            _ => return false,
        }
    }
    true
}

/// The strip half of [`stage_stripped_libraries`], reached only on a cache
/// miss: remove `root`'s [`StripRecord`] and clear the libraries' directory,
/// `llvm-strip` `snapshot` into it and stage the runtime libraries beside
/// it, then write the record naming the snapshot's hash and the build's
/// NDK.
async fn refresh_stripped_libraries(
    project: &Project,
    host: &Host,
    abi: AndroidAbi,
    build: &HydrolysisAndroidBuild,
    root: &Path,
    snapshot: &Path,
    source_sha256: String,
) -> Result<Vec<String>> {
    let ndk = build.context.ndk_path.clone();
    let lib_dir = &root.join(PAYLOAD_DIR).join(PayloadPart::Libraries.dir());
    let record_path = &root.join(STRIP_RECORD_FILE);

    remove_if_present(record_path).await?;
    match fs::remove_dir_all(lib_dir).await {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).wrap_err_with(|| format!("failed to clear {}", lib_dir.display()));
        }
    }
    fs::create_dir_all(lib_dir).await?;

    // Debug info is stripped: the cdylib carries a full desktop-sized symbol
    // set that only bloats the transfer. The build hands back the context it
    // resolved, so the strip runs under the same NDK without a second
    // resolve.
    let library_name = hydrolysis_android::launcher_library_name(project);
    let stripped = lib_dir.join(&library_name);
    let strip = smol::unblock({
        let ndk = ndk.clone();
        move || ndk_llvm_tool(&ndk, "llvm-strip")
    })
    .await?;
    host.run(
        &strip,
        [
            OsStr::new("--strip-debug"),
            OsStr::new("-o"),
            stripped.as_os_str(),
            snapshot.as_os_str(),
        ],
    )
    .await
    .map_err(|error| eyre::eyre!("llvm-strip on {} failed: {error}", snapshot.display()))?;
    let libraries =
        hydrolysis_android::stage_runtime_libraries(lib_dir, library_name, abi, &build.context)
            .await?;

    let record = serde_json::to_vec_pretty(&StripRecord {
        source_sha256,
        ndk,
        libraries: libraries.clone(),
    })
    .wrap_err("failed to serialize the strip record")?;
    fs::write(record_path, &record)
        .await
        .wrap_err_with(|| format!("failed to write {}", record_path.display()))?;
    Ok(libraries)
}

/// The cache's [`StripRecord`], `None` when there is none. A record that
/// does not parse was written by a different CLI version and names no
/// current cache — `None` as well, so the cache is rebuilt.
async fn read_strip_record(path: &Path) -> Result<Option<StripRecord>> {
    let bytes = match fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).wrap_err_with(|| format!("failed to read {}", path.display()));
        }
    };
    Ok(serde_json::from_slice(&bytes)
        .inspect_err(|error| {
            tracing::debug!(
                record = %path.display(),
                %error,
                "the strip record does not parse; rebuilding the stripped libraries"
            );
        })
        .ok())
}

/// The [`InstallRecord`] `path` holds, `None` when there is none or it
/// does not parse — an unreadable record predicts nothing, so the run
/// takes the serial path and rewrites it.
///
/// # Errors
/// Returns an error when the record cannot be read (other than its
/// absence).
pub(super) async fn read_install_record(path: &Path) -> Result<Option<InstallRecord>> {
    let bytes = match fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).wrap_err_with(|| format!("failed to read {}", path.display()));
        }
    };
    Ok(serde_json::from_slice(&bytes)
        .inspect_err(|error| {
            tracing::debug!(
                record = %path.display(),
                %error,
                "the install record does not parse; taking the serial push path"
            );
        })
        .ok())
}

/// Write `record` as the [`InstallRecord`] at `path`.
///
/// # Errors
/// Returns an error when the record cannot be serialized or written.
pub(super) async fn write_install_record(path: &Path, record: &InstallRecord) -> Result<()> {
    let bytes =
        serde_json::to_vec_pretty(record).wrap_err("failed to serialize the install record")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .wrap_err_with(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, &bytes)
        .await
        .wrap_err_with(|| format!("failed to write {}", path.display()))
}
