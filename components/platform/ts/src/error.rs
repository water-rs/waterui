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
    #[error("the bundle carries no module \"{id}\"")]
    UnknownModule {
        /// The requested module id.
        id: Str,
    },

    /// A bundle was already evaluated in this runtime.
    #[error(
        "this runtime already loaded a bundle: one context evaluates one bundle, and an update \
         takes effect at the next launch with a fresh runtime"
    )]
    BundleAlreadyLoaded,

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
