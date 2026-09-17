//! The over-the-air client: cached bundles at launch, and the fetch that
//! fills the cache for the next one.
//!
//! [`Ota`] is the configuration the leaf crate builds — the public key it
//! embeds, the one manifest URL it is given, and the [`BundleStore`] —
//! and the two things it is used for are [`Loader::load`], which walks the
//! store at launch, and [`Ota::fetch`], which runs after launch on the local
//! executor and only ever writes to the store. A running process never
//! switches bundles: a fetch that succeeds is picked up by the next launch.
//!
//! # Selection at launch
//!
//! 1. A boot record left set means the previous launch died between
//!    evaluating that version and reporting that it booted — in a `tsx!`
//!    mount, typically. The version is marked bad and removed.
//! 2. Every cached version at or below the baseline's is stale and removed;
//!    bad-list entries at or below it are forgotten, which bounds the list.
//! 3. The remaining versions, newest first, skipping the bad ones: read,
//!    signature, digest, requirement. A version that fails is removed — it is
//!    either a bundle for a binary this no longer is, or a cache that was
//!    altered — and the walk continues. A version that verifies is recorded
//!    as booting and evaluated; one that throws is marked bad and removed,
//!    and the walk continues. One that evaluates is the launch.
//! 4. Nothing usable: the baseline.
//!
//! # What the fetch does
//!
//! Manifest first, bundle second, and nothing is written until both have
//! passed every check: signature, then version (newer than the baseline, not
//! bad, not already cached), then the requirement, then the bundle's digest.
//! A network failure is not an error — the application is running on
//! whatever it launched with — and is logged at debug; a rejection is logged
//! at warn with its reason. The client fetches the application's own view
//! bundle and nothing else.

mod store;

use ed25519_dalek::VerifyingKey;
use url::Url;
use waterui_ts_schema::SignedManifest;
use zenwave::Client as _;

use crate::bundle::{BootRecord, LaunchError, Launched, Loader, Rejection, Requirement, verify};
use crate::host::HostTable;

pub use store::{BundleStore, StoreError};

/// The over-the-air configuration the leaf crate embedded is not usable: a
/// build-pipeline bug, since `water ota keygen` wrote the key and `Water.toml`
/// the URL.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OtaError {
    /// The embedded public key is not an ed25519 public key.
    #[error("the embedded public key is not a valid ed25519 public key: {0}")]
    PublicKey(#[source] ed25519_dalek::SignatureError),

    /// The manifest URL does not parse as an absolute URL.
    #[error("the manifest URL {url:?} is not an absolute URL: {source}")]
    ManifestUrl {
        /// The URL as configured.
        url: String,
        /// The parser's reason.
        #[source]
        source: url::ParseError,
    },
}

/// Why a fetch did not cache a bundle.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FetchError {
    /// The request did not complete. Logged at debug and not treated as a
    /// failure of anything: the application runs on what it launched with.
    #[error("the request to {url} did not complete: {source}")]
    Network {
        /// The URL that was requested.
        url: String,
        /// The client's reason, boxed because it is large and this is the
        /// exceptional path.
        #[source]
        source: Box<zenwave::Error>,
    },

    /// The server answered with something other than success.
    #[error("the request to {url} was answered with status {status}")]
    Status {
        /// The URL that was requested.
        url: String,
        /// The HTTP status code.
        status: u16,
    },

    /// The manifest is not UTF-8 text.
    #[error("the manifest at {url} is not UTF-8 text: {source}")]
    ManifestEncoding {
        /// The manifest's URL.
        url: String,
        /// The decoder's reason.
        #[source]
        source: std::string::FromUtf8Error,
    },

    /// The manifest is not a signed manifest.
    #[error("the manifest at {url} does not parse: {source}")]
    Manifest {
        /// The manifest's URL.
        url: String,
        /// The parser's reason.
        #[source]
        source: serde_json::Error,
    },

    /// The manifest names a bundle URL that does not resolve against the
    /// manifest's.
    #[error("the bundle URL {relative:?} does not resolve against {base}: {source}")]
    BundleUrl {
        /// The URL as the manifest spells it.
        relative: String,
        /// The manifest's URL.
        base: String,
        /// The parser's reason.
        #[source]
        source: url::ParseError,
    },

    /// The manifest or the bundle failed verification.
    #[error("the bundle was rejected: {0}")]
    Rejected(#[source] Rejection),

    /// The store could not be read or written.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// What a fetch that completed found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    /// The published version is not newer than the baseline, so it would
    /// never be chosen over it.
    NotNewer {
        /// The published version.
        version: u64,
        /// The baseline's version.
        baseline: u64,
    },
    /// The published version failed on an earlier launch and is never tried
    /// again.
    KnownBad {
        /// The published version.
        version: u64,
    },
    /// The published version is already in the cache.
    AlreadyCached {
        /// The published version.
        version: u64,
    },
    /// The published version verified and was written to the cache for the
    /// next launch.
    Cached {
        /// The published version.
        version: u64,
    },
}

/// The over-the-air configuration: where updates come from, which key signs
/// them, and where verified ones are kept.
///
/// Constructing one is what makes an application an updating one; the leaf
/// crate builds it only when a manifest URL was configured, and a build
/// without the `ota` feature has no such type at all.
#[derive(Debug, Clone)]
pub struct Ota {
    key: VerifyingKey,
    manifest_url: Url,
    store: BundleStore,
}

impl Ota {
    /// Updates published at `manifest_url`, signed by the holder of the
    /// private half of `public_key`, cached in `store`.
    ///
    /// # Errors
    ///
    /// Returns [`OtaError`] when the bytes are not an ed25519 public key or
    /// the URL is not an absolute URL.
    pub fn new(
        public_key: [u8; 32],
        manifest_url: &str,
        store: BundleStore,
    ) -> Result<Self, OtaError> {
        let key = VerifyingKey::from_bytes(&public_key).map_err(OtaError::PublicKey)?;
        let manifest_url = Url::parse(manifest_url).map_err(|source| OtaError::ManifestUrl {
            url: manifest_url.to_owned(),
            source,
        })?;
        Ok(Self {
            key,
            manifest_url,
            store,
        })
    }

    /// The manifest URL.
    #[must_use]
    pub fn manifest_url(&self) -> &str {
        self.manifest_url.as_str()
    }

    /// The store verified bundles are kept in.
    #[must_use]
    pub const fn store(&self) -> &BundleStore {
        &self.store
    }

    /// Fetches the published manifest and, when it names a bundle this
    /// binary can use and does not already have, the bundle, and caches
    /// both for the next launch.
    ///
    /// `requirement` is what the bundle is verified against and `baseline`
    /// the version it has to be newer than — both from the [`Loader`] this
    /// launch used, which is why [`Launched::spawn_update`] is the usual
    /// caller.
    ///
    /// # Errors
    ///
    /// Returns [`FetchError`] when the request did not complete, the
    /// manifest or the bundle was rejected, or the store could not be
    /// written. Nothing is written before every check has passed.
    #[expect(
        clippy::future_not_send,
        reason = "the fetch runs on the main-thread local executor beside the runtime it feeds; \
                  it is never sent anywhere"
    )]
    pub async fn fetch(
        &self,
        requirement: &Requirement,
        baseline: u64,
    ) -> Result<Outcome, FetchError> {
        let text = fetch_bytes(&self.manifest_url).await?;
        let text = String::from_utf8(text).map_err(|source| FetchError::ManifestEncoding {
            url: self.manifest_url.to_string(),
            source,
        })?;
        let signed = SignedManifest::from_json(&text).map_err(|source| FetchError::Manifest {
            url: self.manifest_url.to_string(),
            source,
        })?;
        verify::signature(&signed, &self.key).map_err(FetchError::Rejected)?;

        let version = signed.manifest.version;
        if version <= baseline {
            return Ok(Outcome::NotNewer { version, baseline });
        }
        let store = self.store.clone();
        let state = blocking::unblock(move || store.load_state()).await?;
        if state.bad.contains(&version) {
            return Ok(Outcome::KnownBad { version });
        }
        if self.store.has(version) {
            return Ok(Outcome::AlreadyCached { version });
        }
        verify::requirement(&signed.manifest, requirement).map_err(FetchError::Rejected)?;

        let bundle_url = self
            .manifest_url
            .join(&signed.manifest.bundle.url)
            .map_err(|source| FetchError::BundleUrl {
                relative: signed.manifest.bundle.url.clone(),
                base: self.manifest_url.to_string(),
                source,
            })?;
        let bytes = fetch_bytes(&bundle_url).await?;
        verify::bundle(&bytes, &signed.manifest.bundle.sha256).map_err(FetchError::Rejected)?;

        let store = self.store.clone();
        let manifest = signed.to_json();
        blocking::unblock(move || store.write(version, &manifest, &bytes)).await?;
        Ok(Outcome::Cached { version })
    }
}

/// One GET, the whole body.
#[expect(
    clippy::future_not_send,
    reason = "the request builder borrows the client, which the platform backend pins to the \
              calling thread; the fetch runs on the main-thread local executor"
)]
async fn fetch_bytes(url: &Url) -> Result<Vec<u8>, FetchError> {
    let network = |source: zenwave::Error| FetchError::Network {
        url: url.to_string(),
        source: Box::new(source),
    };
    let mut client = zenwave::client();
    let response = client
        .get(url.as_str())
        .map_err(network)?
        .await
        .map_err(network)?;
    let status = response.status();
    if !status.is_success() {
        return Err(FetchError::Status {
            url: url.to_string(),
            status: status.as_u16(),
        });
    }
    let bytes = response
        .into_body()
        .into_bytes()
        .await
        .map_err(|error| network(error.into()))?;
    Ok(bytes.to_vec())
}

impl<H: HostTable + Clone> Loader<H> {
    /// Picks the newest verified cached bundle that has not failed, falling
    /// through to the baseline, and evaluates it.
    ///
    /// The selection is spelled out under "Selection at launch" in
    /// `OTA.md`. Only the store is read; no connection is opened.
    ///
    /// # Errors
    ///
    /// Returns [`LaunchError`] for the same hard failures as
    /// [`baseline_only`](Self::baseline_only), and for a store that cannot
    /// be read or written. A cached bundle that fails is never one of them.
    pub fn load(self, ota: &Ota) -> Result<Launched, LaunchError> {
        let store = &ota.store;
        let mut state = store.load_state()?;
        if let Some(version) = state.booting.take() {
            tracing::warn!(
                version,
                "the previous launch did not report that this bundle booted; marking it bad"
            );
            state.bad.insert(version);
            store.save_state(&state)?;
            store.remove(version)?;
        }

        let baseline = self.check_baseline()?;
        let floor = baseline.manifest.version;
        let before = state.bad.len();
        state.bad.retain(|version| *version > floor);
        if state.bad.len() != before {
            store.save_state(&state)?;
        }

        for version in store.versions()? {
            if version <= floor {
                tracing::debug!(
                    version,
                    floor,
                    "removing a cached bundle the baseline supersedes"
                );
                store.remove(version)?;
                continue;
            }
            if state.bad.contains(&version) {
                tracing::debug!(version, "skipping a bundle marked bad");
                continue;
            }
            let (bundle, catalog) = match self.verify_cached(ota, version) {
                Ok(verified) => verified,
                Err(error) => {
                    tracing::warn!(version, %error, "removing a cached bundle that no longer verifies");
                    store.remove(version)?;
                    continue;
                }
            };
            let prepared = self.prepare(catalog)?;
            store.set_booting(version)?;
            match prepared.load(&bundle) {
                Ok(()) => {
                    tracing::debug!(version, "launched a cached bundle");
                    return Ok(Launched::cached(
                        prepared,
                        BootRecord {
                            store: store.clone(),
                            version,
                        },
                        *self.requirement(),
                        floor,
                    ));
                }
                Err(error) => {
                    tracing::warn!(version, %error, "a cached bundle failed to evaluate; marking it bad");
                    let mut state = store.load_state()?;
                    state.booting = None;
                    state.bad.insert(version);
                    store.save_state(&state)?;
                    store.remove(version)?;
                }
            }
        }

        self.launch_baseline(baseline)
    }

    /// Reads and verifies cached `version`: signature, digest, requirement.
    fn verify_cached(
        &self,
        ota: &Ota,
        version: u64,
    ) -> Result<(String, Option<waterui_locale::TranslationCatalog>), CachedError> {
        let cached = ota.store.read(version)?;
        let signed = SignedManifest::from_json(&cached.manifest)?;
        if signed.manifest.version != version {
            return Err(CachedError::Version {
                declared: signed.manifest.version,
            });
        }
        verify::signature(&signed, &ota.key)?;
        let source = verify::bundle(&cached.bundle, &signed.manifest.bundle.sha256)?;
        let catalog = verify::requirement(&signed.manifest, self.requirement())?;
        Ok((source.to_owned(), catalog))
    }
}

/// Why a cached version could not be used, for the log line that says so.
#[derive(Debug, thiserror::Error)]
enum CachedError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("the cached manifest does not parse: {0}")]
    Manifest(#[from] serde_json::Error),
    #[error("the cached manifest declares version {declared}, not the version it is cached as")]
    Version { declared: u64 },
    #[error(transparent)]
    Rejected(#[from] Rejection),
}

impl Launched {
    /// Fetches the published update on the local executor, after this
    /// launch, and caches it for the next one.
    ///
    /// The task is detached: it runs while the application does, never
    /// delays a frame, and reports what it found through `tracing` — a
    /// cached bundle or a reason it was not at debug, a rejected one at warn.
    /// The local executor must be installed, which every `WaterUI` host has
    /// done before the application root is built.
    ///
    /// The fetch verifies against the requirement this launch used and
    /// treats this launch's baseline version as the floor.
    pub fn spawn_update(&self, ota: Ota) {
        let baseline = self.baseline_version();
        let requirement = *self.requirement();
        executor_core::spawn_local(async move {
            match ota.fetch(&requirement, baseline).await {
                Ok(outcome) => tracing::debug!(?outcome, "the update fetch completed"),
                Err(error @ FetchError::Network { .. }) => {
                    tracing::debug!(%error, "the update fetch did not complete");
                }
                Err(error) => tracing::warn!(%error, "the update fetch failed"),
            }
        })
        .detach();
    }
}
