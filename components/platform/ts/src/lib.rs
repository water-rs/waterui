//! `WaterUI`'s TypeScript runtime: views authored in TypeScript, driven by an
//! embedded JavaScript engine.
//!
//! The engine is selected by target: Apple platforms use the system
//! `JavaScriptCore` (`waterui-ts-engine-jsc`, zero added binary size), every
//! other platform uses `QuickJS-NG` (`waterui-ts-engine-quickjs`). [`Engine`]
//! names the selected implementation; the contract itself lives in
//! [`waterui_ts_engine`], re-exported here as [`engine`].
//!
//! watchOS is the documented asymmetry: it ships no `JavaScriptCore`, and the
//! TypeScript runtime is unsupported there.
//!
//! # What this crate is
//!
//! [`TsRuntime`] is one engine with one bundle loaded into it. The bundle is
//! the JavaScript library in `src/js` plus the application's compiled modules;
//! it publishes [`RuntimeGlobal`], and the runtime installs a [`HostTable`]
//! through it so JSX becomes real `WaterUI` views.
//!
//! Between the two sits the [`Bridge`]: the reactive seam. A JavaScript signal
//! that reaches a native view becomes a real [`Binding<T>`], an accessor
//! becomes a [`Computed<T>`], and a Rust binding handed to TypeScript becomes
//! a JavaScript signal — each one pushed in both directions, with backends
//! reading the Rust value and never crossing into the engine to `get`.
//! [`IntoJs`] and [`FromJs`] are the value half of the same seam, implementing
//! exactly the mapping [`schema`] declares.
//!
//! Materialization is lazy: a signal a TypeScript module never hands to a
//! native view creates nothing on the Rust side.
//!
//! [`Binding<T>`]: nami::Binding
//! [`Computed<T>`]: nami::Computed

#[cfg(target_os = "watchos")]
compile_error!(
    "waterui-ts is unsupported on watchOS: the platform has no JavaScriptCore, \
     and embedding QuickJS-NG is not supported there either. TypeScript views \
     are available on macOS, iOS, tvOS and visionOS (JavaScriptCore) and on \
     Android, Linux and Windows (QuickJS-NG) — this asymmetry is documented, \
     not faked."
);

// The `TsType`/`TsProps` derives expand to `::waterui_ts::…` for a crate that
// reaches this one directly, so the name has to resolve inside this crate too.
extern crate self as waterui_ts;

mod bridge;
mod callback;
mod cell;
mod convert;
mod environment;
mod error;
mod host;
mod runtime;
mod runtime_global;
mod tether;
mod view;

pub use waterui_ts_engine as engine;

/// The props contract a mounted TypeScript view module is typed against.
///
/// This is `waterui-ts-schema` re-exported, so a `TsType`/`TsProps` derive on
/// a crate that consumes the facade can name `waterui::ts::schema` and find
/// every item the expansion uses.
pub use waterui_ts_schema as schema;

pub use bridge::{Bridge, ReactiveSource};
pub use convert::{FromJs, IntoJs, support};
pub use environment::{HostLocale, Theme, locale, theme};
pub use error::TsError;
pub use host::HostTable;
pub use runtime::TsRuntime;
pub use runtime_global::RuntimeGlobal;
pub use view::ViewSlot;

/// The JavaScript engine implementation for this target.
#[cfg(all(target_vendor = "apple", not(target_os = "watchos")))]
pub type Engine = waterui_ts_engine_jsc::JscRuntime;

/// The JavaScript engine implementation for this target.
#[cfg(not(target_vendor = "apple"))]
pub type Engine = waterui_ts_engine_quickjs::QuickJsRuntime;

#[cfg(all(test, not(target_os = "watchos")))]
mod tests {
    use crate::Engine;
    use crate::engine::{JsRuntime, JsValue};

    /// The selected engine satisfies the shared contract end to end on this
    /// target: construct, evaluate, register a host function, call it.
    #[test]
    fn selected_engine_smoke() {
        let engine = Engine::new().expect("the engine constructs");
        let value = engine
            .eval("40 + 2", "smoke.js")
            .expect("evaluating a number literal");
        assert_eq!(value, JsValue::Number(42.0));

        engine
            .register("double", |args| {
                let value = args
                    .first()
                    .and_then(JsValue::as_f64)
                    .expect("the host function receives a number");
                Ok(JsValue::from(value * 2.0))
            })
            .expect("registering a host function");
        let value = engine
            .eval("__waterui_host.double(21)", "smoke.js")
            .expect("calling the host function");
        assert_eq!(value, JsValue::Number(42.0));
    }
}
