//! `JsRuntime` over QuickJS-NG via `rquickjs`.
//!
//! One [`QuickJsRuntime`] owns one `QuickJS` context. Retained handles carry
//! a `Persistent` plus the `Context` that produced them, so a handle keeps
//! its runtime alive instead of aborting in `JS_FreeRuntime`. Opaque Rust
//! values cross as `QuickJS` class instances holding the `Rc` as private
//! data — the class `Drop` is the finalizer, and no JavaScript-visible
//! property names the identity.

use std::any::Any;
use std::fmt;
use std::rc::Rc;

use rquickjs::class::{Class, JsClass, Readable, Trace, Tracer};
use rquickjs::context::EvalOptions;
use rquickjs::function::{Args, Rest, This};
use rquickjs::{
    Array, BigInt as QBigInt, Coerced, Constructor, Context, Ctx, Exception, Function, IntoJs,
    JsLifetime, Object, Persistent, Type, Value,
};
use waterui_ts_engine::handle::Handle;
use waterui_ts_engine::{
    BigInt, HostFunction, JsError, JsFunction, JsObject, JsRuntime, JsValue, MAX_CONVERSION_DEPTH,
    Opaque,
};

/// The namespace object host functions register under:
/// `globalThis.__waterui_host`.
const HOST_NAMESPACE: &str = "__waterui_host";

/// A boxed `Rc<dyn Any>` as a `QuickJS` class instance: the `Rc` is the
/// instance's private data and `Drop` is the class finalizer, so a box
/// JavaScript drops releases it. No JavaScript-visible property carries the
/// identity, so a plain object is never mistaken for a box.
struct OpaqueBox {
    value: Rc<dyn Any>,
}

impl<'js> Trace<'js> for OpaqueBox {
    fn trace<'a>(&self, _tracer: Tracer<'a, 'js>) {}
}

// SAFETY: `OpaqueBox` holds no JavaScript values — no `'js` lifetime derives
// from it, so `Changed` is the type itself.
unsafe impl JsLifetime<'_> for OpaqueBox {
    type Changed<'to> = Self;
}

impl<'js> JsClass<'js> for OpaqueBox {
    const NAME: &'static str = "WaterUiOpaque";

    type Mutable = Readable;

    /// No JavaScript constructor — boxes come into existence only when a
    /// `JsValue::Opaque` crosses into JavaScript.
    fn constructor(_ctx: &Ctx<'js>) -> rquickjs::Result<Option<Constructor<'js>>> {
        Ok(None)
    }
}

/// Per-context lookups `from_js` needs, kept as `QuickJS` userdata so a
/// host-function callback reaches them through its `Ctx`.
struct Bridge {
    /// `Object.prototype` — the line between plain objects and exotics.
    object_prototype: Persistent<Object<'static>>,
    /// `Object.prototype.toString` — names a rejected exotic's kind.
    to_string_tag: Persistent<Function<'static>>,
}

// SAFETY: `Bridge` holds only `'static` persistents — no `'js` lifetime
// derives from it.
unsafe impl JsLifetime<'_> for Bridge {
    type Changed<'to> = Self;
}

/// The retained reference behind `JsFunction` and `JsObject`: the
/// `Persistent` plus the `Context` that produced it, so a handle keeps its
/// context — and the `Runtime` the context owns — alive instead of aborting
/// in `JS_FreeRuntime`.
struct Retained {
    /// The persistent handle; declared before `context` so it drops while
    /// the context is still alive.
    value: Persistent<Value<'static>>,
    /// Keeps the producing context alive for as long as the handle lives.
    context: Context,
}

/// The QuickJS-NG engine.
///
/// `!Send + !Sync`: a `QuickJS` context is bound to the thread that created it.
pub struct QuickJsRuntime {
    /// Owns the underlying `Runtime` — `Context` keeps it alive.
    context: Context,
}

impl QuickJsRuntime {
    /// The `__waterui_host` namespace object, created empty at construction.
    fn host_namespace<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>, JsError> {
        ctx.globals()
            .get::<_, Object<'_>>(HOST_NAMESPACE)
            .map_err(|error| map_error(ctx, &error))
    }

    /// Whether `object` is plain: `Object.prototype` or `null` prototype.
    /// Everything else — `Map`, `Date`, `Error`, `Promise`, class instances —
    /// is an exotic that has no `JsValue`.
    fn is_plain<'js>(ctx: &Ctx<'js>, object: &Object<'js>) -> Result<bool, JsError> {
        let Some(prototype) = object.get_prototype() else {
            return Ok(true);
        };
        let bridge = ctx
            .userdata::<Bridge>()
            .ok_or_else(|| JsError::new("Error", "the context lost its bridge userdata"))?;
        let object_prototype = bridge
            .object_prototype
            .clone()
            .restore(ctx)
            .map_err(|error| map_error(ctx, &error))?;
        Ok(prototype.as_value() == object_prototype.as_value())
    }

    /// `Object.prototype.toString.call(value)` → `"Map"`, `"Date"`, … — the
    /// kind a conversion error names.
    fn kind_name<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> String {
        let tagged = ctx.userdata::<Bridge>().and_then(|bridge| {
            let to_string = bridge.to_string_tag.clone().restore(ctx).ok()?;
            to_string
                .call::<_, String>((This(value.clone()),))
                .map_or_else(
                    |_| {
                        // Drain the pending exception so it cannot leak into
                        // the next engine operation.
                        let _ = ctx.catch();
                        None
                    },
                    Some,
                )
        });
        tagged
            .as_deref()
            .and_then(|tag| tag.strip_prefix("[object "))
            .and_then(|tag| tag.strip_suffix(']'))
            .map_or_else(|| value.type_name().to_owned(), str::to_owned)
    }

    /// Rust → JavaScript.
    fn to_js<'js>(ctx: &Ctx<'js>, value: &JsValue, depth: usize) -> Result<Value<'js>, JsError> {
        if depth > MAX_CONVERSION_DEPTH {
            return Err(JsError::conversion(
                "a value graph deeper than the conversion limit cannot cross the bridge",
            ));
        }
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
                        .set(index, Self::to_js(ctx, item, depth + 1)?)
                        .map_err(fail)?;
                }
                array.into_value()
            }
            JsValue::Object(entries) => {
                let object = Object::new(ctx.clone()).map_err(fail)?;
                for (key, item) in entries {
                    object
                        .set(key.as_str(), Self::to_js(ctx, item, depth + 1)?)
                        .map_err(fail)?;
                }
                object.into_value()
            }
            JsValue::Function(function) => restore(ctx, function.handle())?,
            JsValue::ObjectRef(object) => restore(ctx, object.handle())?,
            JsValue::Opaque(opaque) => Class::instance(
                ctx.clone(),
                OpaqueBox {
                    value: opaque.inner().clone(),
                },
            )
            .map_err(fail)?
            .into_value(),
        })
    }

    /// JavaScript → Rust. `context` is the `Context` behind `ctx`; a
    /// converted function retains it so its handle outlives the runtime.
    fn from_js<'js>(
        ctx: &Ctx<'js>,
        context: &Context,
        value: &Value<'js>,
        depth: usize,
    ) -> Result<JsValue, JsError> {
        if depth > MAX_CONVERSION_DEPTH {
            return Err(JsError::conversion(
                "an object graph deeper than the conversion limit cannot cross the bridge",
            ));
        }
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
                retain_value(ctx, context, value.clone()),
            )),
            Type::Array => {
                let array = value
                    .clone()
                    .into_array()
                    .ok_or_else(|| JsError::conversion("an array-typed value is not an array"))?;
                let mut items = Vec::with_capacity(array.len());
                for item in array.iter::<Value<'js>>() {
                    items.push(Self::from_js(
                        ctx,
                        context,
                        &item.map_err(fail)?,
                        depth + 1,
                    )?);
                }
                JsValue::Array(items)
            }
            Type::Object | Type::Exception | Type::Promise | Type::Proxy => {
                let object = value
                    .clone()
                    .into_object()
                    .ok_or_else(|| JsError::conversion("an object-typed value is not an object"))?;
                if let Some(class) = Class::<OpaqueBox>::from_object(&object) {
                    return Ok(JsValue::Opaque(Opaque::from_inner(
                        class.borrow().value.clone(),
                    )));
                }
                if !Self::is_plain(ctx, &object)? {
                    return Err(JsError::conversion(format!(
                        "a {} cannot cross the bridge",
                        Self::kind_name(ctx, value)
                    )));
                }
                let mut entries = Vec::with_capacity(object.len());
                for pair in object.props::<String, Value<'js>>() {
                    let (key, item) = pair.map_err(fail)?;
                    entries.push((key, Self::from_js(ctx, context, &item, depth + 1)?));
                }
                JsValue::Object(entries)
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

/// Wraps `value` in a handle that keeps its context alive.
fn retain_value<'js>(ctx: &Ctx<'js>, context: &Context, value: Value<'js>) -> Handle {
    Handle::new(Retained {
        value: Persistent::save(ctx, value),
        context: context.clone(),
    })
}

/// Builds the JavaScript-side host function. A free function so the closure's
/// `ctx` and `args` can share one named `'js` — `Value` is invariant over it,
/// so two `'_` holes would never unify.
fn make_host_function<'js>(
    ctx: &Ctx<'js>,
    host: HostFunction,
    context: Context,
) -> Result<Function<'js>, JsError> {
    Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, Rest(args): Rest<Value<'js>>| host_call(&ctx, &context, &args, &host),
    )
    .map_err(|error| map_error(ctx, &error))
}

/// Runs one host-function call: converts the arguments, calls `host`,
/// converts (or throws) the result.
fn host_call<'js>(
    ctx: &Ctx<'js>,
    context: &Context,
    args: &[Value<'js>],
    host: &HostFunction,
) -> rquickjs::Result<Value<'js>> {
    let mut converted = Vec::with_capacity(args.len());
    for arg in args {
        match QuickJsRuntime::from_js(ctx, context, arg, 0) {
            Ok(value) => converted.push(value),
            Err(error) => return Err(throw(ctx, &error)),
        }
    }
    match host(&converted) {
        Ok(value) => QuickJsRuntime::to_js(ctx, &value, 0).map_err(|error| throw(ctx, &error)),
        Err(error) => Err(throw(ctx, &error)),
    }
}

/// Restores a retained handle to a live value of `ctx`.
fn restore<'js>(ctx: &Ctx<'js>, handle: &Handle) -> Result<Value<'js>, JsError> {
    let retained = handle
        .downcast::<Retained>()
        .ok_or_else(|| JsError::conversion("a handle from a different engine was passed in"))?;
    // A handle restores only into the context that produced it.
    let same_context = retained.context.as_raw() == ctx.as_raw();
    if !same_context {
        return Err(JsError::conversion(
            "a handle from a different QuickJS context was passed in",
        ));
    }
    retained
        .value
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
        context.with(|ctx| {
            ctx.globals()
                .set(
                    HOST_NAMESPACE,
                    Object::new(ctx.clone()).map_err(|e| map_error(&ctx, &e))?,
                )
                .map_err(|error| map_error(&ctx, &error))?;
            // `Object.prototype` and its `toString` are fetched once so a
            // script rewriting `globalThis.Object` cannot change what counts
            // as plain or how exotics are named.
            let object_prototype: Object<'_> = ctx
                .globals()
                .get::<_, Object<'_>>("Object")
                .and_then(|ctor| ctor.get("prototype"))
                .map_err(|error| map_error(&ctx, &error))?;
            let to_string_tag: Function<'_> = object_prototype
                .get("toString")
                .map_err(|error| map_error(&ctx, &error))?;
            ctx.store_userdata(Bridge {
                object_prototype: Persistent::save(&ctx, object_prototype),
                to_string_tag: Persistent::save(&ctx, to_string_tag),
            })
            .map_err(|_| JsError::new("Error", "the context rejected its bridge userdata"))?;
            Ok(())
        })?;
        Ok(Self { context })
    }

    fn eval(&self, source: &str, name: &str) -> Result<JsValue, JsError> {
        self.context.with(|ctx| {
            // Classic scripts are sloppy — matching JavaScriptCore; a bundle
            // opts in with its own `"use strict"` directive.
            let mut options = EvalOptions::default();
            options.filename = Some(name.to_owned());
            options.strict = false;
            let value = ctx
                .eval_with_options::<Value<'_>, Vec<u8>>(source.into(), options)
                .map_err(|error| map_error(&ctx, &error))?;
            Self::from_js(&ctx, &self.context, &value, 0)
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
                    .push_arg(Self::to_js(&ctx, arg, 0)?)
                    .map_err(|error| map_error(&ctx, &error))?;
            }
            let value = function
                .call_arg::<Value<'_>>(packed)
                .map_err(|error| map_error(&ctx, &error))?;
            Self::from_js(&ctx, &self.context, &value, 0)
        })
    }

    fn register(
        &self,
        name: &str,
        function: impl Fn(&[JsValue]) -> Result<JsValue, JsError> + 'static,
    ) -> Result<(), JsError> {
        let host: HostFunction = Rc::new(function);
        let context = self.context.clone();
        self.context.with(|ctx| {
            let function = make_host_function(&ctx, host, context)?;
            Self::host_namespace(&ctx)?
                .set(name, function)
                .map_err(|error| map_error(&ctx, &error))
        })
    }

    fn retain(&self, value: &JsValue) -> Result<JsObject, JsError> {
        self.context.with(|ctx| match value {
            JsValue::Object(_) | JsValue::ObjectRef(_) | JsValue::Function(_) => {
                let value = Self::to_js(&ctx, value, 0)?;
                Ok(JsObject::from_handle(retain_value(
                    &ctx,
                    &self.context,
                    value,
                )))
            }
            _ => Err(JsError::conversion(
                "only JavaScript objects and functions can be retained",
            )),
        })
    }

    fn collect_garbage(&self) {
        self.context.with(|ctx| ctx.run_gc());
    }
}

impl fmt::Debug for QuickJsRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("QuickJsRuntime(..)")
    }
}
