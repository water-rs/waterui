//! What goes wrong between a bundle and the framework.
//!
//! Failures inside a value crossing the seam are [`JsError`]s — that is the
//! engine's own currency, and a host function returning one throws it into
//! JavaScript with its message intact. [`TsError`] is the layer above: loading
//! a bundle, reading the runtime global, mounting a module. It converts into
//! [`waterui_core::Error`] like any other `std::error::Error`, and into a
//! `JsError` when it has to be reported through a host call.

use suiteki::Str;
use waterui_ts_engine::{JsError, JsValue};

/// A failure in the TypeScript runtime itself.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum TsError {
    /// The engine raised an exception, or a value could not cross the seam.
    #[error(transparent)]
    Js(#[from] JsError),

    /// The evaluated bundle never published `globalThis.__waterui_runtime`.
    #[error(
        "the bundle did not publish globalThis.__waterui_runtime: its entry must end with \
         installRuntimeGlobal(modules) — see the runtime contract in HOST.md"
    )]
    MissingRuntimeGlobal,

    /// The runtime global exists but an entry is missing or is not a function.
    #[error(
        "globalThis.__waterui_runtime.{name} is {found}, not a function: the bundle carries a \
         runtime that does not match this framework version"
    )]
    RuntimeEntry {
        /// The entry that failed the check.
        name: &'static str,
        /// What was found there instead.
        found: &'static str,
    },

    /// The runtime global's `modules` entry is not a table of modules.
    #[error(
        "globalThis.__waterui_runtime.modules is {found}, not an object mapping each module \
         id to its default export"
    )]
    ModuleTable {
        /// What was found there instead.
        found: &'static str,
    },

    /// The `modules` table holds something that is not a module function.
    #[error(
        "the module \"{id}\" in globalThis.__waterui_runtime.modules is {found}, not a function: \
         a TypeScript view module's default export is the component function"
    )]
    Module {
        /// The module id as the bundle spells it.
        id: Str,
        /// What was found there instead.
        found: &'static str,
    },

    /// A module id no bundle entry carries.
    #[error("the bundle carries no module \"{id}\"; it carries {available}")]
    UnknownModule {
        /// The requested module id.
        id: Str,
        /// The ids the bundle does carry, so a mismatched bundle says what it
        /// holds instead of only what it does not.
        available: Str,
    },

    /// The runtime global's `contracts` entry is not a table of hashes.
    #[error(
        "globalThis.__waterui_runtime.contracts is {found}, not an object mapping each module \
         id to the hexadecimal props contract hash it was built against"
    )]
    ContractTable {
        /// What was found there instead.
        found: &'static str,
    },

    /// A `contracts` entry is not a hexadecimal 64-bit hash.
    #[error(
        "the contract hash of module \"{id}\" is \"{found}\", not the hexadecimal digits of a \
         64-bit props contract hash: a JavaScript number cannot hold one exactly, so it crosses \
         as a string"
    )]
    ContractHash {
        /// The module whose hash could not be read.
        id: Str,
        /// The text the bundle declared.
        found: Str,
    },

    /// The bundle declares no contract for a module being mounted.
    #[error(
        "the bundle declares no props contract for module \"{id}\": every module a binary mounts \
         is built against one, and a bundle that does not say which cannot be checked against \
         the binary"
    )]
    MissingContract {
        /// The module being mounted.
        id: Str,
    },

    /// The bundle's module was built against a different props contract.
    #[error(
        "module \"{id}\" was built against props contract {declared:#018x}, and this binary \
         mounts it with `{props}`, whose contract is {expected:#018x}: the bundle and the binary \
         are from different builds"
    )]
    ContractMismatch {
        /// The module being mounted.
        id: Str,
        /// The props type the binary mounts it with.
        props: &'static str,
        /// The hash the binary's props contract has.
        expected: u64,
        /// The hash the bundle declares for the module.
        declared: u64,
    },

    /// A mount was attempted with no runtime in the environment.
    ///
    /// The message covers every host at once: an application installs a
    /// runtime through its bundle loader, and a test or preview host through
    /// the bundle [`BUNDLE_VARIABLE`](crate::BUNDLE_VARIABLE) names — so a
    /// `#[waterui::test]` run under bare `cargo nextest`, where nothing sets
    /// the variable, reads here which command does.
    #[error(
        "no TypeScript runtime is installed in the environment, so module \"{id}\" cannot be \
         mounted: an application's bundle loader installs one at launch with \
         RuntimeHandle::install, and a test or preview host loads the bundle named by \
         {variable}, which `water test` and `water preview` set — under bare `cargo nextest` \
         nothing sets it, so run the tests through `water test`",
        variable = crate::BUNDLE_VARIABLE
    )]
    NoRuntimeInstalled {
        /// The module that was being mounted.
        id: Str,
    },

    /// What `mount` handed back is not a mounted tree.
    #[error(
        "mounting module \"{id}\" produced {found}, not the {{ handle, dispose }} the runtime \
         contract requires"
    )]
    NotMounted {
        /// The module that was mounted.
        id: Str,
        /// What came back instead.
        found: &'static str,
    },

    /// A mounted tree is missing one of its two entries.
    #[error(
        "the tree mounted for module \"{id}\" carries {found} under `{name}`, not what the \
         runtime contract requires"
    )]
    MountedEntry {
        /// The module that was mounted.
        id: Str,
        /// The entry that failed the check: `handle` or `dispose`.
        name: &'static str,
        /// What was found there.
        found: &'static str,
    },

    /// Clearing `globalThis.__waterui_runtime` left something behind.
    #[error(
        "globalThis.__waterui_runtime still holds {found} after being cleared: a bundle defined \
         it as a non-writable property, and a context whose runtime global cannot be replaced \
         can load no bundle"
    )]
    RuntimeGlobalNotCleared {
        /// What is still published there.
        found: &'static str,
    },

    /// A bundle was already evaluated in this runtime.
    #[error(
        "this runtime already loaded a bundle: one context evaluates one bundle, and an update \
         takes effect at the next launch with a fresh runtime"
    )]
    BundleAlreadyLoaded,

    /// A value was exported into JavaScript with no mount scope open.
    #[error(
        "a value can only be exported into JavaScript inside a mount scope: JavaScript holds \
         the exported value and Rust holds the cell feeding it, so the mount that asked for it \
         is what owns that cell — open one with Bridge::open_scope"
    )]
    NoMountScope,

    /// Something needed the runtime global before a bundle was loaded.
    #[error("no bundle is loaded yet: TsRuntime::load must run before {what}")]
    NotLoaded {
        /// What was attempted.
        what: &'static str,
    },
}

impl From<TsError> for JsError {
    fn from(error: TsError) -> Self {
        match error {
            TsError::Js(error) => error,
            other => Self::new("Error", other.to_string()),
        }
    }
}

/// The JavaScript type name of a value, for an error that says what was found.
#[must_use]
pub const fn kind_of(value: &JsValue) -> &'static str {
    match value {
        JsValue::Undefined => "undefined",
        JsValue::Null => "null",
        JsValue::Bool(_) => "a boolean",
        JsValue::Number(_) => "a number",
        JsValue::BigInt(_) => "a bigint",
        JsValue::String(_) => "a string",
        JsValue::Array(_) => "an array",
        JsValue::Object(_) | JsValue::ObjectRef(_) => "an object",
        JsValue::Function(_) => "a function",
        JsValue::Opaque(_) => "a native handle",
    }
}
