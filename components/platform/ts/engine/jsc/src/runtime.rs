//! `JsRuntime` over the system `JavaScriptCore.framework`.
//!
//! One [`JscRuntime`] owns one `JSContext`. Retained handles carry
//! `Retained<JSValue>` — a `JSValue` keeps its context alive, so a handle
//! outlives the runtime safely. Opaque Rust values cross as instances of a
//! private `JSClass` whose objects hold the `Rc` as private data and release
//! it in the class finalizer; no JavaScript-visible property names the
//! identity.

use std::any::Any;
use std::ffi::c_void;
use std::fmt;
use std::ptr;
use std::rc::Rc;

use block2::{ManualBlockEncoding, RcBlock};
use objc2::Message;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_foundation::{NSArray, NSString, NSURL};
use objc2_javascript_core::{
    JSClassCreate, JSClassDefinition, JSClassRef, JSClassRelease, JSContext, JSObjectGetPrivate,
    JSObjectMake, JSObjectRef, JSValue, kJSClassAttributeNone,
};
use waterui_ts_engine::handle::Handle;
use waterui_ts_engine::{
    BigInt, HostFunction, JsError, JsFunction, JsObject, JsRuntime, JsValue, MAX_CONVERSION_DEPTH,
    Opaque,
};

/// The namespace object host functions register under:
/// `globalThis.__waterui_host`.
const HOST_NAMESPACE: &str = "__waterui_host";

unsafe extern "C-unwind" {
    /// `JSSynchronousGarbageCollectForDebugging` — exported by the framework
    /// and declared in `JSBase.h`; runs a full synchronous collection,
    /// including conservative-stack and external-reference scanning, where
    /// `JSGarbageCollect` can defer finalization.
    fn JSSynchronousGarbageCollectForDebugging(ctx: objc2_javascript_core::JSContextRef);
}

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

/// `JSObjectFinalizeCallback` for the opaque-box class: takes the
/// `Box<Rc<dyn Any>>` the object carried back and drops it — a box
/// JavaScript drops releases its `Rc`. The callback must not allocate or
/// collect; dropping a `Box` does neither.
unsafe extern "C-unwind" fn opaque_finalize(object: JSObjectRef) {
    // SAFETY: `box_opaque` stored a `Box<Rc<dyn Any>>` pointer as the
    // object's private data, and `finalize` runs exactly once per object.
    let data = unsafe { JSObjectGetPrivate(object) };
    if !data.is_null() {
        // SAFETY: `data` is the unique `Box` pointer stored at creation.
        unsafe { drop(Box::from_raw(data.cast::<Rc<dyn Any>>())) };
    }
}

/// The class definition for opaque boxes: every callback `None` except
/// `finalize`, so the boxes expose no JavaScript-visible surface — no
/// property, no name lookup, no constructor. `JSClassCreate` reads it once;
/// it never outlives the call.
fn opaque_class_definition() -> JSClassDefinition {
    JSClassDefinition {
        version: 0,
        attributes: kJSClassAttributeNone,
        className: c"WaterUiOpaque".as_ptr(),
        parentClass: ptr::null_mut(),
        staticValues: ptr::null(),
        staticFunctions: ptr::null(),
        initialize: None,
        finalize: Some(opaque_finalize),
        hasProperty: None,
        getProperty: None,
        setProperty: None,
        deleteProperty: None,
        getPropertyNames: None,
        callAsFunction: None,
        callAsConstructor: None,
        hasInstance: None,
        convertToType: None,
    }
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
    /// `{ keys, isFunction, isPlainObject, kindName }` — helpers the
    /// Objective-C API lacks. Held, not installed on the global object, so
    /// they never leak into the script's namespace.
    helpers: Retained<JSValue>,
    /// The class identifying opaque boxes. A non-owning copy of the one ref
    /// `JscRuntime::new` created — the runtime releases it on drop, and live
    /// boxes keep the class alive through their own refs until then.
    opaque_class: JSClassRef,
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
    fn to_js(
        &self,
        context: &JSContext,
        value: &JsValue,
        depth: usize,
    ) -> Result<Retained<JSValue>, JsError> {
        if depth > MAX_CONVERSION_DEPTH {
            return Err(JsError::conversion(
                "a value graph deeper than the conversion limit cannot cross the bridge",
            ));
        }
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
                        let item = self.to_js(context, item, depth + 1)?;
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
                        let item = self.to_js(context, item, depth + 1)?;
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
    fn to_rust(
        &self,
        context: &JSContext,
        value: &JSValue,
        depth: usize,
    ) -> Result<JsValue, JsError> {
        if depth > MAX_CONVERSION_DEPTH {
            return Err(JsError::conversion(
                "an object graph deeper than the conversion limit cannot cross the bridge",
            ));
        }
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
                if self.is_function(context, value)? {
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
                        items.push(self.to_rust(context, &item, depth + 1)?);
                    }
                    return Ok(JsValue::Array(items));
                }
                if let Some(opaque) = self.unbox(context, value) {
                    return Ok(JsValue::Opaque(opaque));
                }
                if !self.is_plain_object(context, value)? {
                    return Err(JsError::conversion(format!(
                        "a {} cannot cross the bridge",
                        self.kind_name(context, value)
                    )));
                }
                let mut entries = Vec::new();
                for key in self.keys(context, value)? {
                    let item = value
                        .valueForProperty(Some(&ns(&key)))
                        .ok_or_else(|| JsError::conversion("an object entry could not be read"))?;
                    entries.push((key, self.to_rust(context, &item, depth + 1)?));
                }
                return Ok(JsValue::Object(entries));
            }
        }
        Err(JsError::conversion(
            "a value of unknown type cannot cross the bridge",
        ))
    }

    /// Calls the named helper on `value`. On failure any pending exception
    /// is drained so it cannot leak into the next engine operation.
    fn helper_call(
        &self,
        context: &JSContext,
        name: &str,
        value: &JSValue,
    ) -> Result<Retained<JSValue>, JsError> {
        let arguments = NSArray::from_retained_slice(&[Self::as_argument(value)]);
        // SAFETY: `helpers` is a live `JSValue` holding named functions, and
        // `arguments` holds a `JSValue` as `id`.
        let result = unsafe {
            self.helpers
                .invokeMethod_withArguments(Some(&ns(name)), Some(&arguments))
        };
        result.ok_or_else(|| {
            // SAFETY: `context` is a live context; drain anything pending.
            unsafe { context.setException(None) };
            JsError::new("Error", format!("the {name} helper call failed"))
        })
    }

    /// `typeof value === 'function'`, which the Objective-C API cannot answer
    /// (`isObject` is true for functions too).
    fn is_function(&self, context: &JSContext, value: &JSValue) -> Result<bool, JsError> {
        let result = self.helper_call(context, "isFunction", value)?;
        // SAFETY: `result` is a live `JSValue`.
        Ok(unsafe { result.isBoolean() && result.toBool() })
    }

    /// Whether `object` is plain: `Object.prototype` or `null` prototype.
    /// Everything else — `Map`, `Date`, `Error`, `Promise`, class instances —
    /// is an exotic that has no `JsValue`.
    fn is_plain_object(&self, context: &JSContext, value: &JSValue) -> Result<bool, JsError> {
        let result = self.helper_call(context, "isPlainObject", value)?;
        // SAFETY: `result` is a live `JSValue`.
        Ok(unsafe { result.isBoolean() && result.toBool() })
    }

    /// `Object.prototype.toString.call(value)` → `"Map"`, `"Date"`, … — the
    /// kind a conversion error names.
    fn kind_name(&self, context: &JSContext, value: &JSValue) -> String {
        self.helper_call(context, "kindName", value)
            .ok()
            .and_then(|result| string(&result))
            .unwrap_or_else(|| String::from("object"))
    }

    /// `Object.keys(value)` — the engine has no Objective-C API for it.
    fn keys(&self, context: &JSContext, value: &JSValue) -> Result<Vec<String>, JsError> {
        let keys = self.helper_call(context, "keys", value)?;
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

    /// Boxes `opaque` as an instance of the opaque class — the `Rc` is the
    /// object's private data, released by `opaque_finalize` when JavaScript
    /// drops the box. Each crossing creates a new box.
    fn box_opaque(
        &self,
        context: &JSContext,
        opaque: &Opaque,
    ) -> Result<Retained<JSValue>, JsError> {
        let data = Box::into_raw(Box::new(opaque.inner().clone()));
        // SAFETY: `context` is a live context and `opaque_class` a live
        // class; on success the object owns `data` until `opaque_finalize`
        // takes it back.
        let object = unsafe {
            JSObjectMake(
                context.JSGlobalContextRef().cast_const(),
                self.opaque_class,
                data.cast::<c_void>(),
            )
        };
        if object.is_null() {
            // SAFETY: `data` was not adopted; reclaim the `Box`.
            unsafe { drop(Box::from_raw(data)) };
            return Err(JsError::new(
                "Error",
                "JavaScriptCore could not box an opaque value",
            ));
        }
        // SAFETY: `object` is a live object in `context`; a `JSObjectRef`
        // doubles as its `JSValueRef`.
        unsafe { JSValue::valueWithJSValueRef_inContext(object.cast_const(), Some(context)) }
            .ok_or_else(|| JsError::new("Error", "JavaScriptCore could not wrap the opaque box"))
    }

    /// The `Rc` a box carries, or `None` when `value` is not a box. The
    /// check is engine-native — no JavaScript-visible property marks a box,
    /// so a plain object can never be mistaken for one.
    fn unbox(&self, context: &JSContext, value: &JSValue) -> Option<Opaque> {
        // SAFETY: `context` is a live context.
        let context_ref = unsafe { context.JSGlobalContextRef() }.cast_const();
        // SAFETY: `value` is a live `JSValue`.
        let value_ref = unsafe { value.JSValueRef() };
        // SAFETY: both refs are live and `opaque_class` is a live class.
        if !unsafe { JSValue::is_object_of_class(context_ref, value_ref, self.opaque_class) } {
            return None;
        }
        // SAFETY: the class check guarantees the private data is the
        // `Box<Rc<dyn Any>>` `box_opaque` stored.
        let data = unsafe { JSObjectGetPrivate(value_ref.cast_mut()) };
        if data.is_null() {
            return None;
        }
        // SAFETY: `data` is a `Box<Rc<dyn Any>>` still owned by the object;
        // it is only borrowed for the clone.
        let rc = unsafe { &*data.cast::<Rc<dyn Any>>() }.clone();
        Some(Opaque::from_inner(rc))
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
/// `!Send + !Sync`: a `JSContext` is bound to the thread that created it.
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

impl Drop for JscRuntime {
    fn drop(&mut self) {
        // SAFETY: `opaque_class` is the one owned ref `new` created; live
        // boxes keep the class alive through their own refs, so releasing
        // here cannot orphan a finalizer.
        unsafe { JSClassRelease(self.bridge.opaque_class) };
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
        // `keys`, `isFunction`, `isPlainObject` and `kindName` have no
        // Objective-C API; one constant, known-good expression installs them.
        // SAFETY: `context` is a live context.
        let helpers = unsafe {
            context.evaluateScript(Some(&ns("Object.freeze({\
                    keys: (object) => Object.keys(object),\
                    isFunction: (value) => typeof value === 'function',\
                    isPlainObject: (value) => {\
                        const prototype = Object.getPrototypeOf(value);\
                        return prototype === null || prototype === Object.prototype;\
                    },\
                    kindName: (value) => Object.prototype.toString.call(value).slice(8, -1)\
                })")))
        }
        .ok_or_else(|| JsError::new("Error", "JavaScriptCore could not create the helpers"))?;
        // The one owned ref to the opaque-box class; `JscRuntime::drop`
        // releases it.
        let definition = opaque_class_definition();
        // SAFETY: the definition is a valid struct that lives for the call.
        let opaque_class = unsafe { JSClassCreate(&raw const definition) };
        if opaque_class.is_null() {
            return Err(JsError::new(
                "Error",
                "JavaScriptCore could not create the opaque class",
            ));
        }
        Ok(Self {
            context,
            bridge: Bridge {
                helpers,
                opaque_class,
            },
        })
    }

    fn eval(&self, source: &str, name: &str) -> Result<JsValue, JsError> {
        // Every `valueWith…` produces an autoreleased object; the pool keeps
        // them from accumulating in whatever pool the caller happens to run
        // under — and lets a dropped box be finalized promptly.
        autoreleasepool(|_| {
            // SAFETY: clears any stale pending exception before evaluating.
            unsafe { self.context.setException(None) };
            let script = ns(source);
            // `name` is an arbitrary string; `URLWithString` returns `None`
            // for one it cannot parse, which `evaluateScript` accepts.
            let url = NSURL::URLWithString(&ns(name));
            // SAFETY: `context` is a live context and `script` a live
            // `NSString`.
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
            self.bridge.to_rust(&self.context, &result, 0)
        })
    }

    fn call(&self, function: &JsFunction, args: &[JsValue]) -> Result<JsValue, JsError> {
        autoreleasepool(|_| {
            let function = restore(function.handle())?;
            let mut marshaled = Vec::with_capacity(args.len());
            for arg in args {
                let arg = self.bridge.to_js(&self.context, arg, 0)?;
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
            let result = result.ok_or_else(|| {
                JsError::new("Error", "the call produced no value and no exception")
            })?;
            self.bridge.to_rust(&self.context, &result, 0)
        })
    }

    fn register(
        &self,
        name: &str,
        function: impl Fn(&[JsValue]) -> Result<JsValue, JsError> + 'static,
    ) -> Result<(), JsError> {
        autoreleasepool(|_| self.register_inner(name, function))
    }

    fn retain(&self, value: &JsValue) -> Result<JsObject, JsError> {
        autoreleasepool(|_| match value {
            JsValue::Object(_) | JsValue::ObjectRef(_) | JsValue::Function(_) => {
                let value = self.bridge.to_js(&self.context, value, 0)?;
                Ok(JsObject::from_handle(Handle::new(value)))
            }
            _ => Err(JsError::conversion(
                "only JavaScript objects and functions can be retained",
            )),
        })
    }

    fn collect_garbage(&self) {
        // The synchronous-debug entry point runs a complete collection —
        // including the external-reference scan that releases objects pinned
        // only by dead `JSValue` wrappers — where `JSGarbageCollect` leaves
        // them marked.
        // SAFETY: `context` is a live context.
        unsafe {
            JSSynchronousGarbageCollectForDebugging(self.context.JSGlobalContextRef().cast_const());
        }
    }
}

impl JscRuntime {
    /// The body of [`JsRuntime::register`], run inside an autorelease pool.
    fn register_inner(
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
                    match bridge.to_rust(&context, &value, 0) {
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
                Ok(value) => match bridge.to_js(&context, &value, 0) {
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
        // SAFETY: `namespace` is a live `JSValue`.
        if !unsafe { namespace.isObject() } {
            return Err(JsError::new(
                "Error",
                "__waterui_host is not an object; registration is impossible",
            ));
        }
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
        // `setObject_forKeyedSubscript` reports failure only through the
        // context's pending exception — drain it instead of returning `Ok`.
        if let Some(exception) = self.take_exception() {
            return Err(exception);
        }
        Ok(())
    }
}

impl fmt::Debug for JscRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JscRuntime(..)")
    }
}
