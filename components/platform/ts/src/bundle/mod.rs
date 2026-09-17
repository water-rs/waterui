//! Loading the application's bundle at launch: the baseline the binary ships,
//! the requirement it was compiled with, and the loader that picks a bundle,
//! verifies it and evaluates it.
//!
//! # The two manifests
//!
//! The binary's side is the [`Requirement`]: the runtime fingerprint the
//! binary was built with and, for every module it mounts, the module id and
//! the props contract hash it was compiled against. The `water` CLI reads
//! all of it out of the compiled artifacts — `waterui_meta_tsx_*` for the
//! mounts, `waterui_meta_ts_runtime_*` for the fingerprint halves — and
//! writes a `const` [`Requirement`] into the leaf crate it generates. The
//! fingerprint it writes is [`RuntimeFingerprint`] built from those halves,
//! which is the same value the facade computes at compile time from the same
//! two constants, so `waterui::ts::RUNTIME_FINGERPRINT` is what the leaf
//! crate names.
//!
//! The bundle's side is the [`BundleManifest`] the CLI writes beside every
//! bundle it builds. The [`Baseline`] is the bundle built into the binary with
//! its manifest, embedded with `include_str!`; a downloaded bundle is the
//! same document plus a signature, and the `ota` feature's client is what
//! fetches, verifies and caches one.
//!
//! # What the loader does
//!
//! [`Loader::baseline_only`] verifies the baseline against the requirement
//! and evaluates it. A baseline that does not satisfy the requirement is a
//! build-pipeline bug and a hard [`LaunchError`], never a bundle to fall past:
//! the baseline is the floor. With the `ota` feature, `Loader::load` first
//! walks the cache, newest verified bundle first, and falls through to the
//! baseline when none is usable.
//!
//! What comes back is a [`Launched`]: the environment the application root
//! renders under, with the [`RuntimeHandle`] every `tsx!` mount finds and the
//! bundle's translation catalog installed, plus [`Launched::booted`], the call
//! the application makes once its first frame has rendered.

pub mod verify;

use waterui_core::Environment;
use waterui_locale::TranslationCatalog;
use waterui_ts_schema::{BundleManifest, RuntimeFingerprint};

use crate::error::TsError;
use crate::host::HostTable;
use crate::mount::RuntimeHandle;
use crate::runtime::TsRuntime;

pub use verify::Rejection;

/// One module the binary mounts, with the props contract it was compiled
/// against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequiredModule {
    id: &'static str,
    contract: u64,
}

impl RequiredModule {
    /// The module `id` — the `.tsx` path relative to the crate's manifest
    /// directory, forward-slashed, extension kept — mounted with the props
    /// type whose `TsProps::CONTRACT_HASH` is `contract`.
    #[must_use]
    pub const fn new(id: &'static str, contract: u64) -> Self {
        Self { id, contract }
    }

    /// The module id.
    #[must_use]
    pub const fn id(&self) -> &'static str {
        self.id
    }

    /// The props contract hash the binary mounts the module with.
    #[must_use]
    pub const fn contract(&self) -> u64 {
        self.contract
    }
}

/// What the binary requires of a bundle: the runtime it was built with and
/// every module it mounts.
///
/// Instantiated as a `const` by the CLI-generated leaf crate:
///
/// ```
/// use waterui_ts::{RequiredModule, Requirement};
/// use waterui_ts::schema::RuntimeFingerprint;
///
/// // The leaf crate names `waterui::ts::RUNTIME_FINGERPRINT` here.
/// const FINGERPRINT: RuntimeFingerprint = RuntimeFingerprint::new(0x1, 0x2);
///
/// const REQUIREMENT: Requirement = Requirement::new(
///     FINGERPRINT,
///     &[RequiredModule::new("src/views/promo.tsx", 0x0123_4567_89ab_cdef)],
/// );
/// assert_eq!(REQUIREMENT.modules().len(), 1);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Requirement {
    fingerprint: RuntimeFingerprint,
    modules: &'static [RequiredModule],
}

impl Requirement {
    /// The requirement of a binary with runtime `fingerprint` that mounts
    /// `modules`.
    #[must_use]
    pub const fn new(fingerprint: RuntimeFingerprint, modules: &'static [RequiredModule]) -> Self {
        Self {
            fingerprint,
            modules,
        }
    }

    /// The runtime fingerprint the binary was built with.
    #[must_use]
    pub const fn fingerprint(&self) -> RuntimeFingerprint {
        self.fingerprint
    }

    /// Every module the binary mounts.
    #[must_use]
    pub const fn modules(&self) -> &'static [RequiredModule] {
        self.modules
    }
}

/// The bundle built into the binary: the floor every launch can fall back to.
///
/// Both halves are embedded by the leaf crate the CLI generates, from the
/// files the bundle step wrote:
///
/// ```ignore
/// const BASELINE: Baseline = Baseline::new(
///     include_str!("bundle.js"),
///     include_str!("manifest.json"),
/// );
/// ```
///
/// The manifest is a [`BundleManifest`] — unsigned, because the binary's own
/// code signature already covers everything embedded in it — and the loader
/// verifies it against the [`Requirement`] like any other bundle's.
#[derive(Debug, Clone, Copy)]
pub struct Baseline {
    bundle: &'static str,
    manifest: &'static str,
}

impl Baseline {
    /// The baseline with source `bundle` and the JSON text of its manifest.
    #[must_use]
    pub const fn new(bundle: &'static str, manifest: &'static str) -> Self {
        Self { bundle, manifest }
    }

    /// The bundle's source.
    #[must_use]
    pub const fn bundle(&self) -> &'static str {
        self.bundle
    }

    /// The manifest's JSON text.
    #[must_use]
    pub const fn manifest(&self) -> &'static str {
        self.manifest
    }
}

/// Why the launch could not produce a runtime.
///
/// Every variant is a hard error: the JavaScript engine could not be created,
/// or the baseline — the one bundle that must always work — does not match
/// the binary it ships in or does not evaluate. A downloaded bundle failing
/// is never one of these; it is logged, recorded and fallen past.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LaunchError {
    /// The engine could not be created or a host function could not be
    /// registered.
    #[error("the JavaScript engine could not be created: {0}")]
    Engine(#[source] TsError),

    /// The baseline's manifest is not a manifest.
    #[error(
        "the baseline manifest embedded in this binary does not parse: {0}; the bundle step that \
         wrote it and the leaf crate that embedded it disagree"
    )]
    BaselineManifest(#[source] serde_json::Error),

    /// The baseline does not satisfy the binary's requirement: a
    /// build-pipeline bug, because both were produced from the same build.
    #[error("the baseline bundle embedded in this binary does not match it: {0}")]
    BaselineRejected(#[source] Rejection),

    /// The baseline threw while it evaluated.
    #[error("the baseline bundle embedded in this binary failed to evaluate: {0}")]
    BaselineFailed(#[source] TsError),

    /// The bundle store could not be read or written.
    #[cfg(feature = "ota")]
    #[error(transparent)]
    Store(#[from] crate::ota::StoreError),
}

/// Picks, verifies and evaluates the bundle a launch runs.
///
/// One per launch: the loader is consumed by the load, and there is no second
/// bundle in a running process — an update takes effect at the next launch.
#[must_use]
pub struct Loader<H> {
    requirement: Requirement,
    baseline: Baseline,
    table: H,
    environment: Environment,
}

impl<H> core::fmt::Debug for Loader<H> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Loader")
            .field("requirement", &self.requirement)
            .finish_non_exhaustive()
    }
}

/// The verified baseline: its manifest and the catalog it carries.
pub struct CheckedBaseline {
    pub manifest: BundleManifest,
    pub catalog: Option<TranslationCatalog>,
}

impl<H: HostTable + Clone> Loader<H> {
    /// A loader for a binary with `requirement`, shipping `baseline`, whose
    /// runtime installs `table` and whose modules see `environment`.
    ///
    /// The table is cloned for every bundle the loader evaluates, because a
    /// bundle that throws leaves its context behind and the next candidate
    /// gets a fresh one.
    pub const fn new(
        requirement: Requirement,
        baseline: Baseline,
        table: H,
        environment: Environment,
    ) -> Self {
        Self {
            requirement,
            baseline,
            table,
            environment,
        }
    }

    /// What the binary requires of a bundle.
    #[must_use]
    pub const fn requirement(&self) -> &Requirement {
        &self.requirement
    }

    /// Verifies the baseline and evaluates it.
    ///
    /// This is the whole launch of an application with no update source: no
    /// store is read, no file is touched, and nothing but the embedded bundle
    /// is considered. A build without the `ota` feature has no update client
    /// at all — the type does not exist — so such an application opens no
    /// connection by construction rather than by a check:
    ///
    #[cfg_attr(not(feature = "ota"), doc = "```compile_fail")]
    #[cfg_attr(feature = "ota", doc = "```ignore")]
    /// let _ = core::any::type_name::<waterui_ts::Ota>();
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`LaunchError`] when the engine cannot be created, or when the
    /// baseline does not parse, does not satisfy the requirement, or fails to
    /// evaluate.
    pub fn baseline_only(self) -> Result<Launched, LaunchError> {
        let checked = self.check_baseline()?;
        self.launch_baseline(checked)
    }

    /// Parses the baseline manifest and verifies it against the requirement.
    pub(crate) fn check_baseline(&self) -> Result<CheckedBaseline, LaunchError> {
        let manifest = BundleManifest::from_json(self.baseline.manifest)
            .map_err(LaunchError::BaselineManifest)?;
        let catalog = verify::requirement(&manifest, &self.requirement)
            .map_err(LaunchError::BaselineRejected)?;
        verify::bundle(self.baseline.bundle.as_bytes(), &manifest.bundle.sha256)
            .map_err(LaunchError::BaselineRejected)?;
        Ok(CheckedBaseline { manifest, catalog })
    }

    /// Evaluates the verified baseline.
    pub(crate) fn launch_baseline(
        &self,
        checked: CheckedBaseline,
    ) -> Result<Launched, LaunchError> {
        let prepared = self.prepare(checked.catalog)?;
        prepared
            .load(self.baseline.bundle)
            .map_err(LaunchError::BaselineFailed)?;
        tracing::debug!(
            version = checked.manifest.version,
            "launched the baseline bundle"
        );
        Ok(Launched::new(
            prepared,
            checked.manifest.version,
            self.requirement,
            checked.manifest.version,
        ))
    }

    /// A fresh runtime over the loader's environment, with `catalog`
    /// installed for the bundle's own modules when it carries one.
    pub(crate) fn prepare(
        &self,
        catalog: Option<TranslationCatalog>,
    ) -> Result<PreparedRuntime, LaunchError> {
        let environment = catalog.map_or_else(
            || self.environment.clone(),
            |catalog| self.environment.extending(catalog),
        );
        let runtime =
            TsRuntime::new(environment.clone(), self.table.clone()).map_err(LaunchError::Engine)?;
        Ok(PreparedRuntime {
            runtime,
            environment,
        })
    }
}

/// A runtime with its environment, before a bundle is loaded into it.
pub struct PreparedRuntime {
    runtime: TsRuntime,
    environment: Environment,
}

impl PreparedRuntime {
    /// Evaluates `bundle` into the runtime.
    ///
    /// # Errors
    ///
    /// Returns the [`TsError`] the bundle failed with.
    pub(crate) fn load(&self, bundle: &str) -> Result<(), TsError> {
        self.runtime.load(bundle)
    }
}

/// The record that a cached bundle is booting, cleared by
/// [`Launched::booted`].
#[cfg(feature = "ota")]
#[derive(Debug)]
pub struct BootRecord {
    pub store: crate::ota::BundleStore,
    pub version: u64,
}

/// The outcome of a launch: the runtime the chosen bundle is loaded into and
/// the environment the application root renders under.
///
/// Hold it until the first frame has rendered, then call
/// [`booted`](Self::booted); the runtime itself lives on in the environment
/// as a [`RuntimeHandle`] for as long as the application does.
#[derive(Debug)]
pub struct Launched {
    handle: RuntimeHandle,
    environment: Environment,
    version: u64,
    requirement: Requirement,
    baseline_version: u64,
    #[cfg(feature = "ota")]
    boot: Option<BootRecord>,
}

impl Launched {
    /// The baseline, or any launch that records nothing.
    fn new(
        prepared: PreparedRuntime,
        version: u64,
        requirement: Requirement,
        baseline_version: u64,
    ) -> Self {
        let handle = RuntimeHandle::new(prepared.runtime);
        let environment = handle.clone().install(&prepared.environment);
        Self {
            handle,
            environment,
            version,
            requirement,
            baseline_version,
            #[cfg(feature = "ota")]
            boot: None,
        }
    }

    /// A cached bundle, whose boot record is cleared by [`booted`](Self::booted).
    #[cfg(feature = "ota")]
    pub(crate) fn cached(
        prepared: PreparedRuntime,
        boot: BootRecord,
        requirement: Requirement,
        baseline_version: u64,
    ) -> Self {
        let mut launched = Self::new(prepared, boot.version, requirement, baseline_version);
        launched.boot = Some(boot);
        launched
    }

    /// What the binary requires of a bundle: what this launch verified
    /// against, and what a fetch verifies against.
    #[must_use]
    pub const fn requirement(&self) -> &Requirement {
        &self.requirement
    }

    /// The baseline's version: the floor a published bundle has to be newer
    /// than.
    #[must_use]
    pub const fn baseline_version(&self) -> u64 {
        self.baseline_version
    }

    /// The environment the application root renders under: the loader's
    /// environment with the runtime installed for every `tsx!` mount and, when
    /// the bundle carries one, its translation catalog.
    #[must_use]
    pub const fn environment(&self) -> &Environment {
        &self.environment
    }

    /// The loaded runtime.
    #[must_use]
    pub const fn handle(&self) -> &RuntimeHandle {
        &self.handle
    }

    /// The version of the bundle that launched.
    #[must_use]
    pub const fn version(&self) -> u64 {
        self.version
    }

    /// Whether the baseline launched rather than a cached bundle.
    #[must_use]
    pub const fn is_baseline(&self) -> bool {
        #[cfg(feature = "ota")]
        {
            self.boot.is_none()
        }
        #[cfg(not(feature = "ota"))]
        {
            true
        }
    }

    /// Records that the launched bundle booted: the application's root has
    /// rendered its first frame, so every module it mounts has run.
    ///
    /// The CLI-generated leaf crate makes this call after the first frame is
    /// presented, and not before: until then a `tsx!` mount may still fail,
    /// and the record this clears is what marks the bundle bad at the next
    /// launch if the process dies first. For the baseline there is nothing to
    /// record and the call does nothing.
    ///
    /// # Errors
    ///
    /// Returns `LaunchError::Store` when the record could not be written.
    #[cfg_attr(
        not(feature = "ota"),
        expect(
            clippy::missing_const_for_fn,
            reason = "with `ota` on this writes the store; the signature is the same in both \
                      builds so the leaf crate's call does not change with the feature"
        )
    )]
    pub fn booted(&self) -> Result<(), LaunchError> {
        #[cfg(feature = "ota")]
        if let Some(boot) = &self.boot {
            boot.store.clear_booting(boot.version)?;
            tracing::debug!(version = boot.version, "the cached bundle booted");
        }
        Ok(())
    }
}
