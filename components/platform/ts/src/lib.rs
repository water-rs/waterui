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

#[cfg(target_os = "watchos")]
compile_error!(
    "waterui-ts is unsupported on watchOS: the platform has no JavaScriptCore, \
     and embedding QuickJS-NG is not supported there either. TypeScript views \
     are available on macOS, iOS, tvOS and visionOS (JavaScriptCore) and on \
     Android, Linux and Windows (QuickJS-NG) — this asymmetry is documented, \
     not faked."
);

pub use waterui_ts_engine as engine;

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
