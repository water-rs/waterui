//! `JsRuntime` over QuickJS-NG via `rquickjs`.
//!
//! One [`QuickJsRuntime`] owns one `QuickJS` context. Retained handles carry
//! `Persistent<Value<'static>>`; opaque Rust values cross as a JavaScript
//! object whose `__waterui_opaque` property indexes a registry the runtime
//! owns — JavaScript can hold and pass the box but cannot reach the `Rc`.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use rquickjs::context::EvalOptions;
use rquickjs::function::{Args, Rest};
use rquickjs::{
    Array, BigInt as QBigInt, Coerced, Context, Ctx, Exception, Function, IntoJs, Object,
    Persistent, Type, Value,
};
use waterui_ts_engine::handle::Handle;
use waterui_ts_engine::{
    BigInt, HostFunction, JsError, JsFunction, JsObject, JsRuntime, JsValue, Opaque,
};

/// The property a boxed [`Opaque`] carries: the index of its `Rc` in the
/// runtime's registry.
const OPAQUE_KEY: &str = "__waterui_opaque";

/// The namespace object host functions register under:
/// `globalThis.__waterui_host`.
const HOST_NAMESPACE: &str = "__waterui_host";

/// The retained reference behind `JsFunction` and `JsObject`.
type Retained = Persistent<Value<'static>>;

/// The opaque registry: `Rc`s boxed for JavaScript to hold.
type Registry = Rc<RefCell<Vec<Rc<dyn std::any::Any>>>>;

/// The QuickJS-NG engine.
///
/// `!Send + !Sync`: a `QuickJS` context is bound to the thread that created it.
pub struct QuickJsRuntime {
    /// Owns the underlying `Runtime` — `Context` keeps it alive.
    context: Context,
    registry: Registry,
}

impl QuickJsRuntime {
    /// The `__waterui_host` namespace object, created empty at construction.
    fn host_namespace<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>, JsError> {
        ctx.globals()
            .get::<_, Object<'_>>(HOST_NAMESPACE)
            .map_err(|error| map_error(ctx, &error))
    }

    /// Boxes `opaque` as a JavaScript object holding its registry index.
    fn box_opaque<'js>(
        ctx: &Ctx<'js>,
        registry: &Registry,
        opaque: &Opaque,
    ) -> Result<Value<'js>, JsError> {
        let index = {
            let mut registry = registry.borrow_mut();
            registry.push(opaque.inner().clone());
            registry.len() - 1
        };
        #[expect(
            clippy::cast_precision_loss,
            reason = "a registry index is bounded by the number of boxed values, always exact in a double"
        )]
        let index = index as f64;
        let object = Object::new(ctx.clone()).map_err(|error| map_error(ctx, &error))?;
        object
            .set(OPAQUE_KEY, index)
            .map_err(|error| map_error(ctx, &error))?;
        Ok(object.into_value())
    }

    /// The `Rc` a boxed object points at, or `None` when it is not a box.
    fn unbox<'js>(
        ctx: &Ctx<'js>,
        registry: &Registry,
        object: &Object<'js>,
    ) -> Result<Option<Opaque>, JsError> {
        if !object
            .contains_key(OPAQUE_KEY)
            .map_err(|error| map_error(ctx, &error))?
        {
            return Ok(None);
        }
        let index: f64 = object
            .get(OPAQUE_KEY)
            .map_err(|error| map_error(ctx, &error))?;
        #[expect(
            clippy::cast_sign_loss,
            clippy::cast_possible_truncation,
            reason = "registry indices are non-negative and always fit usize"
        )]
        let index = index as usize;
        let boxed = registry.borrow().get(index).cloned().ok_or_else(|| {
            JsError::conversion("an opaque handle names a slot that does not exist")
        })?;
        Ok(Some(Opaque::from_inner(boxed)))
    }

    /// Rust → JavaScript.
    fn to_js<'js>(
        ctx: &Ctx<'js>,
        registry: &Registry,
        value: &JsValue,
    ) -> Result<Value<'js>, JsError> {
        let fail = |error: rquickjs::Error| map_error(ctx, &error);
        Ok(match value {
            JsValue::Undefined => Value::new_undefined(ctx.clone()),
            JsValue::Null => Value::new_null(ctx.clone()),
            JsValue::Bool(value) => Value::new_bool(ctx.clone(), *value),
            JsValue::Number(value) => Value::new_number(ctx.clone(), *value),
            JsValue::BigInt(BigInt::Signed(value)) => QBigInt::from_i64(ctx.clone(), *value)
                .map_err(fail)?
                .into_value(),
            JsValue::BigInt(BigInt::Unsigned(value)) => QBigInt::from_u64(ctx.clone(), *value)
                .map_err(fail)?
                .into_value(),
            JsValue::String(value) => value.clone().into_js(ctx).map_err(fail)?,
            JsValue::Array(items) => {
                let array = Array::new(ctx.clone()).map_err(fail)?;
                for (index, item) in items.iter().enumerate() {
                    array
                        .set(index, Self::to_js(ctx, registry, item)?)
                        .map_err(fail)?;
                }
                array.into_value()
            }
            JsValue::Object(entries) => {
                let object = Object::new(ctx.clone()).map_err(fail)?;
                for (key, item) in entries {
                    object
                        .set(key.as_str(), Self::to_js(ctx, registry, item)?)
                        .map_err(fail)?;
                }
                object.into_value()
            }
            JsValue::Function(function) => restore(ctx, function.handle())?,
            JsValue::ObjectRef(object) => restore(ctx, object.handle())?,
            JsValue::Opaque(opaque) => Self::box_opaque(ctx, registry, opaque)?,
        })
    }

    /// JavaScript → Rust.
    fn from_js<'js>(
        ctx: &Ctx<'js>,
        registry: &Registry,
        value: &Value<'js>,
    ) -> Result<JsValue, JsError> {
        let fail = |error: rquickjs::Error| map_error(ctx, &error);
        Ok(match value.type_of() {
            Type::Undefined => JsValue::Undefined,
            Type::Null => JsValue::Null,
            Type::Bool => JsValue::Bool(value.get::<bool>().map_err(fail)?),
            Type::Int | Type::Float => JsValue::Number(value.get::<f64>().map_err(fail)?),
            Type::String => JsValue::String(value.get::<String>().map_err(fail)?),
            Type::BigInt => {
                // Coerce to decimal text, then take the exact integer the
                // digits spell; a `bigint` beyond `u64` has no `JsValue`.
                let digits = value.get::<Coerced<String>>().map_err(fail)?.0;
                if let Ok(signed) = digits.parse::<i64>() {
                    JsValue::BigInt(BigInt::Signed(signed))
                } else if let Ok(unsigned) = digits.parse::<u64>() {
                    JsValue::BigInt(BigInt::Unsigned(unsigned))
                } else {
                    return Err(JsError::conversion(format!(
                        "a bigint beyond u64 cannot cross the bridge: {digits}"
                    )));
                }
            }
            Type::Function | Type::Constructor => JsValue::Function(JsFunction::from_handle(
                Handle::new(Persistent::save(ctx, value.clone())),
            )),
            Type::Array => {
                let array = value
                    .clone()
                    .into_array()
                    .ok_or_else(|| JsError::conversion("an array-typed value is not an array"))?;
                let mut items = Vec::with_capacity(array.len());
                for item in array.iter::<Value<'js>>() {
                    items.push(Self::from_js(ctx, registry, &item.map_err(fail)?)?);
                }
                JsValue::Array(items)
            }
            Type::Object | Type::Exception | Type::Promise | Type::Proxy => {
                let object = value
                    .clone()
                    .into_object()
                    .ok_or_else(|| JsError::conversion("an object-typed value is not an object"))?;
                if let Some(opaque) = Self::unbox(ctx, registry, &object)? {
                    return Ok(JsValue::Opaque(opaque));
                }
                if matches!(value.type_of(), Type::Object) {
                    let mut entries = Vec::with_capacity(object.len());
                    for pair in object.props::<String, Value<'js>>() {
                        let (key, item) = pair.map_err(fail)?;
                        entries.push((key, Self::from_js(ctx, registry, &item)?));
                    }
                    JsValue::Object(entries)
                } else {
                    return Err(JsError::conversion(format!(
                        "a {} cannot cross the bridge",
                        value.type_name()
                    )));
                }
            }
            other => {
                return Err(JsError::conversion(format!(
                    "a {} cannot cross the bridge",
                    other.as_str()
                )));
            }
        })
    }
}

/// Builds the JavaScript-side host function. A free function so the closure's
/// `ctx` and `args` can share one named `'js` — `Value` is invariant over it,
/// so two `'_` holes would never unify.
fn make_host_function<'js>(
    ctx: &Ctx<'js>,
    host: HostFunction,
    registry: Registry,
) -> Result<Function<'js>, JsError> {
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, Rest(args): Rest<Value<'js>>| host_call(&ctx, &args, &host, &registry),
    )
    .map_err(|error| map_error(ctx, &error))
}

/// Runs one host-function call: converts the arguments, calls `host`,
/// converts (or throws) the result.
fn host_call<'js>(
    ctx: &Ctx<'js>,
    args: &[Value<'js>],
    host: &HostFunction,
    registry: &Registry,
) -> rquickjs::Result<Value<'js>> {
    let mut converted = Vec::with_capacity(args.len());
    for arg in args {
        match QuickJsRuntime::from_js(ctx, registry, arg) {
            Ok(value) => converted.push(value),
            Err(error) => return Err(throw(ctx, &error)),
        }
    }
    match host(&converted) {
        Ok(value) => {
            QuickJsRuntime::to_js(ctx, registry, &value).map_err(|error| throw(ctx, &error))
        }
        Err(error) => Err(throw(ctx, &error)),
    }
}

/// Restores a retained handle to a live value of `ctx`.
fn restore<'js>(ctx: &Ctx<'js>, handle: &Handle) -> Result<Value<'js>, JsError> {
    let retained = handle
        .downcast::<Retained>()
        .ok_or_else(|| JsError::conversion("a handle from a different engine was passed in"))?;
    (*retained)
        .clone()
        .restore(ctx)
        .map_err(|error| map_error(ctx, &error))
}

/// Throws a `JsError` into JavaScript as an `Error` carrying its name and
/// message, and returns the `rquickjs` error that reports it to the caller.
fn throw(ctx: &Ctx<'_>, error: &JsError) -> rquickjs::Error {
    match Exception::from_message(ctx.clone(), &error.message) {
        Ok(exception) => {
            let _ = exception.as_object().set("name", error.name.as_str());
            exception.throw()
        }
        // The context is out of memory or torn down; still report an
        // exception so the call unwinds.
        Err(error) => error,
    }
}

/// The `rquickjs` error as a `JsError`, recovering the pending exception —
/// message, class name and stack — when one was thrown.
fn map_error(ctx: &Ctx<'_>, error: &rquickjs::Error) -> JsError {
    if !matches!(error, rquickjs::Error::Exception) {
        return JsError::new("InternalError", error.to_string());
    }
    let thrown = ctx.catch();
    if let Some(exception) = thrown.as_exception() {
        let name = exception
            .as_object()
            .get::<_, Option<Coerced<String>>>("name")
            .ok()
            .flatten()
            .map_or_else(|| String::from("Error"), |name| name.0);
        let message = exception
            .message()
            .unwrap_or_else(|| String::from("JavaScript exception"));
        let mut error = JsError::new(name, message);
        error.stack = exception.stack();
        return error;
    }
    // A non-`Error` throw (`throw 3`) still surfaces as a `JsError`.
    let message = thrown
        .get::<Coerced<String>>()
        .map_or_else(|_| String::from("JavaScript exception"), |text| text.0);
    JsError::new("Error", message)
}

impl JsRuntime for QuickJsRuntime {
    fn new() -> Result<Self, JsError>
    where
        Self: Sized,
    {
        let runtime =
            rquickjs::Runtime::new().map_err(|error| JsError::new("Error", error.to_string()))?;
        let context =
            Context::full(&runtime).map_err(|error| JsError::new("Error", error.to_string()))?;
        let this = Self {
            context,
            registry: Registry::default(),
        };
        this.context.with(|ctx| {
            ctx.globals()
                .set(
                    HOST_NAMESPACE,
                    Object::new(ctx.clone()).map_err(|e| map_error(&ctx, &e))?,
                )
                .map_err(|error| map_error(&ctx, &error))
        })?;
        Ok(this)
    }

    fn eval(&self, source: &str, name: &str) -> Result<JsValue, JsError> {
        self.context.with(|ctx| {
            let mut options = EvalOptions::default();
            options.filename = Some(name.to_owned());
            let value = ctx
                .eval_with_options::<Value<'_>, Vec<u8>>(source.into(), options)
                .map_err(|error| map_error(&ctx, &error))?;
            Self::from_js(&ctx, &self.registry, &value)
        })
    }

    fn call(&self, function: &JsFunction, args: &[JsValue]) -> Result<JsValue, JsError> {
        self.context.with(|ctx| {
            let value = restore(&ctx, function.handle())?;
            let function = value.into_function().ok_or_else(|| {
                JsError::conversion("a JsFunction no longer refers to a function")
            })?;
            let mut packed = Args::new(ctx.clone(), args.len());
            for arg in args {
                packed
                    .push_arg(Self::to_js(&ctx, &self.registry, arg)?)
                    .map_err(|error| map_error(&ctx, &error))?;
            }
            let value = function
                .call_arg::<Value<'_>>(packed)
                .map_err(|error| map_error(&ctx, &error))?;
            Self::from_js(&ctx, &self.registry, &value)
        })
    }

    fn register(
        &self,
        name: &str,
        function: impl Fn(&[JsValue]) -> Result<JsValue, JsError> + 'static,
    ) -> Result<(), JsError> {
        let host: HostFunction = Rc::new(function);
        let registry = self.registry.clone();
        self.context.with(|ctx| {
            let function = make_host_function(&ctx, host, registry)?;
            Self::host_namespace(&ctx)?
                .set(name, function)
                .map_err(|error| map_error(&ctx, &error))
        })
    }

    fn retain(&self, value: &JsValue) -> Result<JsObject, JsError> {
        self.context.with(|ctx| match value {
            JsValue::Object(_) | JsValue::ObjectRef(_) | JsValue::Function(_) => {
                let value = Self::to_js(&ctx, &self.registry, value)?;
                Ok(JsObject::from_handle(Handle::new(Persistent::save(
                    &ctx, value,
                ))))
            }
            _ => Err(JsError::conversion(
                "only JavaScript objects and functions can be retained",
            )),
        })
    }
}

impl fmt::Debug for QuickJsRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("QuickJsRuntime(..)")
    }
}
