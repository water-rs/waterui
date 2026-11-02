//! The JNI edge the HWUI clients share: one Kotlin object, its class and
//! method ids looked up once, and calls on the thread the host attached —
//! text methods may run on native shaping workers, which each call attaches.

use std::fmt;

use jni::objects::{Global, JClass, JMethodID, JObject, JString};
use jni::signature::{MethodSignature, ReturnType, RuntimeMethodSignature};
use jni::strings::JNIString;
use jni::sys::jvalue;
use jni::vm::AttachConfig;
use jni::{Env, JavaVM, jni_sig, jni_str};

/// A Kotlin object the HWUI target calls into.
pub struct JniObject {
    vm: JavaVM,
    object: Global<JObject<'static>>,
    class: Global<JClass<'static>>,
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
        env: &mut Env<'_>,
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
            .map_err(|error| failed("its class", error))?;
        let class = env
            .new_global_ref(class)
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
        env: &mut Env<'_>,
        name: &'static str,
        signature: &str,
    ) -> Result<Method, String> {
        let parsed = RuntimeMethodSignature::from_str(signature).map_err(|error| {
            format!(
                "{}.{name}{signature}: malformed signature: {error}",
                self.owner
            )
        })?;
        env.get_method_id(
            &self.class,
            JNIString::new(name),
            MethodSignature::from(&parsed),
        )
        .map(|id| Method { id, name })
        .map_err(|error| {
            let detail = if env.exception_check() {
                pending_exception(env)
            } else {
                error.to_string()
            };
            format!("{}.{name}{signature}: {detail}", self.owner)
        })
    }

    /// Runs `call` on this thread's JNI env inside a local frame of
    /// `capacity` references. A thrown exception is described and cleared
    /// inside the frame. Native worker threads stay attached until they
    /// exit (the 0.22 default), which reuses the attach across calls;
    /// every local reference still lives only for the frame.
    ///
    /// # Errors
    ///
    /// A description naming the method when this thread cannot attach to
    /// the JVM, or when `call` fails.
    pub fn call<R>(
        &self,
        method: Method,
        capacity: usize,
        call: impl FnOnce(&mut Env<'_>, &JObject<'static>, Method) -> jni::errors::Result<R>,
    ) -> Result<R, String> {
        let named = |detail: String| format!("{}.{}: {detail}", self.owner, method.name);
        let result: Result<R, Failure> = self.vm.attach_current_thread_with_config(
            AttachConfig::default,
            Some(capacity),
            |env| {
                call(env, self.object.as_obj(), method).map_err(|error| {
                    Failure::Call(if env.exception_check() {
                        pending_exception(env)
                    } else {
                        error.to_string()
                    })
                })
            },
        );
        result.map_err(|failure| match failure {
            Failure::Frame(error) => named(format!("attaching or creating a local frame: {error}")),
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
        env: &mut Env<'local>,
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
fn pending_exception(env: &mut Env<'_>) -> String {
    let Some(throwable) = env.exception_occurred() else {
        return "a Java exception".to_owned();
    };
    env.exception_clear();
    let described = (|| -> Result<String, jni::errors::Error> {
        let text = env
            .call_method(
                &throwable,
                jni_str!("toString"),
                jni_sig!("()Ljava/lang/String;"),
                &[],
            )?
            .l()?;
        let text = JString::cast_local(env, text)?;
        text.try_to_string(env)
    })();
    described.unwrap_or_else(|_| {
        // Describing failed: drop that exception too, keep the call's error.
        env.exception_clear();
        "a Java exception whose toString failed".to_owned()
    })
}
