//! Where verified bundles live between launches.
//!
//! ```text
//! <root>/
//!   state.json              { "bad": [4], "booting": 5 }
//!   bundles/
//!     5/
//!       bundle.js
//!       manifest.json       the SignedManifest as downloaded
//! ```
//!
//! One directory per bundle version, written whole: the files go into a
//! staging directory first and the directory is renamed into place, so a
//! version directory that exists is a complete one. The state file records
//! the versions that failed — verification at launch, evaluation, or a mount
//! the process died in — and the version currently booting. It is written
//! the same way, to a sibling file renamed over it, so a launch never finds
//! half a state file; one it cannot read anyway is replaced by the empty
//! state, because the store is a cache and nothing in it is the only copy.
//!
//! Every method here reports its own I/O honestly. What a failure means is
//! the caller's decision: the loader treats every one as "skip this entry"
//! and the fetch as "this fetch did not cache".
//!
//! The root is the application's cache directory by default
//! ([`BundleStore::in_cache_dir`]), which on every platform `waterkit-fs`
//! reaches is app-private and regenerable: a purge costs one re-download and
//! nothing else, because the baseline is the floor. `documents_dir` is
//! user-visible on iOS and Android and `data_local_path` exists only on
//! desktop, so neither is the cache's home.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};

use waterkit_fs::WaterFs;

/// The state file's name.
const STATE: &str = "state.json";
/// The directory holding one subdirectory per cached version.
const BUNDLES: &str = "bundles";
/// The bundle file inside a version directory.
const BUNDLE: &str = "bundle.js";
/// The manifest file inside a version directory.
const MANIFEST: &str = "manifest.json";
/// The subdirectory under the platform cache directory.
const CACHE_SUBDIR: &str = "waterui-ts";
/// The suffix of a staging directory: `.<version>.staging`, renamed to
/// `<version>` once both files are in it.
const STAGING_SUFFIX: &str = ".staging";

/// Why the store could not be read or written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The platform exposes no cache directory.
    #[error(transparent)]
    Fs(#[from] waterkit_fs::FsError),

    /// A file or directory could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The failure.
        #[source]
        source: io::Error,
    },

    /// A cached bundle file is longer than its manifest says, so it is not
    /// the file the manifest was published with and is not read.
    #[error("{path} is {len} bytes, and its manifest declares {declared}")]
    BundleSize {
        /// The bundle file.
        path: PathBuf,
        /// The file's length on disk.
        len: u64,
        /// The length its manifest declares.
        declared: u64,
    },
}

impl StoreError {
    fn io(path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io { path, source }
    }
}

/// What the store remembers between launches.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct State {
    /// Versions that failed and are never tried again.
    pub bad: BTreeSet<u64>,
    /// The cached version that started booting and has not yet reported
    /// that it booted.
    pub booting: Option<u64>,
}

/// The on-disk cache of verified bundles and the record of which ones
/// failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleStore {
    root: PathBuf,
}

impl BundleStore {
    /// A store rooted at `root`, which is created on first write.
    ///
    /// This is the general constructor; an application uses
    /// [`in_cache_dir`](Self::in_cache_dir).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// A store under the application's cache directory, in a subdirectory
    /// named by `namespace` — the application's bundle identifier, so
    /// applications sharing a cache directory on desktop platforms do not
    /// share bundles.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Fs`] when the platform exposes no cache
    /// directory.
    pub fn in_cache_dir(namespace: &str) -> Result<Self, StoreError> {
        let root = WaterFs::cache_dir()?.join(namespace).join(CACHE_SUBDIR);
        Ok(Self::new(root))
    }

    /// The directory the store lives in.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn state_path(&self) -> PathBuf {
        self.root.join(STATE)
    }

    fn bundles_dir(&self) -> PathBuf {
        self.root.join(BUNDLES)
    }

    fn version_dir(&self, version: u64) -> PathBuf {
        self.bundles_dir().join(version.to_string())
    }

    /// The staging directory a version is written into before it is renamed
    /// into place.
    fn staging_dir(&self, version: u64) -> PathBuf {
        self.bundles_dir()
            .join(format!(".{version}{STAGING_SUFFIX}"))
    }

    /// The state, or the empty state when there is none.
    ///
    /// A state file that cannot be read or does not parse is the empty state
    /// too: it is logged at warn, naming the file and the reason, and
    /// replaced on disk by the empty state so the next launch does not find
    /// it again. Nothing in it is the only copy of anything — a bad mark is
    /// re-learned the next time the version fails — so this never fails.
    pub(crate) fn load_state(&self) -> State {
        let path = self.state_path();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return State::default(),
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    %error,
                    "the bundle store's state file could not be read; starting from no state"
                );
                return self.reset_state();
            }
        };
        match serde_json::from_str(&text) {
            Ok(state) => state,
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    %error,
                    "the bundle store's state file does not parse; starting from no state"
                );
                self.reset_state()
            }
        }
    }

    /// Writes the empty state over whatever the state file holds, and hands
    /// the empty state back whether or not the write succeeded.
    fn reset_state(&self) -> State {
        let state = State::default();
        if let Err(error) = self.save_state(&state) {
            tracing::warn!(%error, "the bundle store's state file could not be replaced");
        }
        state
    }

    /// Writes the state file whole: to a sibling, then one rename over it.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] naming the path that failed.
    pub(crate) fn save_state(&self, state: &State) -> Result<(), StoreError> {
        write_json_atomically(&self.state_path(), state)
    }

    /// Clears the boot record for `version`, if it is the one recorded.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] when the state file cannot be written.
    pub(crate) fn clear_booting(&self, version: u64) -> Result<(), StoreError> {
        let mut state = self.load_state();
        if state.booting == Some(version) {
            state.booting = None;
            self.save_state(&state)?;
        }
        Ok(())
    }

    /// Every cached version, newest first.
    ///
    /// An entry under `bundles/` whose name is not a version is not the
    /// store's — it is logged and left alone. A version-named entry that is
    /// not a directory is listed: reading it fails, and the reader removes it.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] when the bundles directory exists but
    /// cannot be listed.
    pub(crate) fn versions(&self) -> Result<Vec<u64>, StoreError> {
        let dir = self.bundles_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(StoreError::io(dir)(error)),
        };
        let mut versions = Vec::new();
        for entry in entries {
            let entry = entry.map_err(StoreError::io(dir.clone()))?;
            let name = entry.file_name();
            if let Some(version) = name.to_str().and_then(|name| name.parse::<u64>().ok()) {
                versions.push(version);
            } else {
                tracing::debug!(
                    entry = %entry.path().display(),
                    "an entry under the bundle store is not a version directory"
                );
            }
        }
        versions.sort_unstable_by(|left, right| right.cmp(left));
        Ok(versions)
    }

    /// Removes every staging directory left under `bundles/` by a write that
    /// died between filling one and renaming it into place.
    ///
    /// Each one reaped is logged at debug; one that cannot be listed or
    /// removed is logged at warn and left for the next launch. Nothing here
    /// fails: a stale staging directory is never read, only ever replaced by
    /// the next write of its version.
    pub(crate) fn reap_staging(&self) {
        let dir = self.bundles_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return,
            Err(error) => {
                tracing::warn!(
                    path = %dir.display(),
                    %error,
                    "the bundle store's bundles directory could not be listed for staging leftovers"
                );
                return;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    tracing::warn!(
                        path = %dir.display(),
                        %error,
                        "an entry of the bundle store's bundles directory could not be listed"
                    );
                    continue;
                }
            };
            let name = entry.file_name();
            let is_staging = name
                .to_str()
                .is_some_and(|name| name.starts_with('.') && name.ends_with(STAGING_SUFFIX));
            if !is_staging {
                continue;
            }
            let path = entry.path();
            match fs::remove_dir_all(&path) {
                Ok(()) => tracing::debug!(
                    path = %path.display(),
                    "removed a staging directory a previous write left behind"
                ),
                Err(error) => tracing::warn!(
                    path = %path.display(),
                    %error,
                    "a staging directory a previous write left behind could not be removed"
                ),
            }
        }
    }

    /// Whether `version` is cached.
    pub(crate) fn has(&self, version: u64) -> bool {
        self.version_dir(version).join(MANIFEST).is_file()
    }

    /// Reads the cached manifest of `version` as it was written.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] when the file cannot be read.
    pub(crate) fn read_manifest(&self, version: u64) -> Result<String, StoreError> {
        let path = self.version_dir(version).join(MANIFEST);
        fs::read_to_string(&path).map_err(StoreError::io(path))
    }

    /// Reads the cached bundle file of `version`, which its manifest declares
    /// to be `declared` bytes long.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::BundleSize`] when the file is longer than
    /// `declared` — it is read only once its length is known to be within the
    /// bound — and [`StoreError::Io`] when it cannot be read.
    pub(crate) fn read_bundle(&self, version: u64, declared: u64) -> Result<Vec<u8>, StoreError> {
        let path = self.version_dir(version).join(BUNDLE);
        let file = fs::File::open(&path).map_err(StoreError::io(path.clone()))?;
        let len = file.metadata().map_err(StoreError::io(path.clone()))?.len();
        if len > declared {
            return Err(StoreError::BundleSize {
                path,
                len,
                declared,
            });
        }
        // The read is bounded by the declaration too, not by the length just
        // measured: a file that grows between the two is refused, never
        // buffered whole.
        let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(usize::MAX));
        let read = (&file)
            .take(declared + 1)
            .read_to_end(&mut bytes)
            .map_err(StoreError::io(path.clone()))?;
        let read = u64::try_from(read).expect("a byte count fits a u64");
        if read > declared {
            return Err(StoreError::BundleSize {
                path,
                len: read,
                declared,
            });
        }
        Ok(bytes)
    }

    /// Writes `version` whole: both files into a staging directory, then one
    /// rename into place.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] naming the path that failed.
    pub(crate) fn write(
        &self,
        version: u64,
        manifest: &str,
        bundle: &[u8],
    ) -> Result<(), StoreError> {
        let staging = self.staging_dir(version);
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(StoreError::io(staging.clone()))?;
        }
        fs::create_dir_all(&staging).map_err(StoreError::io(staging.clone()))?;
        let manifest_path = staging.join(MANIFEST);
        fs::write(&manifest_path, manifest).map_err(StoreError::io(manifest_path))?;
        let bundle_path = staging.join(BUNDLE);
        fs::write(&bundle_path, bundle).map_err(StoreError::io(bundle_path))?;
        let target = self.version_dir(version);
        fs::rename(&staging, &target).map_err(StoreError::io(target))
    }

    /// Removes whatever is at `version`'s path in the cache, if anything: the
    /// version directory, or a stray file that would keep the version from
    /// ever being written.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Io`] when the entry exists and cannot be removed.
    pub(crate) fn remove(&self, version: u64) -> Result<(), StoreError> {
        let path = self.version_dir(version);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(StoreError::io(path)(error)),
        };
        let removed = if metadata.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        removed.map_err(StoreError::io(path))
    }
}

/// Writes `value` as JSON to `path` whole: to a sibling temporary file that
/// is flushed to disk, then one rename over `path`, so a reader never sees a
/// partial document.
fn write_json_atomically<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), StoreError> {
    let parent = path
        .parent()
        .expect("a store file sits inside the store's root directory");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("a store file has a name");
    fs::create_dir_all(parent).map_err(StoreError::io(parent))?;
    let temporary = parent.join(format!(".{name}.tmp"));
    let text = serde_json::to_vec_pretty(value)
        .expect("the store's state serializes: every field is a plain value");
    let mut file = fs::File::create(&temporary).map_err(StoreError::io(temporary.clone()))?;
    file.write_all(&text)
        .and_then(|()| file.sync_all())
        .map_err(StoreError::io(temporary.clone()))?;
    drop(file);
    fs::rename(&temporary, path).map_err(StoreError::io(path))?;
    // The rename is durable only once the directory that records it is: a
    // power loss before that would show the old document, never a torn one,
    // but it would also lose a write this call reported as done.
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(StoreError::io(parent))
}
