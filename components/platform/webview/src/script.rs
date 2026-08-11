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
    /// Renders the call a backend evaluates to run this expression.
    ///
    /// The result is always the JSON envelope [`JsOutcome`] parses, whatever the
    /// engine underneath.
    #[must_use]
    pub fn wrapped_call(&self) -> String {
        wrap(&format!("return ({});", self.source.as_str()), self.args())
    }
}

impl JsProgram {
    /// Renders the call a backend evaluates to run this program.
    #[must_use]
    pub fn wrapped_call(&self) -> String {
        wrap(self.source.as_str(), self.args())
    }
}

/// Builds `__wateruiEval(async (__wa0, …) => { … }, [args])`.
///
/// The arguments are serialized into the call rather than concatenated into the
/// body, so an interpolated value cannot be read as source.
fn wrap(body: &str, args: &[serde_json::Value]) -> String {
    let parameters = (0..args.len())
        .map(|index| format!("__wa{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let args = serde_json::to_string(args).expect("interpolated JavaScript arguments must serialize");
    format!("globalThis.__wateruiEval(async ({parameters}) => {{ {body} }}, {args})")
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
    use super::{JsError, JsExpr, JsOutcome};

    fn outcome(json: &str) -> JsOutcome {
        serde_json::from_str(json).expect("well-formed outcome")
    }

    #[test]
    fn a_value_decodes_into_the_requested_type() {
        let title: String = outcome(r#"{"ok":true,"value":"WaterUI"}"#)
            .decode()
            .expect("decodes");
        assert_eq!(title, "WaterUI");

        let count: u32 = outcome(r#"{"ok":true,"value":7}"#).decode().expect("decodes");
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
}
