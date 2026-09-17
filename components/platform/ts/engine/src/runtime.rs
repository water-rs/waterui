//! The `JsRuntime` contract.

use std::rc::Rc;

use crate::{JsError, JsFunction, JsObject, JsValue};

/// A Rust function callable from JavaScript as `__waterui_host.<name>(…)`.
///
/// The slice carries the JavaScript arguments converted to [`JsValue`]; the
/// return converts back. `Err` propagates into JavaScript as a thrown
/// `Error` carrying the [`JsError::message`].
pub type HostFunction = Rc<dyn Fn(&[JsValue]) -> Result<JsValue, JsError>>;

/// An embedded JavaScript engine, pinned to the thread that created it.
///
/// Implementations are deliberately neither `Send` nor `Sync`: both
/// `JavaScriptCore` contexts and `QuickJS` runtimes are bound to their thread,
/// and `WaterUI` drives the runtime from the main thread only. The trait says
/// nothing about the engine — that is the point — so `waterui-ts` selects
/// the implementation by `cfg` and the rest of the runtime never names one.
///
/// A runtime owns a fresh global object. Host functions register under the
/// `__waterui_host` namespace object on it (`globalThis.__waterui_host.<name>`).
/// When and how the engine reclaims garbage is the engine's business; the
/// contract never exposes a collector.
pub trait JsRuntime: 'static {
    /// Creates a runtime with a fresh global object.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when the engine cannot be initialized.
    fn new() -> Result<Self, JsError>
    where
        Self: Sized;

    /// Evaluates `source` as a classic script — sloppy mode, like an inline
    /// `<script>` — named `name` in stack traces, and converts the
    /// completion value. A bundle opts into strict mode with its own
    /// `"use strict"` directive.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] for a syntax error, a thrown exception (with its
    /// JavaScript stack), or a completion value that has no [`JsValue`].
    fn eval(&self, source: &str, name: &str) -> Result<JsValue, JsError>;

    /// Calls `function` with `this` bound to `undefined` and converts the
    /// return value.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when `function` was produced by a different
    /// engine, when the call throws (with its JavaScript stack), or when an
    /// argument or the return value has no [`JsValue`].
    fn call(&self, function: &JsFunction, args: &[JsValue]) -> Result<JsValue, JsError>;

    /// Registers `function` as `globalThis.__waterui_host.<name>`, owned by
    /// the runtime for as long as the context lives.
    ///
    /// A returned `Err` throws inside JavaScript as an `Error` whose message
    /// is the [`JsError::message`]; a returned `Ok` converts to JavaScript
    /// the same way [`JsRuntime::call`] arguments convert in.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when the function cannot be installed.
    fn register(
        &self,
        name: &str,
        function: impl Fn(&[JsValue]) -> Result<JsValue, JsError> + 'static,
    ) -> Result<(), JsError>;

    /// Retains a JavaScript object as an opaque [`JsObject`] handle.
    ///
    /// [`JsValue::Object`] data materializes as a fresh JavaScript object and
    /// is retained; [`JsValue::ObjectRef`] and [`JsValue::Function`] refer to
    /// their existing value. Retaining a function yields a `JsObject`
    /// wrapping that same function — it crosses back through
    /// [`JsValue::ObjectRef`], and `ptr_eq` between it and the original
    /// [`JsFunction`] is `false` because each handle is a fresh reference.
    /// The handle goes back into JavaScript through [`JsValue::ObjectRef`].
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when `value` is not an object — scalars need no
    /// handle, and [`JsValue::Opaque`] is already one.
    fn retain(&self, value: &JsValue) -> Result<JsObject, JsError>;
}
