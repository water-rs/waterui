//! The payload an Android preview run ships into the preview host's private
//! files: the stripped launcher libraries and the project's asset bundle,
//! identified by one content hash.
//!
//! On the device the payload lives in [`PAYLOAD_DIR`] under the run
//! directory, beside [`STAMP_FILE`], which records the content hash of the
//! payload extracted there. A run whose hash matches the device's stamp
//! ships nothing; any other run streams the payload as a tar archive whose
//! last member is the new stamp, written as [`INCOMING_STAMP_FILE`] and
//! renamed into place only after the whole archive extracted.
//!
//! Locally, the stripped libraries are a cache keyed by the unstripped
//! cdylib's hash: `llvm-strip` and the library scans run only when the
//! compiled artifact changes.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io::{self, BufWriter, Read as _, Write};
use std::path::{Component, Path, PathBuf};

use eyre::{Context as _, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use smol::fs;

use crate::android::platform::{AndroidAbi, ndk_llvm_tool};
use crate::build::{BuildOptions, BuildProfile};
use crate::hydrolysis::android::{self as hydrolysis_android, HydrolysisAndroidBuild};
use crate::hydrolysis::backend::HydrolysisBackend;
use crate::preview::hydrolysis::HydrolysisPreviewRequest;
use crate::project::Project;
use crate::project_model::assets;
use crate::toolchain::Host;

/// The payload directory inside the device run directory.
pub(super) const PAYLOAD_DIR: &str = "payload";

/// The device run directory's record of the payload extracted in
/// [`PAYLOAD_DIR`]: its content hash, present only once every file of it
/// landed.
pub(super) const STAMP_FILE: &str = "payload.stamp";

/// The archive member carrying the new stamp. It is the archive's last
/// member, so it exists only when every payload file before it extracted.
pub(super) const INCOMING_STAMP_FILE: &str = "payload.stamp.new";

/// The libraries' directory inside the payload, and the local directory
/// under `<backend>/android-preview/` the stripped libraries are cached in.
const LIBRARY_DIR: &str = "lib";

/// The directory the asset bundle sits in inside the payload.
const RESOURCES_DIR: &str = "resources";

/// The record beside the cached stripped libraries naming the artifact they
/// were made from. It lives inside [`LIBRARY_DIR`], so removing the cache
/// removes the record with it.
const STRIP_RECORD_FILE: &str = "strip-source.json";

/// The buffer the hashing reads and the archive writes go through.
const IO_CHUNK: usize = 64 * 1024;

/// One file the payload carries.
#[derive(Debug, Clone)]
struct PayloadEntry {
    /// The `/`-separated path inside [`PAYLOAD_DIR`].
    path: String,
    /// The local file holding its bytes.
    source: PathBuf,
}

/// A staged preview payload: its files, the libraries in `System.load`
/// order, and the content hash a device stamp is compared with.
#[derive(Debug)]
pub(super) struct DevicePayload {
    entries: Vec<PayloadEntry>,
    libraries: Vec<String>,
    stamp: String,
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
    /// assets, and hash the result.
    ///
    /// # Errors
    /// Returns an error when the build, the strip, the asset staging or the
    /// hashing fails.
    pub(super) async fn stage(
        project: &Project,
        host: &Host,
        request: &HydrolysisPreviewRequest<'_>,
        abi: AndroidAbi,
    ) -> Result<Self> {
        let root = project
            .backend_path::<HydrolysisBackend>()
            .join("android-preview");
        let mut options = BuildOptions::development(BuildProfile::Debug);
        if let Some(sccache_path) = request.sccache_path.clone() {
            options = options.with_sccache(sccache_path);
        }
        if let Some(progress) = request.progress.clone() {
            options = options.with_progress(progress);
        }
        let build = hydrolysis_android::build_with_features(
            project,
            host,
            abi,
            options,
            &["waterui-preview-mode"],
        )
        .await?;

        // Asset mounts ship as the library layout's `waterui_assets`
        // directory, staged through the shared planner and read from there.
        let symbols = build.built.app_symbols()?;
        let lib_dir = root.join(LIBRARY_DIR);
        let stage_dir = root.join("stage");
        let (libraries, (_manifest, staged)) = futures_util::try_join!(
            stage_stripped_libraries(project, host, abi, &build, &lib_dir),
            assets::stage_project_assets_for_android_library(project, &stage_dir, &symbols, false),
        )?;
        Self::from_staged(&lib_dir, libraries, &staged.bundle).await
    }

    /// The payload of the libraries `libraries` in `lib_dir` plus the asset
    /// bundle at `assets_bundle`, hashed.
    ///
    /// # Errors
    /// Returns an error when the bundle cannot be walked or a file cannot be
    /// read.
    pub(super) async fn from_staged(
        lib_dir: &Path,
        libraries: Vec<String>,
        assets_bundle: &Path,
    ) -> Result<Self> {
        let lib_dir = lib_dir.to_path_buf();
        let assets_bundle = assets_bundle.to_path_buf();
        smol::unblock(move || {
            let mut entries: Vec<PayloadEntry> = libraries
                .iter()
                .map(|name| PayloadEntry {
                    path: format!("{LIBRARY_DIR}/{name}"),
                    source: lib_dir.join(name),
                })
                .collect();
            entries.extend(bundle_entries(&assets_bundle)?);
            entries.sort_by(|left, right| left.path.cmp(&right.path));
            let stamp = content_stamp(&entries)?;
            Ok(Self {
                entries,
                libraries,
                stamp,
            })
        })
        .await
    }

    /// The payload's content hash.
    pub(super) fn stamp(&self) -> &str {
        &self.stamp
    }

    /// The libraries' paths relative to the device run directory, in
    /// `System.load` order.
    pub(super) fn library_paths(&self) -> Vec<String> {
        self.libraries
            .iter()
            .map(|name| format!("{PAYLOAD_DIR}/{LIBRARY_DIR}/{name}"))
            .collect()
    }

    /// The asset bundle's path relative to the device run directory.
    pub(super) fn assets_root() -> String {
        format!(
            "{PAYLOAD_DIR}/{RESOURCES_DIR}/{}",
            assets::ANDROID_ASSET_BUNDLE_DIR
        )
    }

    /// Write the payload as a tar archive into `sink`, chunk by chunk: the
    /// directories, every file under [`PAYLOAD_DIR`], then the stamp as
    /// [`INCOMING_STAMP_FILE`]. The archive runs on a blocking thread; the
    /// bounded `sink` paces it to the consumer.
    ///
    /// # Errors
    /// Returns an error when a payload file cannot be read, or a
    /// `BrokenPipe` when the consumer dropped its end of `sink`.
    pub(super) async fn write_archive(
        &self,
        sink: async_channel::Sender<Vec<u8>>,
    ) -> io::Result<()> {
        let entries = self.entries.clone();
        let stamp = self.stamp.clone();
        smol::unblock(move || write_archive(&entries, &stamp, ChunkSink(sink))).await
    }
}

/// Bring the stripped-library cache in `lib_dir` up to date with the
/// build's artifact and answer the staged libraries in `System.load` order.
///
/// The cache is current when its [`StripRecord`] names the artifact's hash
/// and the build's NDK; otherwise `lib_dir` is rebuilt from scratch — the
/// record removed with it first, so an interrupted rebuild is never taken
/// for a current one — and the record written last.
async fn stage_stripped_libraries(
    project: &Project,
    host: &Host,
    abi: AndroidAbi,
    build: &HydrolysisAndroidBuild,
    lib_dir: &Path,
) -> Result<Vec<String>> {
    let artifact = build.built.artifact.clone();
    let ndk = build.context.ndk_path.clone();
    let source_sha256 = smol::unblock({
        let artifact = artifact.clone();
        move || -> io::Result<String> {
            let mut hasher = Sha256::new();
            hash_file(&mut hasher, &artifact)?;
            Ok(hex::encode(hasher.finalize()))
        }
    })
    .await
    .wrap_err_with(|| format!("failed to hash {}", artifact.display()))?;

    let record_path = lib_dir.join(STRIP_RECORD_FILE);
    if let Some(record) = read_strip_record(&record_path).await?
        && record.source_sha256 == source_sha256
        && record.ndk == ndk
    {
        return Ok(record.libraries);
    }

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
            artifact.as_os_str(),
        ],
    )
    .await
    .map_err(|error| eyre::eyre!("llvm-strip on {} failed: {error}", artifact.display()))?;
    let libraries =
        hydrolysis_android::stage_runtime_libraries(lib_dir, library_name, abi, &build.context)
            .await?;

    let record = serde_json::to_vec_pretty(&StripRecord {
        source_sha256,
        ndk,
        libraries: libraries.clone(),
    })
    .wrap_err("failed to serialize the strip record")?;
    fs::write(&record_path, &record)
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

/// Every regular file of the asset bundle as a payload entry under
/// `resources/<bundle dir>/`.
fn bundle_entries(bundle: &Path) -> io::Result<Vec<PayloadEntry>> {
    let mut entries = Vec::new();
    for entry in walkdir::WalkDir::new(bundle) {
        let entry = entry?;
        if entry.file_type().is_dir() {
            continue;
        }
        if !entry.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "the staged asset bundle holds {}, which is not a regular file",
                    entry.path().display()
                ),
            ));
        }
        let relative = entry
            .path()
            .strip_prefix(bundle)
            .map_err(io::Error::other)?;
        entries.push(PayloadEntry {
            path: format!(
                "{RESOURCES_DIR}/{}/{}",
                assets::ANDROID_ASSET_BUNDLE_DIR,
                slash_path(relative)?
            ),
            source: entry.into_path(),
        });
    }
    Ok(entries)
}

/// `relative` spelled with `/` separators, as the device and the archive
/// name it.
fn slash_path(relative: &Path) -> io::Result<String> {
    let parts = relative
        .components()
        .map(|component| match component {
            Component::Normal(part) => part.to_str().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} is not valid UTF-8", relative.display()),
                )
            }),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a plain relative path", relative.display()),
            )),
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok(parts.join("/"))
}

/// The payload's content hash: every entry's path, size and bytes, in path
/// order.
fn content_stamp(entries: &[PayloadEntry]) -> io::Result<String> {
    let mut hasher = Sha256::new();
    for entry in entries {
        hasher.update(entry.path.as_bytes());
        hasher.update([0]);
        hash_file(&mut hasher, &entry.source)?;
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Feed `path`'s size and bytes into `hasher`, streamed.
fn hash_file(hasher: &mut Sha256, path: &Path) -> io::Result<()> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?;
    hasher.update(file.metadata()?.len().to_le_bytes());
    let mut chunk = vec![0u8; IO_CHUNK];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            return Ok(());
        }
        hasher.update(&chunk[..read]);
    }
}

/// Write the archive [`DevicePayload::write_archive`] describes into `sink`.
fn write_archive(entries: &[PayloadEntry], stamp: &str, sink: impl Write) -> io::Result<()> {
    let mut builder = tar::Builder::new(BufWriter::with_capacity(IO_CHUNK, sink));

    // Parents sort ahead of their children, so every directory exists
    // before anything lands in it.
    let directories: BTreeSet<String> = entries
        .iter()
        .flat_map(|entry| {
            let path = format!("{PAYLOAD_DIR}/{}", entry.path);
            path.match_indices('/')
                .map(|(index, _)| path[..index].to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    for directory in directories {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_mode(0o755);
        header.set_size(0);
        header.set_mtime(0);
        builder.append_data(&mut header, &directory, io::empty())?;
    }

    for entry in entries {
        let file = std::fs::File::open(&entry.source).map_err(|error| {
            io::Error::new(error.kind(), format!("{}: {error}", entry.source.display()))
        })?;
        let size = file.metadata()?.len();
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        header.set_size(size);
        header.set_mtime(0);
        builder.append_data(
            &mut header,
            format!("{PAYLOAD_DIR}/{}", entry.path),
            file.take(size),
        )?;
    }

    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(0o644);
    header.set_size(stamp.len() as u64);
    header.set_mtime(0);
    builder.append_data(&mut header, INCOMING_STAMP_FILE, stamp.as_bytes())?;

    builder.into_inner()?.flush()
}

/// A blocking [`Write`] that hands each chunk to an async consumer through
/// a bounded channel; a dropped consumer reads as `BrokenPipe`.
struct ChunkSink(async_channel::Sender<Vec<u8>>);

impl Write for ChunkSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .send_blocking(buf.to_vec())
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
