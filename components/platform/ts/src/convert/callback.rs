//! Rust closures TypeScript calls.
//!
//! A host function is registered under a name, never handed over as a value,
//! so a closure crossing into JavaScript is registered in the bridge's
//! registry and wrapped by `makeCallback(id)`. The wrapper's arguments are
//! converted with [`FromJs`] in declaration order — the same order the
//! schema's `Callback` node lists them — and a wrong shape is an error
//! naming the position, thrown where JavaScript called it.
//!
//! The spellings mirror `waterui-ts-schema`: boxed and atomically
//! reference-counted closures in every `Send`/`Sync` flavour, bare `Rc`
//! closures, and plain `fn` pointers, each up to eight arguments.

use std::rc::Rc;
use std::sync::Arc;

use waterui_ts_engine::{JsError, JsValue};

use super::{FromJs, IntoJs};
use crate::bridge::Bridge;

/// Registers `call` and returns the JavaScript function that invokes it.
///
/// The registration is kept for as long as the bundle is loaded: JavaScript
/// holds the wrapper, and nothing on the Rust side does.
fn export<F>(bridge: &Bridge, call: F) -> Result<JsValue, JsError>
where
    F: Fn(&[JsValue], &Bridge) -> Result<(), JsError> + 'static,
{
    let weak = bridge.downgrade();
    let handle = bridge.register_callback(move |args| {
        let bridge = weak.upgrade().ok_or_else(|| {
            JsError::new(
                "Error",
                "the TypeScript bridge was dropped while JavaScript was calling a Rust callback",
            )
        })?;
        call(args, &bridge)?;
        Ok(JsValue::Undefined)
    })?;
    let id = handle.id();
    bridge.retain_export(Rc::new(handle));
    bridge.make_callback(id)
}

/// Reads one argument, saying which position failed.
fn argument<T: FromJs>(args: &[JsValue], index: usize, bridge: &Bridge) -> Result<T, JsError> {
    let value = args.get(index).unwrap_or(&JsValue::Undefined);
    T::from_js(value, bridge).map_err(|error| JsError {
        message: format!("argument {index}: {}", error.message),
        ..error
    })
}

/// One callable spelling at one arity.
macro_rules! callback_spelling {
    ($ty:ty $(, $param:ident : $var:ident : $index:literal)*) => {
        impl<$($param: FromJs + 'static),*> IntoJs for $ty {
            /// Registers the closure and hands JavaScript the wrapper.
            fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError> {
                export(bridge, move |args, bridge| {
                    // A callback of no arguments uses neither.
                    let _ = (args, bridge);
                    $(let $var = argument::<$param>(args, $index, bridge)?;)*
                    self($($var),*);
                    Ok(())
                })
            }
        }
    };
}

/// Every spelling at one arity.
macro_rules! callback_arity {
    ($($param:ident : $var:ident : $index:literal),*) => {
        callback_spelling!(Box<dyn Fn($($param),*)> $(, $param : $var : $index)*);
        callback_spelling!(Box<dyn Fn($($param),*) + Send> $(, $param : $var : $index)*);
        callback_spelling!(Box<dyn Fn($($param),*) + Sync> $(, $param : $var : $index)*);
        callback_spelling!(Box<dyn Fn($($param),*) + Send + Sync> $(, $param : $var : $index)*);
        callback_spelling!(Arc<dyn Fn($($param),*)> $(, $param : $var : $index)*);
        callback_spelling!(Arc<dyn Fn($($param),*) + Send> $(, $param : $var : $index)*);
        callback_spelling!(Arc<dyn Fn($($param),*) + Sync> $(, $param : $var : $index)*);
        callback_spelling!(Arc<dyn Fn($($param),*) + Send + Sync> $(, $param : $var : $index)*);
        callback_spelling!(Rc<dyn Fn($($param),*)> $(, $param : $var : $index)*);
        callback_spelling!(fn($($param),*) $(, $param : $var : $index)*);
    };
}

callback_arity!();
callback_arity!(A: a: 0);
callback_arity!(A: a: 0, B: b: 1);
callback_arity!(A: a: 0, B: b: 1, C: c: 2);
callback_arity!(A: a: 0, B: b: 1, C: c: 2, D: d: 3);
callback_arity!(A: a: 0, B: b: 1, C: c: 2, D: d: 3, E: e: 4);
callback_arity!(A: a: 0, B: b: 1, C: c: 2, D: d: 3, E: e: 4, F: f: 5);
callback_arity!(A: a: 0, B: b: 1, C: c: 2, D: d: 3, E: e: 4, F: f: 5, G: g: 6);
callback_arity!(A: a: 0, B: b: 1, C: c: 2, D: d: 3, E: e: 4, F: f: 5, G: g: 6, H: h: 7);
