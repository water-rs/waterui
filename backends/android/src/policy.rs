//! The JNI error policy every `extern "system"` entry resolves through.
//!
//! `jni`'s own `ThrowRuntimeExAndDefault` downcasts only `&'static str`
//! panic payloads — but `expect`/`format!`-style panics carry `String`,
//! which it reports as the opaque "non-string panic payload". This policy
//! is the same behavior — throw `RuntimeException`, return `T::default` —
//! with both payload kinds downcast, so the Java exception carries the
//! real message.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

use jni::Env;
use jni::errors::ErrorPolicy;

/// Throws `java.lang.RuntimeException` for errors and panics, returning
/// the default — `jni::errors::ThrowRuntimeExAndDefault` plus `String`
/// panic payloads.
#[derive(Debug, Default)]
pub struct ThrowRuntimeExAndDefault;

impl<T: Default, E: std::error::Error> ErrorPolicy<T, E> for ThrowRuntimeExAndDefault {
    type Captures<'unowned_env_local: 'native_method, 'native_method> = ();

    fn on_error<'unowned_env_local: 'native_method, 'native_method>(
        env: &mut Env<'unowned_env_local>,
        _cap: &mut Self::Captures<'unowned_env_local, 'native_method>,
        err: E,
    ) -> jni::errors::Result<T> {
        if env.exception_check() {
            return Ok(T::default()); // already thrown
        }
        // `env.throw` reports `Error::JavaException` after throwing; the
        // exception is the point, so the error it returns is discarded.
        let _ = env.throw(format!("Rust error: {err}"));
        Ok(T::default())
    }

    fn on_panic<'unowned_env_local: 'native_method, 'native_method>(
        env: &mut Env<'unowned_env_local>,
        _cap: &mut Self::Captures<'unowned_env_local, 'native_method>,
        payload: Box<dyn Any + Send + 'static>,
    ) -> jni::errors::Result<T> {
        let panic_string = match payload.downcast::<&'static str>() {
            Ok(s) => (*s).to_string(),
            Err(payload) => match payload.downcast::<String>() {
                Ok(s) => *s,
                Err(payload) => {
                    // Dropping an exotic payload can itself panic; the
                    // panic that drop produced is leaked rather than
                    // re-entered.
                    if let Err(drop_panic) = catch_unwind(AssertUnwindSafe(|| drop(payload))) {
                        std::mem::forget(drop_panic);
                    }
                    "non-string panic payload".to_string()
                }
            },
        };
        // Same discard as `on_error` — the exception is the report.
        let _ = env.throw(format!("Rust panic: {panic_string}"));
        Ok(T::default())
    }
}
