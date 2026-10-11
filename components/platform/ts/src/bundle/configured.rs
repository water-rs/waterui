//! The bundle a test or preview host loads: named by one environment variable,
//! set by the `water` CLI.
//!
//! An application's bundle loader owns the bundle it ships with; a host that
//! mounts a view outside an application — the `waterui-testing` session a
//! `#[waterui::test]` runs in, the support app `water preview` renders in —
//! has no loader, so the CLI hands it the bundle it just built through
//! [`BUNDLE_VARIABLE`], and the host installs the runtime that
//! [`configured_runtime`] loads from it. Both hosts call the one function
//! here rather than reading the variable themselves, so the path rule and the
//! failure messages are written once, beside the [`RuntimeHandle`] the
//! runtime is installed as and the error a mount without one raises.
//!
//! # The path rule
//!
//! The variable names the bundle file. An absolute path is used as it is. A
//! relative path is resolved against the directory `CARGO_MANIFEST_DIR` names
//! in the process environment — cargo and nextest set it for every test
//! binary they run, to the manifest directory of the package under test — and
//! is an error when that variable is unset, so a host that is not a cargo
//! test process, such as the preview support app, is given an absolute path.
//!
//! When the variable is unset there is no runtime, and nothing fails until a
//! view reaches a [`Mount`](crate::Mount): a test that mounts no TypeScript
//! does not need a bundle. The mount then fails with
//! [`TsError::NoRuntimeInstalled`], whose message names the variable and the
//! command that sets it.

use std::path::{Path, PathBuf};

use waterui_core::Environment;

use crate::error::TsError;
use crate::host::HostTable;
use crate::mount::RuntimeHandle;
use crate::runtime::TsRuntime;

/// The environment variable that names the bundle a test or preview host
/// loads.
///
/// `water test` sets it to the bundle it built for the crate before running
/// `cargo nextest`; `water preview` sets it for the preview support app.
pub const BUNDLE_VARIABLE: &str = "WATERUI_TS_BUNDLE";

/// The variable a relative bundle path is resolved against.
const MANIFEST_DIR_VARIABLE: &str = "CARGO_MANIFEST_DIR";

/// Why the bundle [`BUNDLE_VARIABLE`] names could not be loaded.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BundleError {
    /// The variable is set to nothing.
    #[error("{variable} is set but empty; it names the bundle file to load", variable = BUNDLE_VARIABLE)]
    Empty,

    /// The path is relative and there is no manifest directory to resolve it
    /// against.
    #[error(
        "{variable} is the relative path {path:?}, and {manifest_dir} is not set to resolve it \
         against: cargo and nextest set it for a test binary, and every other host is given an \
         absolute path",
        variable = BUNDLE_VARIABLE,
        manifest_dir = MANIFEST_DIR_VARIABLE
    )]
    Relative {
        /// The path as the variable spells it.
        path: PathBuf,
    },

    /// The bundle file could not be read.
    #[error(
        "the bundle {variable} names, {path:?}, could not be read: {source}",
        variable = BUNDLE_VARIABLE
    )]
    Read {
        /// The resolved path.
        path: PathBuf,
        /// What the read failed with.
        #[source]
        source: std::io::Error,
    },

    /// The runtime could not be created, or the bundle could not be loaded
    /// into it.
    #[error(
        "the bundle {variable} names, {path:?}, could not be loaded: {source}",
        variable = BUNDLE_VARIABLE
    )]
    Load {
        /// The resolved path.
        path: PathBuf,
        /// What the runtime refused it with.
        #[source]
        source: TsError,
    },
}

/// The runtime the process environment configures: the bundle
/// [`BUNDLE_VARIABLE`] names, loaded into a fresh runtime whose modules build
/// views through `table` in `environment`.
///
/// Answers `Ok(None)` when the variable is unset, which is what a host that
/// mounts no TypeScript sees. A host installs the handle into the environment
/// the view is mounted in with [`RuntimeHandle::install`].
///
/// # Errors
///
/// Returns [`BundleError`] when the variable is set but the bundle it names
/// cannot be resolved, read, or loaded. The variable being set is a request
/// to load that bundle, so none of those is silently an absent runtime.
pub fn configured_runtime(
    environment: &Environment,
    table: impl HostTable,
) -> Result<Option<RuntimeHandle>, BundleError> {
    let Some(value) = std::env::var_os(BUNDLE_VARIABLE) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Err(BundleError::Empty);
    }
    let path = resolve(Path::new(&value))?;
    let source = std::fs::read_to_string(&path).map_err(|source| BundleError::Read {
        path: path.clone(),
        source,
    })?;
    let runtime = load(environment, table, &source).map_err(|source| BundleError::Load {
        path: path.clone(),
        source,
    })?;
    tracing::debug!(path = %path.display(), "loaded the TypeScript bundle the host was configured with");
    Ok(Some(RuntimeHandle::new(runtime)))
}

/// The path rule: absolute as written, relative against `CARGO_MANIFEST_DIR`.
fn resolve(path: &Path) -> Result<PathBuf, BundleError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let manifest_dir =
        std::env::var_os(MANIFEST_DIR_VARIABLE).ok_or_else(|| BundleError::Relative {
            path: path.to_path_buf(),
        })?;
    Ok(PathBuf::from(manifest_dir).join(path))
}

/// A runtime with `source` loaded into it.
fn load(
    environment: &Environment,
    table: impl HostTable,
    source: &str,
) -> Result<TsRuntime, TsError> {
    let runtime = TsRuntime::new(environment.clone(), table)?;
    runtime.load(source)?;
    Ok(runtime)
}
