//! The engine-agnostic half of `WaterUI`'s TypeScript runtime.
//!
//! A [`JsRuntime`] is an embedded JavaScript engine pinned to the thread that
//! created it — deliberately neither `Send` nor `Sync` — that evaluates
//! bundles, calls JavaScript functions, and hosts Rust functions under
//! `globalThis.__waterui_host`. [`JsValue`] is the currency crossing the seam
//! in both directions; [`JsError`] is what a JavaScript exception becomes.
//!
//! Two engines implement the contract: `waterui-ts-engine-jsc` (the system
//! `JavaScriptCore` on Apple platforms) and `waterui-ts-engine-quickjs`
//! (`QuickJS-NG` everywhere else). `waterui-ts` picks between them by `cfg`, so
//! the rest of the runtime never names an engine.

mod error;
#[doc(hidden)]
pub mod handle;
mod runtime;
mod value;

#[cfg(feature = "conformance")]
pub mod conformance;

pub use error::JsError;
pub use runtime::{HostFunction, JsRuntime};
pub use value::{BigInt, JsFunction, JsObject, JsValue, MAX_SAFE_INTEGER, Opaque};
