//! Typed JavaScript for [`WebView::eval`](crate::WebView::eval) and
//! [`WebView::exec`](crate::WebView::exec).
//!
//! Two things separate this from handing a backend a string:
//!
//! * An expression and a program are different types, because backends disagree
//!   about which one they evaluate. `WebKit`'s `callAsyncJavaScript` takes a
//!   function body and needs an explicit `return`; CDP's `Runtime.evaluate`
//!   takes an expression. Deciding once, in the type, keeps that disagreement out
//!   of user code.
//! * Interpolated values travel beside the source rather than inside it. A value
//!   spliced into source is both an injection risk and unreadable when it is a
//!   large payload; the `exec!`/`eval!` macros put `@{...}` holes into arguments
//!   the engine binds.

use waterui_str::Str;

/// Everything `WaterUI` injects at document start: the bridge and the evaluation
/// wrapper.
///
/// Backends inject this one constant rather than assembling the pieces, so a
/// backend cannot end up with a bridge but no wrapper.
pub const DOCUMENT_START_SCRIPT: &str = concat!(
    include_str!("js/bridge.js"),
    include_str!("js/eval.js"),
    include_str!("js/state.js"),
);

/// A JavaScript expression that evaluates to a value.
///
/// Build one with [`eval!`](waterui_macros::eval) for a checked literal, or
/// [`JsExpr::raw`] for source that only exists at runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsExpr {
    source: Str,
    args: Vec<serde_json::Value>,
}

/// A JavaScript program run for its effects.
///
/// Build one with [`exec!`](crate::exec) or [`js_file!`](crate::js_file) for a
/// checked literal or file, or [`JsProgram::raw`] for runtime source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsProgram {
    source: Str,
    args: Vec<serde_json::Value>,
}

macro_rules! script_body {
    ($ty:ident, $what:literal) => {
        impl $ty {
            /// Builds from source that only exists at runtime.
            ///
            /// Unchecked by construction, and deliberately a different spelling
            /// from the macros so it is visible in review.
            #[must_use]
            pub fn raw(source: impl Into<Str>) -> Self {
                Self {
                    source: source.into(),
                    args: Vec::new(),
                }
            }

            /// Used by the macros; not part of the stable surface.
            #[doc(hidden)]
            #[must_use]
            pub const fn __from_parts(source: &'static str, args: Vec<serde_json::Value>) -> Self {
                Self {
                    source: Str::from_static(source),
                    args,
                }
            }

            #[doc = concat!("The ", $what, " source.")]
            #[must_use]
            pub const fn source(&self) -> &str {
                self.source.as_str()
            }

            /// The values bound to the source's `@{...}` holes, in order.
            #[must_use]
            pub fn args(&self) -> &[serde_json::Value] {
                &self.args
            }
        }

        impl From<&'static str> for $ty {
            fn from(source: &'static str) -> Self {
                Self::raw(source)
            }
        }
    };
}

script_body!(JsExpr, "expression");
script_body!(JsProgram, "program");

impl JsExpr {
    /// Renders the async function body a backend runs to evaluate this
    /// expression.
    ///
    /// The value it resolves with is always the JSON envelope [`JsOutcome`]
    /// parses, whatever the engine underneath.
    #[must_use]
    pub fn wrapped_call(&self) -> String {
        wrap(&format!("return ({});", self.source.as_str()), self.args())
    }
}

impl JsProgram {
    /// Renders the async function body a backend runs to execute this program.
    #[must_use]
    pub fn wrapped_call(&self) -> String {
        wrap(self.source.as_str(), self.args())
    }

    /// Renders the program as one self-contained script, depending on nothing
    /// `WaterUI` installed.
    ///
    /// [`wrapped_call`](Self::wrapped_call) is for the two APIs that hand a
    /// backend a function body and wait for an answer. Injection has neither: a
    /// script registered with [`WebViewOpen::inject`](crate::WebViewOpen::inject)
    /// is a whole program the engine runs on every page load, before the bridge
    /// is guaranteed to exist and with nobody to receive an envelope. So this
    /// drops the wrapper and keeps only the part that binds arguments: an async
    /// IIFE whose parameters are the `__wa0`, `__wa1`, … names the macros already
    /// substituted for the source's `@{...}` holes, applied to the same JSON
    /// array literal `wrapped_call` inlines. The argument path is therefore the
    /// one `__wateruiEval` takes — `fn.apply(null, args)` over an inlined JSON
    /// array — minus the envelope, so a program means the same thing injected as
    /// it does executed.
    ///
    /// This is what `From<JsProgram> for Str` uses, which is why a program can be
    /// handed straight to `inject`.
    #[must_use]
    pub fn standalone_script(&self) -> String {
        let parameters = parameters(self.args());
        let arguments = arguments(self.args());
        let source = self.source.as_str();
        format!("(async ({parameters}) => {{ {source} }}).apply(null, {arguments});")
    }
}

/// Renders the program for injection; see [`JsProgram::standalone_script`].
impl From<JsProgram> for Str {
    fn from(program: JsProgram) -> Self {
        Self::from(program.standalone_script())
    }
}

/// Builds `return __wateruiEval(async (__wa0, …) => { … }, [args]);`.
///
/// The arguments are serialized into the call rather than concatenated into the
/// body, so an interpolated value cannot be read as source.
///
/// The result is a **function body**, not a bare expression, because
/// [`call_async_javascript`](crate::WebViewHandle::call_async_javascript) is
/// what runs it: `__wateruiEval` is `async`, so the value here is a promise that
/// the backend has to await. Returning it from an async function body is the
/// shape every engine's awaiting API takes.
fn wrap(body: &str, args: &[serde_json::Value]) -> String {
    let parameters = parameters(args);
    let arguments = arguments(args);
    format!("return globalThis.__wateruiEval(async ({parameters}) => {{ {body} }}, {arguments});")
}

/// The parameter list binding one argument each, in order.
///
/// The names match what the `exec!`/`eval!` macros write in place of a `@{...}`
/// hole, which is the whole reason an argument reaches the source at all.
fn parameters(args: &[serde_json::Value]) -> String {
    (0..args.len())
        .map(|index| format!("__wa{index}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Serializes the arguments as the JavaScript array literal the source is
/// applied to.
///
/// U+2028 and U+2029 are ordinary characters inside a JSON string but were line
/// terminators in JavaScript source until ES2019, where they end a string
/// literal and leave the rest of the script unparseable. Which engine runs this
/// is the backend's business and not always a current one, so they are escaped
/// unconditionally. The escape denotes the same character to a JSON reader,
/// and the raw one can only ever appear inside a string, so replacing it blind
/// cannot touch anything else.
fn arguments(args: &[serde_json::Value]) -> String {
    serde_json::to_string(args)
        .expect("interpolated JavaScript arguments must serialize")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Why evaluating JavaScript failed.
///
/// The variants that matter are kept apart: a script that threw is a different
/// problem from a result that could not be represented, which is different again
/// from asking for the wrong Rust type. Collapsing them into one string, as the
/// old `Result<Str, Str>` did, made all three look alike.
#[derive(Debug, thiserror::Error)]
pub enum JsError {
    /// The script threw.
    #[error("JavaScript threw: {message}")]
    Exception {
        /// The exception's message.
        message: Str,
        /// Its stack, when the engine provided one.
        stack: Option<Str>,
    },
    /// The value could not be turned into JSON — a circular structure, a DOM
    /// node, a function.
    #[error("result is not JSON-serializable: {0}")]
    Unserializable(Str),
    /// The result was well-formed but is not the requested Rust type.
    #[error("cannot decode result as `{expected}`: {source}")]
    Decode {
        /// The Rust type that was asked for.
        expected: &'static str,
        /// The underlying serde error.
        #[source]
        source: serde_json::Error,
    },
    /// The document was replaced before the script ran.
    #[error("the document was replaced before the script ran")]
    Navigated,
    /// The web view went away.
    #[error("the web view was closed")]
    Closed,
}

/// What a backend hands back after running a script.
///
/// Every backend produces this same envelope, built by the shared wrapper in
/// JavaScript, so the result of `document.title` does not depend on which engine
/// ran it. Backends used to disagree: one returned the string, another its
/// JSON-quoted form, a third whatever its platform marshalling produced.
///
/// `ok` is a JSON boolean rather than a tag, so this is a struct rather than an
/// enum; [`JsOutcome::decode`] is the only way it is read.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct JsOutcome {
    /// Whether the script completed without throwing.
    ok: bool,
    /// The value, when there was one. Absent for `undefined`.
    #[serde(default)]
    value: Option<serde_json::Value>,
    /// Set when the result existed but could not be turned into JSON.
    #[serde(default)]
    unserializable: bool,
    /// The failure message.
    #[serde(default)]
    message: String,
    /// The stack, when the engine provided one.
    #[serde(default)]
    stack: Option<String>,
}

impl JsOutcome {
    /// Decodes the outcome into `T`.
    ///
    /// # Errors
    ///
    /// Returns the failure the script reported, or a [`JsError::Decode`] when the
    /// value is well-formed but is not a `T`.
    pub fn decode<T: serde::de::DeserializeOwned>(self) -> Result<T, JsError> {
        if !self.ok {
            let message = Str::from(self.message);
            return Err(if self.unserializable {
                JsError::Unserializable(message)
            } else {
                JsError::Exception {
                    message,
                    stack: self.stack.map(Str::from),
                }
            });
        }
        let value = self.value.unwrap_or(serde_json::Value::Null);
        serde_json::from_value(value).map_err(|source| JsError::Decode {
            expected: core::any::type_name::<T>(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{JsError, JsExpr, JsOutcome, JsProgram};
    use serde_json::json;
    use waterui_str::Str;

    /// Builds the program `exec!("app.set(@{a}, @{b})")` expands to: the macro has
    /// already replaced each `@{...}` hole with a positional parameter name.
    fn program(source: &'static str, args: Vec<serde_json::Value>) -> JsProgram {
        JsProgram::__from_parts(source, args)
    }

    fn outcome(json: &str) -> JsOutcome {
        serde_json::from_str(json).expect("well-formed outcome")
    }

    #[test]
    fn a_value_decodes_into_the_requested_type() {
        let title: String = outcome(r#"{"ok":true,"value":"WaterUI"}"#)
            .decode()
            .expect("decodes");
        assert_eq!(title, "WaterUI");

        let count: u32 = outcome(r#"{"ok":true,"value":7}"#)
            .decode()
            .expect("decodes");
        assert_eq!(count, 7);
    }

    /// `undefined` omits the key entirely, `null` sends it explicitly; both are
    /// the unit value, and neither is an error.
    #[test]
    fn undefined_and_null_both_decode_as_unit() {
        outcome(r#"{"ok":true}"#).decode::<()>().expect("undefined");
        outcome(r#"{"ok":true,"value":null}"#)
            .decode::<()>()
            .expect("null");
    }

    #[test]
    fn a_throw_and_a_wrong_type_are_different_errors() {
        let threw = outcome(r#"{"ok":false,"message":"boom","stack":"at f"}"#)
            .decode::<String>()
            .expect_err("throws");
        assert!(matches!(threw, JsError::Exception { .. }));

        let wrong_type = outcome(r#"{"ok":true,"value":"not a number"}"#)
            .decode::<u32>()
            .expect_err("wrong type");
        assert!(matches!(wrong_type, JsError::Decode { .. }));
    }

    #[test]
    fn an_unserializable_result_is_its_own_error() {
        let error = outcome(r#"{"ok":false,"unserializable":true,"message":"circular"}"#)
            .decode::<String>()
            .expect_err("unserializable");
        assert!(matches!(error, JsError::Unserializable(_)));
    }

    #[test]
    fn raw_source_carries_no_arguments() {
        let expr = JsExpr::raw("document.title");
        assert_eq!(expr.source(), "document.title");
        assert!(expr.args().is_empty());
    }

    /// The wrapped form is a function *body* that returns the wrapper's promise,
    /// so the backend's awaiting API resolves it. Emitting a bare expression is
    /// what left every backend holding an unawaited `Promise` instead of the
    /// envelope, and made `eval`/`exec` fail on every engine that does not await
    /// on its own.
    #[test]
    fn the_wrapped_form_returns_the_promise_for_the_backend_to_await() {
        let call = JsExpr::raw("document.title").wrapped_call();
        assert!(
            call.starts_with("return globalThis.__wateruiEval("),
            "{call}"
        );
        assert!(call.trim_end().ends_with(");"), "{call}");
    }

    /// The parameters are the names the macro substituted for the holes, and the
    /// values reach them the way `__wateruiEval` delivers them: applied from an
    /// inlined JSON array.
    #[test]
    fn a_standalone_script_binds_each_hole_positionally() {
        let script =
            program("app.set(__wa0, __wa1);", vec![json!("dark"), json!(2)]).standalone_script();
        assert_eq!(
            script,
            r#"(async (__wa0,__wa1) => { app.set(__wa0, __wa1); }).apply(null, ["dark",2]);"#
        );
    }

    /// The same values, spelled the same way, whichever rendering runs them —
    /// which is what makes injecting a program mean what executing it means.
    #[test]
    fn both_renderings_inline_the_same_arguments() {
        let program = program("app.set(__wa0);", vec![json!({ "seen": true })]);
        let arguments = r#"[{"seen":true}]"#;
        assert!(program.standalone_script().contains(arguments));
        assert!(program.wrapped_call().contains(arguments));
    }

    #[test]
    fn a_program_without_arguments_takes_no_parameters() {
        assert_eq!(
            program("document.title = 'hi';", Vec::new()).standalone_script(),
            "(async () => { document.title = 'hi'; }).apply(null, []);"
        );
    }

    /// A string argument is JSON, so its quotes and newlines are escaped rather
    /// than closing the literal or ending the line — the property that keeps an
    /// interpolated value from being read as source.
    #[test]
    fn a_string_argument_cannot_break_out_of_its_literal() {
        let script = program(
            "log(__wa0);",
            vec![json!("\");\nalert('pwned');//"), json!("a\tb")],
        )
        .standalone_script();
        assert!(
            script.contains(r#"["\");\nalert('pwned');//","a\tb"]"#),
            "{script}"
        );
        assert!(!script.contains('\n'), "{script}");
    }

    /// U+2028 and U+2029 pass through `serde_json` raw: legal JSON, and a line
    /// terminator to a pre-ES2019 JavaScript parser, which would end the string
    /// literal and leave the rest of the script unparseable.
    #[test]
    fn line_separators_are_escaped_in_both_renderings() {
        let program = program("log(__wa0);", vec![json!("before\u{2028}\u{2029}after")]);
        let escaped = "[\"before\\u2028\\u2029after\"]";
        for script in [program.standalone_script(), program.wrapped_call()] {
            assert!(script.contains(escaped), "{script}");
            assert!(!script.contains('\u{2028}'), "{script}");
            assert!(!script.contains('\u{2029}'), "{script}");
        }
    }

    /// What `WebViewOpen::inject` relies on: a program converts to the script it
    /// renders, so it can be passed where a `Str` is expected.
    #[test]
    fn converting_to_a_string_renders_the_standalone_script() {
        let program = program("app.ready(__wa0);", vec![json!(true)]);
        assert_eq!(
            Str::from(program.clone()),
            Str::from(program.standalone_script())
        );
    }
}
