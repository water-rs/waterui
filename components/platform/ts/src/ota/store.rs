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
//! the process died in — and the version currently booting.
//!
//! The root is the application's cache directory by default
//! ([`BundleStore::in_cache_dir`]), which on every platform `waterkit-fs`
//! reaches is app-private and regenerable: a purge costs one re-download and
//! nothing else, because the baseline is the floor. `documents_dir` is
//! user-visible on iOS and Android and `data_local_path` exists only on
//! desktop, so neither is the cache's home.

use std::collections::BTreeSet;
use std::fs;
use std::io;
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

/// Why the store could not be read or written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The platform exposes no cache directory, or the state file could not
    /// be read or written through `waterkit-fs`.
    #[error(transparent)]
    Fs(#[from] waterkit_fs::FsError),

    /// A bundle file or directory could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The failure.
        #[source]
        source: io::Error,
    },
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

/// One cached bundle, read whole.
pub struct Cached {
    /// The signed manifest's JSON text as it was written.
    pub manifest: String,
    /// The bundle file's bytes.
    pub bundle: Vec<u8>,
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
        self.bundles_dir().join(format!(".{version}.staging"))
    }

    pub(crate) fn load_state(&self) -> Result<State, StoreError> {
        Ok(WaterFs::load_json_store(&self.state_path())?)
    }

    pub(crate) fn save_state(&self, state: &State) -> Result<(), StoreError> {
        Ok(WaterFs::write_json_store(&self.state_path(), state)?)
    }

    /// Records that `version` is booting.
    pub(crate) fn set_booting(&self, version: u64) -> Result<(), StoreError> {
        let mut state = self.load_state()?;
        state.booting = Some(version);
        self.save_state(&state)
    }

    /// Clears the boot record for `version`, if it is the one recorded.
    pub(crate) fn clear_booting(&self, version: u64) -> Result<(), StoreError> {
        let mut state = self.load_state()?;
        if state.booting == Some(version) {
            state.booting = None;
            self.save_state(&state)?;
        }
        Ok(())
    }

    /// Every cached version, newest first.
    ///
    /// An entry under `bundles/` that is not a version directory is not the
    /// store's — it is logged and left alone.
    pub(crate) fn versions(&self) -> Result<Vec<u64>, StoreError> {
        let dir = self.bundles_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(StoreError::Io { path: dir, source }),
        };
        let mut versions = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| StoreError::Io {
                path: dir.clone(),
                source,
            })?;
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

    /// Whether `version` is cached.
    pub(crate) fn has(&self, version: u64) -> bool {
        self.version_dir(version).join(MANIFEST).is_file()
    }

    /// Reads the cached `version`.
    pub(crate) fn read(&self, version: u64) -> Result<Cached, StoreError> {
        let dir = self.version_dir(version);
        let manifest_path = dir.join(MANIFEST);
        let manifest = fs::read_to_string(&manifest_path).map_err(|source| StoreError::Io {
            path: manifest_path,
            source,
        })?;
        let bundle_path = dir.join(BUNDLE);
        let bundle = fs::read(&bundle_path).map_err(|source| StoreError::Io {
            path: bundle_path,
            source,
        })?;
        Ok(Cached { manifest, bundle })
    }

    /// Writes `version` whole: both files into a staging directory, then one
    /// rename into place.
    pub(crate) fn write(
        &self,
        version: u64,
        manifest: &str,
        bundle: &[u8],
    ) -> Result<(), StoreError> {
        let staging = self.staging_dir(version);
        let io = |path: PathBuf| move |source| StoreError::Io { path, source };
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(io(staging.clone()))?;
        }
        fs::create_dir_all(&staging).map_err(io(staging.clone()))?;
        let manifest_path = staging.join(MANIFEST);
        fs::write(&manifest_path, manifest).map_err(io(manifest_path))?;
        let bundle_path = staging.join(BUNDLE);
        fs::write(&bundle_path, bundle).map_err(io(bundle_path))?;
        let target = self.version_dir(version);
        fs::rename(&staging, &target).map_err(io(target))
    }

    /// Removes `version` from the cache, if it is there.
    pub(crate) fn remove(&self, version: u64) -> Result<(), StoreError> {
        let dir = self.version_dir(version);
        match fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StoreError::Io { path: dir, source }),
        }
    }
}
