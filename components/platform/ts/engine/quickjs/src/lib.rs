//! The QuickJS-NG implementation of [`JsRuntime`].
//!
//! QuickJS-NG is embedded (through `rquickjs`) on every target that is not an
//! Apple platform: Android, Linux, Windows.
//!
//! [`JsRuntime`]: waterui_ts_engine::JsRuntime

mod runtime;

pub use runtime::QuickJsRuntime;

#[cfg(test)]
mod tests {
    use crate::QuickJsRuntime;
    use waterui_ts_engine::conformance_tests;

    conformance_tests!(QuickJsRuntime);
}

#[cfg(test)]
mod promise_jobs {
    use crate::QuickJsRuntime;
    use waterui_ts_engine::{JsRuntime, JsValue};

    /// A promise reaction queued by a script runs once that script returns,
    /// as on every other engine; `await` in a handler depends on it.
    #[test]
    fn promise_reaction_runs_after_eval() {
        let runtime = QuickJsRuntime::new().expect("runtime");
        runtime
            .eval(
                "globalThis.step = 0; (async () => { await null; globalThis.step = 1; })(); undefined;",
                "promise_jobs.js",
            )
            .expect("script runs");
        let step = runtime.eval("globalThis.step", "read.js").expect("read");
        assert_eq!(
            step,
            JsValue::Number(1.0),
            "the awaited continuation never resumed"
        );
    }
}
