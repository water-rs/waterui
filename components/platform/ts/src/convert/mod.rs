//! Rust values, as JavaScript sees them.
//!
//! [`IntoJs`] and [`FromJs`] are the runtime half of the props contract the
//! schema describes: what `#[derive(TsProps)]` promises the TypeScript side,
//! these two produce and accept. There is no serializer in between — a value
//! becomes a [`JsValue`] directly — and every mapped type is implemented here,
//! in the same shapes the schema declares:
//!
//! | Schema | JavaScript |
//! | --- | --- |
//! | [`Number`] of 32 bits or fewer, `f32`, `f64` | `number` |
//! | [`Number`] of `i64`, `u64`, `isize`, `usize` | `bigint` |
//! | [`Bool`] | `boolean` |
//! | [`Unit`] | `undefined` |
//! | [`String`] | `string` |
//! | [`Option`] | the value, or `null` |
//! | [`List`] | an array |
//! | [`Map`] | an object |
//! | [`Signal`] | a `Signal` created by the runtime |
//! | [`Accessor`] | a memo over a signal the bridge pushes |
//! | [`View`] | an opaque view slot |
//! | [`Callback`] | a function wrapping a registered Rust closure |
//! | [`Struct`] | an object with the field names as properties |
//! | [`Enum`] | a string, or `{ type, value }` |
//!
//! The width rule is the schema's, not the value's: an `i64` field is typed
//! `bigint` on the TypeScript side, so it always crosses as a `bigint` — a
//! `number` where the type says `bigint` would break arithmetic in JavaScript
//! the moment the two met. Coming back, a `bigint` and a `number` holding the
//! integer exactly are both accepted, and anything inexact is an error rather
//! than a rounded value.
//!
//! [`Number`]: waterui_ts_schema::TypeSchema::Number
//! [`Bool`]: waterui_ts_schema::TypeSchema::Bool
//! [`Unit`]: waterui_ts_schema::TypeSchema::Unit
//! [`String`]: waterui_ts_schema::TypeSchema::String
//! [`Option`]: waterui_ts_schema::TypeSchema::Option
//! [`List`]: waterui_ts_schema::TypeSchema::List
//! [`Map`]: waterui_ts_schema::TypeSchema::Map
//! [`Signal`]: waterui_ts_schema::TypeSchema::Signal
//! [`Accessor`]: waterui_ts_schema::TypeSchema::Accessor
//! [`View`]: waterui_ts_schema::TypeSchema::View
//! [`Callback`]: waterui_ts_schema::TypeSchema::Callback
//! [`Struct`]: waterui_ts_schema::TypeSchema::Struct
//! [`Enum`]: waterui_ts_schema::TypeSchema::Enum

mod callback;
mod collection;
mod reactive;
mod scalar;

pub mod support;

use waterui_ts_engine::{JsError, JsValue};

use crate::bridge::Bridge;

/// A Rust value that can become a JavaScript value.
///
/// The bridge is passed in because some values need the runtime to exist: a
/// `Binding<T>` becomes a real JavaScript signal, and a callback is registered
/// so JavaScript can call it.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot cross into JavaScript",
    label = "no `IntoJs` conversion for `{Self}`",
    note = "props types are the mapped ones: numbers, `bool`, strings, `Option`, `Vec`, \
            `BTreeMap`/`HashMap`, `Binding`, `Computed`, `AnyView`, callbacks, or a struct or \
            enum deriving `TsType`"
)]
pub trait IntoJs: Sized {
    /// Converts this value for JavaScript.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when the value cannot cross — a registry that is
    /// exhausted, a view already taken, a runtime call that threw.
    fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError>;
}

/// A Rust value that can be read back out of a JavaScript value.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be read out of a JavaScript value",
    label = "no `FromJs` conversion for `{Self}`",
    note = "callback arguments and two-way values are the mapped types: numbers, `bool`, \
            strings, `Option`, `Vec`, `BTreeMap`/`HashMap`, `AnyView`, or a struct or enum \
            deriving `TsType`"
)]
pub trait FromJs: Sized {
    /// Reads this value out of `value`.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when the JavaScript value has the wrong shape, or
    /// holds a number the Rust type cannot represent exactly.
    fn from_js(value: &JsValue, bridge: &Bridge) -> Result<Self, JsError>;
}

/// "expected a `u32`, found a string" — the shape every mismatch reports.
pub fn expected(what: &str, value: &JsValue) -> JsError {
    JsError::conversion(format!(
        "expected {what}, found {}",
        crate::error::kind_of(value)
    ))
}

impl IntoJs for JsValue {
    /// A value already in the engine's currency crosses unchanged, which is
    /// what lets a host table pass a config value straight through.
    fn into_js(self, _bridge: &Bridge) -> Result<Self, JsError> {
        Ok(self)
    }
}

impl FromJs for JsValue {
    fn from_js(value: &Self, _bridge: &Bridge) -> Result<Self, JsError> {
        Ok(value.clone())
    }
}
