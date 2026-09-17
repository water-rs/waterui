//! `JsRuntime` over the system `JavaScriptCore.framework`.
//!
//! One [`JscRuntime`] owns one `JSContext`. Retained handles carry
//! `Retained<JSValue>`; opaque Rust values cross as a JavaScript object
//! carrying a symbol-keyed index into a registry the runtime owns —
//! JavaScript can hold and pass the box but cannot reach the `Rc`.

use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::ptr;
use std::rc::Rc;

use block2::{ManualBlockEncoding, RcBlock};
use objc2::Message;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSArray, NSString, NSURL};
use objc2_javascript_core::{JSContext, JSValue};
use waterui_ts_engine::handle::Handle;
use waterui_ts_engine::{
    BigInt, HostFunction, JsError, JsFunction, JsObject, JsRuntime, JsValue, Opaque,
};

/// The namespace object host functions register under:
/// `globalThis.__waterui_host`.
const HOST_NAMESPACE: &str = "__waterui_host";

/// The opaque registry: `Rc`s boxed for JavaScript to hold.
type Registry = Rc<RefCell<Vec<Rc<dyn Any>>>>;

/// `text` as an `NSString`.
fn ns(text: &str) -> Retained<NSString> {
    NSString::from_str(text)
}

/// `object` as `id`, for `JavaScriptCore`'s `AnyObject` parameters.
fn as_id<T: Message>(object: &Retained<T>) -> &AnyObject {
    // SAFETY: `AnyObject` is `id`, and `T: Message` is an Objective-C object
    // type, so the object `object` retains is already a valid `id`; the cast
    // only retypes the same pointer for the duration of the borrow.
    unsafe { &*ptr::from_ref(&**object).cast::<AnyObject>() }
}

/// A `JSValue`'s string form, or `None` when conversion throws (which leaves
/// a pending exception the next engine operation clears).
fn string(value: &JSValue) -> Option<String> {
    // SAFETY: `value` is a live `JSValue`; a failed `toString` only leaves a
    // pending exception, which callers drain before the next operation.
    unsafe { value.toString() }.map(|text| text.to_string())
}

/// A live `undefined` in `context`.
fn undefined(context: &JSContext) -> Retained<JSValue> {
    // SAFETY: `context` is a live context.
    unsafe { JSValue::valueWithUndefinedInContext(Some(context)) }
        .expect("JavaScriptCore always produces `undefined`")
}

/// Throws `error` into JavaScript: sets the context's pending exception to an
/// `Error` carrying the message (and class name) and returns `undefined`,
/// which `JavaScriptCore` discards in favour of the pending exception.
fn throw(context: &JSContext, error: &JsError) -> Retained<JSValue> {
    // SAFETY: `context` is a live context.
    if let Some(exception) = unsafe {
        JSValue::valueWithNewErrorFromMessage_inContext(Some(&ns(&error.message)), Some(context))
    } {
        if !error.name.is_empty() && error.name != "Error" {
            // SAFETY: `exception` is a live `JSValue` in `context`.
            unsafe {
                exception.setValue_forProperty(Some(as_id(&ns(&error.name))), Some(&ns("name")));
            }
        }
        // SAFETY: `context` is a live context and `exception` its value.
        unsafe { context.setException(Some(&exception)) };
    }
    undefined(context)
}

/// A thrown `JSValue` as a `JsError`: `name`, `message` and `stack` for an
/// `Error` object, the coerced value for anything else.
fn exception_error(exception: &JSValue) -> JsError {
    // SAFETY: `exception` is a live `JSValue`.
    if unsafe { exception.isObject() } {
        JsError {
            name: property_string(exception, "name").unwrap_or_else(|| String::from("Error")),
            message: property_string(exception, "message").unwrap_or_default(),
            stack: property_string(exception, "stack"),
        }
    } else {
        JsError::new("Error", string(exception).unwrap_or_default())
    }
}

/// A string property of a `JSValue`, or `None` when it is absent, `null` or
/// `undefined`.
fn property_string(value: &JSValue, key: &str) -> Option<String> {
    // SAFETY: `value` is a live `JSValue`; the property it returns is a live
    // `JSValue` for as long as `value` lives.
    let property = unsafe { value.valueForProperty(Some(&ns(key))) }?;
    // SAFETY: `property` is a live `JSValue`.
    if unsafe { property.isUndefined() } || unsafe { property.isNull() } {
        return None;
    }
    string(&property)
}

/// The block signature `id (^)(void)` — `RcBlock::new` attaches no
/// signature, and `JavaScriptCore` only converts a block to a JavaScript
/// function when `_Block_signature` parses, so the encoding is declared
/// explicitly.
struct HostBlockEncoding;

// SAFETY: `@8@?0` (`@4@?0` on 32-bit) is the correct signature for a block
// taking no declared parameters and returning an object pointer;
// `*mut JSValue` encodes as `@`.
unsafe impl ManualBlockEncoding for HostBlockEncoding {
    type Arguments = ();
    type Return = *mut JSValue;
    const ENCODING_CSTR: &'static core::ffi::CStr = if cfg!(target_pointer_width = "64") {
        cr"@8@?0"
    } else {
        cr"@4@?0"
    };
}

/// The pieces a host-function block needs to convert values in both
/// directions, cloned into each block.
#[derive(Clone)]
struct Bridge {
    /// `{ keys, isFunction }` — helpers the Objective-C API lacks. Held, not
    /// installed on the global object, so they never leak into the script's
    /// namespace.
    helpers: Retained<JSValue>,
    /// The symbol marking a boxed [`Opaque`] on its JavaScript object.
    opaque_key: Retained<JSValue>,
    registry: Registry,
}

impl Bridge {
    /// `value` as an owned `id`, the element type `callWithArguments` and
    /// friends expect.
    fn as_argument(value: &JSValue) -> Retained<AnyObject> {
        // SAFETY: `value` is an Objective-C object; `AnyObject` is `id`, so
        // retyping keeps the same live object.
        unsafe { Retained::cast_unchecked::<AnyObject>(value.retain()) }
    }

    /// Rust → JavaScript.
    fn to_js(&self, context: &JSContext, value: &JsValue) -> Result<Retained<JSValue>, JsError> {
        let created = match value {
            JsValue::Undefined => Some(undefined(context)),
            // SAFETY: `context` is a live context.
            JsValue::Null => unsafe { JSValue::valueWithNullInContext(Some(context)) },
            // SAFETY: `context` is a live context.
            JsValue::Bool(value) => unsafe {
                JSValue::valueWithBool_inContext(*value, Some(context))
            },
            // SAFETY: `context` is a live context.
            JsValue::Number(value) => unsafe {
                JSValue::valueWithDouble_inContext(*value, Some(context))
            },
            // SAFETY: `context` is a live context.
            JsValue::BigInt(BigInt::Signed(value)) => unsafe {
                JSValue::valueWithNewBigIntFromInt64_inContext(*value, context)
            },
            // SAFETY: `context` is a live context.
            JsValue::BigInt(BigInt::Unsigned(value)) => unsafe {
                JSValue::valueWithNewBigIntFromUInt64_inContext(*value, context)
            },
            // SAFETY: `context` is a live context and the `NSString` a live
            // Objective-C object.
            JsValue::String(value) => unsafe {
                JSValue::valueWithObject_inContext(Some(as_id(&ns(value))), Some(context))
            },
            JsValue::Array(items) => {
                // SAFETY: `context` is a live context.
                let array = unsafe { JSValue::valueWithNewArrayInContext(Some(context)) };
                if let Some(array) = &array {
                    for (index, item) in items.iter().enumerate() {
                        let item = self.to_js(context, item)?;
                        // SAFETY: `array` and `item` are live `JSValue`s.
                        unsafe {
                            array.setObject_atIndexedSubscript(Some(as_id(&item)), index);
                        }
                    }
                }
                array
            }
            JsValue::Object(entries) => {
                // SAFETY: `context` is a live context.
                let object = unsafe { JSValue::valueWithNewObjectInContext(Some(context)) };
                if let Some(object) = &object {
                    for (key, item) in entries {
                        let item = self.to_js(context, item)?;
                        // SAFETY: `object` and `item` are live `JSValue`s;
                        // the key is a live `NSString`, converted to a JS
                        // string.
                        unsafe {
                            object.setObject_forKeyedSubscript(
                                Some(as_id(&item)),
                                Some(as_id(&ns(key))),
                            );
                        }
                    }
                }
                object
            }
            JsValue::Function(function) => Some(restore(function.handle())?),
            JsValue::ObjectRef(object) => Some(restore(object.handle())?),
            JsValue::Opaque(opaque) => Some(self.box_opaque(context, opaque)?),
        };
        created
            .ok_or_else(|| JsError::new("Error", "JavaScriptCore could not materialize the value"))
    }

    /// JavaScript → Rust.
    fn to_rust(&self, value: &JSValue) -> Result<JsValue, JsError> {
        // SAFETY: every predicate and conversion in this block is called on
        // the live `value` or on a `JSValue` it produced.
        unsafe {
            if value.isUndefined() {
                return Ok(JsValue::Undefined);
            }
            if value.isNull() {
                return Ok(JsValue::Null);
            }
            if value.isBoolean() {
                return Ok(JsValue::Bool(value.toBool()));
            }
            if value.isNumber() {
                return Ok(JsValue::Number(value.toDouble()));
            }
            if value.isString() {
                return Ok(JsValue::String(string(value).unwrap_or_default()));
            }
            if value.isBigInt() {
                // `bigint.toString()` spells the exact integer; a `bigint`
                // beyond `u64` has no `JsValue`.
                let digits = string(value).unwrap_or_default();
                if let Ok(value) = digits.parse::<i64>() {
                    return Ok(JsValue::BigInt(BigInt::Signed(value)));
                }
                if let Ok(value) = digits.parse::<u64>() {
                    return Ok(JsValue::BigInt(BigInt::Unsigned(value)));
                }
                return Err(JsError::conversion(format!(
                    "a bigint beyond u64 cannot cross the bridge: {digits}"
                )));
            }
            if value.isSymbol() {
                return Err(JsError::conversion("a symbol cannot cross the bridge"));
            }
            if value.isObject() {
                // Functions are objects; check them first so they stay
                // callable rather than collapsing into entries.
                if self.is_function(value)? {
                    return Ok(JsValue::Function(JsFunction::from_handle(Handle::new(
                        value.retain(),
                    ))));
                }
                if value.isArray() {
                    let length = value
                        .valueForProperty(Some(&ns("length")))
                        .and_then(|length| usize::try_from(length.toInt64()).ok())
                        .unwrap_or(0);
                    let mut items = Vec::with_capacity(length);
                    for index in 0..length {
                        let item = value.objectAtIndexedSubscript(index).ok_or_else(|| {
                            JsError::conversion("an array element could not be read")
                        })?;
                        items.push(self.to_rust(&item)?);
                    }
                    return Ok(JsValue::Array(items));
                }
                if let Some(opaque) = self.unbox(value)? {
                    return Ok(JsValue::Opaque(opaque));
                }
                let mut entries = Vec::new();
                for key in self.keys(value)? {
                    let item = value
                        .valueForProperty(Some(&ns(&key)))
                        .ok_or_else(|| JsError::conversion("an object entry could not be read"))?;
                    entries.push((key, self.to_rust(&item)?));
                }
                return Ok(JsValue::Object(entries));
            }
        }
        Err(JsError::conversion(
            "a value of unknown type cannot cross the bridge",
        ))
    }

    /// `typeof value === 'function'`, which the Objective-C API cannot answer
    /// (`isObject` is true for functions too).
    fn is_function(&self, value: &JSValue) -> Result<bool, JsError> {
        let arguments = NSArray::from_retained_slice(&[Self::as_argument(value)]);
        // SAFETY: `helpers` is a live `JSValue` holding an `isFunction`
        // function, and `arguments` holds a `JSValue` as `id`.
        let result = unsafe {
            self.helpers
                .invokeMethod_withArguments(Some(&ns("isFunction")), Some(&arguments))
        }
        .ok_or_else(|| JsError::new("Error", "the isFunction helper call failed"))?;
        // SAFETY: `result` is a live `JSValue`.
        Ok(unsafe { result.isBoolean() && result.toBool() })
    }

    /// `Object.keys(value)` — the engine has no Objective-C API for it.
    fn keys(&self, value: &JSValue) -> Result<Vec<String>, JsError> {
        let arguments = NSArray::from_retained_slice(&[Self::as_argument(value)]);
        // SAFETY: `helpers` holds a `keys` function and `arguments` a
        // `JSValue` as `id`.
        let keys = unsafe {
            self.helpers
                .invokeMethod_withArguments(Some(&ns("keys")), Some(&arguments))
        }
        .ok_or_else(|| JsError::new("Error", "the keys helper call failed"))?;
        // SAFETY: `keys` is a live `JSValue` — an array of strings.
        unsafe {
            let length = keys
                .valueForProperty(Some(&ns("length")))
                .and_then(|length| usize::try_from(length.toInt64()).ok())
                .unwrap_or(0);
            let mut names = Vec::with_capacity(length);
            for index in 0..length {
                let name = keys
                    .objectAtIndexedSubscript(index)
                    .and_then(|name| string(&name))
                    .ok_or_else(|| JsError::conversion("an object key could not be read"))?;
                names.push(name);
            }
            Ok(names)
        }
    }

    /// Boxes `opaque` as a JavaScript object carrying its registry index
    /// under the unguessable symbol key.
    fn box_opaque(
        &self,
        context: &JSContext,
        opaque: &Opaque,
    ) -> Result<Retained<JSValue>, JsError> {
        let index = {
            let mut registry = self.registry.borrow_mut();
            registry.push(opaque.inner().clone());
            registry.len() - 1
        };
        // SAFETY: `context` is a live context.
        let boxed = unsafe { JSValue::valueWithNewObjectInContext(Some(context)) }
            .ok_or_else(|| JsError::new("Error", "JavaScriptCore could not create an object"))?;
        #[expect(
            clippy::cast_precision_loss,
            reason = "a registry index is bounded by the number of boxed values, always exact in a double"
        )]
        let index = index as f64;
        // SAFETY: `context` is a live context.
        let marker = unsafe { JSValue::valueWithDouble_inContext(index, Some(context)) }
            .ok_or_else(|| JsError::new("Error", "JavaScriptCore could not create a number"))?;
        // SAFETY: `boxed`, `marker` and `opaque_key` are live `JSValue`s.
        unsafe {
            boxed.setObject_forKeyedSubscript(Some(as_id(&marker)), Some(as_id(&self.opaque_key)));
        }
        Ok(boxed)
    }

    /// The `Rc` a boxed object points at, or `None` when it is not a box.
    fn unbox(&self, value: &JSValue) -> Result<Option<Opaque>, JsError> {
        // SAFETY: `value` and `opaque_key` are live `JSValue`s.
        let marker = unsafe { value.objectForKeyedSubscript(Some(as_id(&self.opaque_key))) };
        let Some(marker) = marker else {
            return Ok(None);
        };
        // SAFETY: `marker` is a live `JSValue`.
        if unsafe { marker.isUndefined() } {
            return Ok(None);
        }
        // SAFETY: `marker` is a live `JSValue` holding the registry index.
        let index = unsafe { marker.toInt64() };
        let boxed = usize::try_from(index)
            .ok()
            .and_then(|index| self.registry.borrow().get(index).cloned())
            .ok_or_else(|| {
                JsError::conversion("an opaque handle names a slot that does not exist")
            })?;
        Ok(Some(Opaque::from_inner(boxed)))
    }
}

/// Restores a retained handle to the engine's own `Retained<JSValue>`.
fn restore(handle: &Handle) -> Result<Retained<JSValue>, JsError> {
    handle
        .downcast::<Retained<JSValue>>()
        .map(|retained| (*retained).clone())
        .ok_or_else(|| JsError::conversion("a handle from a different engine was passed in"))
}

/// The `JavaScriptCore` engine.
///
/// `!Send + !Sync`: a `JSContext` is bound to the thread that created it, and
/// the registry `Rc` pins the struct to it too.
pub struct JscRuntime {
    context: Retained<JSContext>,
    bridge: Bridge,
}

impl JscRuntime {
    /// Drains the context's pending exception into a `JsError`.
    fn take_exception(&self) -> Option<JsError> {
        // SAFETY: `context` is a live context.
        let exception = unsafe { self.context.exception() }?;
        // SAFETY: clears the pending exception just captured.
        unsafe { self.context.setException(None) };
        Some(exception_error(&exception))
    }
}

impl JsRuntime for JscRuntime {
    fn new() -> Result<Self, JsError>
    where
        Self: Sized,
    {
        // SAFETY: `JSContext::new` creates a fresh context on this thread.
        let context = unsafe { JSContext::new() };
        // SAFETY: `context` is a live context.
        let global = unsafe { context.globalObject() }
            .ok_or_else(|| JsError::new("Error", "JavaScriptCore created no global object"))?;
        // SAFETY: `context` is a live context.
        let host = unsafe { JSValue::valueWithNewObjectInContext(Some(&context)) }
            .ok_or_else(|| JsError::new("Error", "JavaScriptCore could not create an object"))?;
        // SAFETY: `global` and `host` are live `JSValue`s; the key `NSString`
        // converts to a JS string.
        unsafe {
            global
                .setObject_forKeyedSubscript(Some(as_id(&host)), Some(as_id(&ns(HOST_NAMESPACE))));
        }
        // `keys` and `isFunction` have no Objective-C API; one constant,
        // known-good expression installs them.
        // SAFETY: `context` is a live context.
        let helpers = unsafe {
            context.evaluateScript(Some(&ns("Object.freeze({\
                    keys: (object) => Object.keys(object),\
                    isFunction: (value) => typeof value === 'function'\
                })")))
        }
        .ok_or_else(|| JsError::new("Error", "JavaScriptCore could not create the helpers"))?;
        // SAFETY: `context` is a live context.
        let opaque_key = unsafe {
            JSValue::valueWithNewSymbolFromDescription_inContext(
                Some(&ns("waterui.opaque")),
                Some(&context),
            )
        }
        .ok_or_else(|| JsError::new("Error", "JavaScriptCore could not create a symbol"))?;
        Ok(Self {
            context,
            bridge: Bridge {
                helpers,
                opaque_key,
                registry: Registry::default(),
            },
        })
    }

    fn eval(&self, source: &str, name: &str) -> Result<JsValue, JsError> {
        // SAFETY: clears any stale pending exception before evaluating.
        unsafe { self.context.setException(None) };
        let script = ns(source);
        // `name` is an arbitrary string; `URLWithString` returns `None` for
        // one it cannot parse, which `evaluateScript` accepts.
        let url = NSURL::URLWithString(&ns(name));
        // SAFETY: `context` is a live context and `script` a live `NSString`.
        let result = unsafe {
            self.context
                .evaluateScript_withSourceURL(Some(&script), url.as_deref())
        };
        if let Some(exception) = self.take_exception() {
            return Err(exception);
        }
        let result = result.ok_or_else(|| {
            JsError::new("Error", "evaluation produced no value and no exception")
        })?;
        self.bridge.to_rust(&result)
    }

    fn call(&self, function: &JsFunction, args: &[JsValue]) -> Result<JsValue, JsError> {
        let function = restore(function.handle())?;
        let mut marshaled = Vec::with_capacity(args.len());
        for arg in args {
            let arg = self.bridge.to_js(&self.context, arg)?;
            marshaled.push(Bridge::as_argument(&arg));
        }
        let marshaled = NSArray::from_retained_slice(&marshaled);
        // SAFETY: clears any stale pending exception before calling.
        unsafe { self.context.setException(None) };
        // SAFETY: `function` is a live `JSValue` in `context` and `argv`
        // holds `JSValue`s as `id`.
        let result = unsafe { function.callWithArguments(Some(&marshaled)) };
        if let Some(exception) = self.take_exception() {
            return Err(exception);
        }
        let result = result
            .ok_or_else(|| JsError::new("Error", "the call produced no value and no exception"))?;
        self.bridge.to_rust(&result)
    }

    fn register(
        &self,
        name: &str,
        function: impl Fn(&[JsValue]) -> Result<JsValue, JsError> + 'static,
    ) -> Result<(), JsError> {
        let host: HostFunction = Rc::new(function);
        let bridge = self.bridge.clone();
        // JavaScriptCore marshals declared block parameters from the call;
        // a zero-parameter block instead takes the full argument list
        // through `+[JSContext currentArguments]`.
        let block = RcBlock::<dyn Fn() -> *mut JSValue>::with_encoding::<
            (),
            *mut JSValue,
            _,
            HostBlockEncoding,
        >(move || -> *mut JSValue {
            // SAFETY: the block only runs inside a JavaScriptCore callback,
            // where `currentContext`/`currentArguments` describe that call.
            let context = unsafe { JSContext::currentContext() }
                .expect("a host function runs inside a JavaScriptCore callback");
            // SAFETY: same callback context — `currentArguments` describes
            // this call's argument list.
            let arguments = unsafe { JSContext::currentArguments() };
            let mut converted = Vec::new();
            if let Some(arguments) = arguments {
                for index in 0..arguments.count() {
                    let object = arguments.objectAtIndex(index);
                    let value = object.downcast_ref::<JSValue>().map_or_else(
                        || {
                            // SAFETY: `object` is a live Objective-C object
                            // and `context` its context.
                            unsafe {
                                JSValue::valueWithObject_inContext(Some(&object), Some(&context))
                            }
                            .unwrap_or_else(|| undefined(&context))
                        },
                        objc2::Message::retain,
                    );
                    match bridge.to_rust(&value) {
                        Ok(value) => converted.push(value),
                        Err(error) => {
                            return Retained::autorelease_return(throw(&context, &error));
                        }
                    }
                }
            }
            // `autorelease_return`: a block returns its object at +0, the
            // same convention `valueWithObject_inContext` callers follow.
            match host(&converted) {
                Ok(value) => match bridge.to_js(&context, &value) {
                    Ok(value) => Retained::autorelease_return(value),
                    Err(error) => Retained::autorelease_return(throw(&context, &error)),
                },
                Err(error) => Retained::autorelease_return(throw(&context, &error)),
            }
        });
        // SAFETY: `context` is a live context.
        let global = unsafe { self.context.globalObject() }
            .ok_or_else(|| JsError::new("Error", "JavaScriptCore created no global object"))?;
        // SAFETY: `global` is a live `JSValue`; the key `NSString` converts
        // to a JS string.
        let namespace = unsafe { global.objectForKeyedSubscript(Some(as_id(&ns(HOST_NAMESPACE)))) }
            .ok_or_else(|| JsError::new("Error", "the __waterui_host namespace is missing"))?;
        // SAFETY: a heap block is an Objective-C object (`NSBlock`), a valid
        // `id`; JavaScriptCore retains it as the function it installs, so it
        // outlives this `RcBlock`.
        unsafe {
            namespace.setObject_forKeyedSubscript(
                Some(
                    &*ptr::from_ref::<block2::Block<dyn Fn() -> *mut JSValue>>(&*block)
                        .cast::<AnyObject>(),
                ),
                Some(as_id(&ns(name))),
            );
        }
        Ok(())
    }

    fn retain(&self, value: &JsValue) -> Result<JsObject, JsError> {
        match value {
            JsValue::Object(_) | JsValue::ObjectRef(_) | JsValue::Function(_) => {
                let value = self.bridge.to_js(&self.context, value)?;
                Ok(JsObject::from_handle(Handle::new(value)))
            }
            _ => Err(JsError::conversion(
                "only JavaScript objects and functions can be retained",
            )),
        }
    }
}

impl fmt::Debug for JscRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JscRuntime(..)")
    }
}
