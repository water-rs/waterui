//! The JNI edge the HWUI clients share: one Kotlin object, its class and
//! method ids looked up once, and calls made on the UI thread the host has
//! already attached.

use std::fmt;

use jni::objects::{GlobalRef, JMethodID, JObject, JString};
use jni::signature::ReturnType;
use jni::sys::jvalue;
use jni::{JNIEnv, JavaVM};

/// A Kotlin object the HWUI target calls into.
pub struct JniObject {
    vm: JavaVM,
    object: GlobalRef,
    class: GlobalRef,
    owner: &'static str,
}

impl fmt::Debug for JniObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JniObject")
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

/// A method of a [`JniObject`]'s class, resolved once.
#[derive(Clone, Copy)]
pub struct Method {
    id: JMethodID,
    name: &'static str,
}

/// Why a call failed, before it is named.
enum Failure {
    Frame(jni::errors::Error),
    Call(String),
}

impl From<jni::errors::Error> for Failure {
    fn from(error: jni::errors::Error) -> Self {
        Self::Frame(error)
    }
}

impl JniObject {
    /// A client of `object`, an instance of the Kotlin class `owner` names.
    ///
    /// # Errors
    ///
    /// A description when the JVM or a global reference is unavailable.
    pub fn new(
        env: &JNIEnv<'_>,
        object: &JObject<'_>,
        owner: &'static str,
    ) -> Result<Self, String> {
        let failed = |what: &str, error: jni::errors::Error| format!("{owner}: {what}: {error}");
        let vm = env
            .get_java_vm()
            .map_err(|error| failed("the JavaVM", error))?;
        let object = env
            .new_global_ref(object)
            .map_err(|error| failed("a global reference", error))?;
        let class = env
            .get_object_class(&object)
            .and_then(|class| env.new_global_ref(class))
            .map_err(|error| failed("its class", error))?;
        Ok(Self {
            vm,
            object,
            class,
            owner,
        })
    }

    /// The method `name` with the JNI signature `signature`.
    ///
    /// # Errors
    ///
    /// A description when the class has no such method; the lookup's
    /// `NoSuchMethodError` is cleared.
    pub fn method(
        &self,
        env: &mut JNIEnv<'_>,
        name: &'static str,
        signature: &str,
    ) -> Result<Method, String> {
        env.get_method_id(&self.class, name, signature)
            .map(|id| Method { id, name })
            .map_err(|error| {
                let detail = if env.exception_check().unwrap_or(false) {
                    pending_exception(env)
                } else {
                    error.to_string()
                };
                format!("{}.{name}{signature}: {detail}", self.owner)
            })
    }

    /// Runs `call` on this thread's JNI env inside a local frame of
    /// `capacity` references. A thrown exception is described and cleared
    /// inside the frame.
    ///
    /// # Errors
    ///
    /// A description naming the method when this thread is not attached to
    /// the JVM (HWUI calls run on the UI thread, which the host attached),
    /// or when `call` fails.
    pub fn call<R>(
        &self,
        method: Method,
        capacity: i32,
        call: impl FnOnce(&mut JNIEnv<'_>, &JObject<'static>, Method) -> jni::errors::Result<R>,
    ) -> Result<R, String> {
        let named = |detail: String| format!("{}.{}: {detail}", self.owner, method.name);
        let mut env = self.vm.get_env().map_err(|error| {
            named(format!(
                "the calling thread is not attached to the JVM ({error}); HWUI calls run on the UI thread"
            ))
        })?;
        let object = self.object.as_obj();
        env.with_local_frame(capacity, |env| {
            call(env, object, method).map_err(|error| {
                Failure::Call(if env.exception_check().unwrap_or(false) {
                    pending_exception(env)
                } else {
                    error.to_string()
                })
            })
        })
        .map_err(|failure| match failure {
            Failure::Frame(error) => named(format!("a local frame: {error}")),
            Failure::Call(detail) => named(detail),
        })
    }
}

impl Method {
    /// Calls this method on `object`.
    ///
    /// # Safety
    ///
    /// `args` match the signature this method was resolved with, and `ret`
    /// is its return type.
    pub unsafe fn invoke<'local>(
        self,
        env: &mut JNIEnv<'local>,
        object: &JObject<'_>,
        ret: ReturnType,
        args: &[jvalue],
    ) -> jni::errors::Result<jni::objects::JValueOwned<'local>> {
        // SAFETY: the caller's contract: `self.id` belongs to `object`'s
        // class and `args`/`ret` match its signature.
        unsafe { env.call_method_unchecked(object, self.id, ret, args) }
    }
}

/// Clears the pending Java exception and describes it. Its local
/// references live in the caller's frame.
fn pending_exception(env: &mut JNIEnv<'_>) -> String {
    let Ok(throwable) = env.exception_occurred() else {
        return "a Java exception".to_owned();
    };
    if env.exception_clear().is_err() {
        return "a Java exception that could not be cleared".to_owned();
    }
    let described = env
        .call_method(&throwable, "toString", "()Ljava/lang/String;", &[])
        .and_then(jni::objects::JValueGen::l)
        .and_then(|text| env.get_string(&JString::from(text)).map(String::from));
    described.unwrap_or_else(|_| {
        // Describing failed: drop that exception too, keep the call's error.
        let _cleared = env.exception_clear();
        "a Java exception whose toString failed".to_owned()
    })
}
