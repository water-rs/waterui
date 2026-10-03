//! Numbers, booleans, strings, and `()`.

use suiteki::Str;
use waterui_ts_engine::{BigInt, JsError, JsValue};

use super::{FromJs, IntoJs, expected};
use crate::bridge::Bridge;

impl IntoJs for () {
    /// `()` is the schema's `void`, and JavaScript's is `undefined`.
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(JsValue::Undefined)
    }
}

impl FromJs for () {
    /// A function that returns nothing may end with `return;` or with no
    /// `return` at all, so both spellings of "no value" read as `()`.
    fn from_js(value: &JsValue, _bridge: &Bridge) -> Result<Self, JsError> {
        match value {
            JsValue::Undefined | JsValue::Null => Ok(()),
            other => Err(expected("no value", other)),
        }
    }
}

impl IntoJs for bool {
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(JsValue::Bool(self))
    }
}

impl FromJs for bool {
    /// Strictly a `boolean`: JavaScript's truthiness is not a conversion, and
    /// silently reading `0` as `false` would hide a contract mismatch.
    fn from_js(value: &JsValue, _bridge: &Bridge) -> Result<Self, JsError> {
        value.as_bool().ok_or_else(|| expected("a boolean", value))
    }
}

impl IntoJs for f64 {
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(JsValue::Number(self))
    }
}

impl FromJs for f64 {
    fn from_js(value: &JsValue, _bridge: &Bridge) -> Result<Self, JsError> {
        value.as_f64().ok_or_else(|| expected("a number", value))
    }
}

impl IntoJs for f32 {
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(JsValue::Number(f64::from(self)))
    }
}

impl FromJs for f32 {
    /// JavaScript has one floating-point type, so a `f32` field narrows the
    /// double it is given. That is the mapping's own lossiness, declared by
    /// the schema, not a value being silently repaired.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "TypeScript has only the double; an f32 field narrows it by contract"
    )]
    fn from_js(value: &JsValue, _bridge: &Bridge) -> Result<Self, JsError> {
        value
            .as_f64()
            .map(|value| value as Self)
            .ok_or_else(|| expected("a number", value))
    }
}

/// The integers TypeScript sees as `number`: at most 32 bits, so every value
/// is exact in a double.
macro_rules! number_integers {
    ($($ty:ty),* $(,)?) => {
        $(
            impl IntoJs for $ty {
                fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
                    Ok(JsValue::from(self))
                }
            }

            impl FromJs for $ty {
                /// Accepts a `number` (or a `bigint`) holding this integer
                /// exactly; a fraction, a `NaN`, or a value out of range is an
                /// error rather than a truncation.
                fn from_js(value: &JsValue, _bridge: &Bridge) -> Result<Self, JsError> {
                    let integer = value
                        .as_i64()
                        .ok_or_else(|| expected(concat!("an exact ", stringify!($ty)), value))?;
                    Self::try_from(integer).map_err(|_| {
                        JsError::conversion(format!(
                            concat!("{} is out of range for ", stringify!($ty)),
                            integer
                        ))
                    })
                }
            }
        )*
    };
}

number_integers!(i8, i16, i32, u8, u16);

impl IntoJs for u32 {
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(JsValue::from(self))
    }
}

impl FromJs for u32 {
    /// Read through the unsigned accessor, so a `u32` past `i64::MAX` is
    /// impossible to reach but a negative value is still refused.
    fn from_js(value: &JsValue, _bridge: &Bridge) -> Result<Self, JsError> {
        let integer = value
            .as_u64()
            .ok_or_else(|| expected("an exact u32", value))?;
        Self::try_from(integer)
            .map_err(|_| JsError::conversion(format!("{integer} is out of range for u32")))
    }
}

/// The integers TypeScript sees as `bigint`: 64-bit and pointer-sized, whose
/// low bits a double loses.
macro_rules! bigint_integers {
    ($($ty:ty => $read:ident, $tag:expr),* $(,)?) => {
        $(
            impl IntoJs for $ty {
                /// Always a `bigint`, because the schema types the field
                /// `bigint`: a `number` for the small values would make
                /// arithmetic with the large ones throw.
                fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
                    let tag: fn($ty) -> BigInt = $tag;
                    Ok(JsValue::BigInt(tag(self)))
                }
            }

            impl FromJs for $ty {
                /// A `bigint`, or a `number` holding the integer exactly.
                fn from_js(value: &JsValue, _bridge: &Bridge) -> Result<Self, JsError> {
                    let integer = value
                        .$read()
                        .ok_or_else(|| expected(concat!("an exact ", stringify!($ty)), value))?;
                    Self::try_from(integer).map_err(|_| {
                        JsError::conversion(format!(
                            concat!("{} is out of range for ", stringify!($ty)),
                            integer
                        ))
                    })
                }
            }
        )*
    };
}

bigint_integers!(
    i64 => as_i64, BigInt::Signed,
    u64 => as_u64, BigInt::Unsigned,
    // `isize` and `usize` are at most 64 bits on every target WaterUI builds
    // for, so the widening below is exact.
    isize => as_i64, |value: isize| BigInt::Signed(value as i64),
    usize => as_u64, |value: usize| BigInt::Unsigned(value as u64),
);

impl IntoJs for String {
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(JsValue::String(self))
    }
}

impl FromJs for String {
    fn from_js(value: &JsValue, _bridge: &Bridge) -> Result<Self, JsError> {
        value
            .as_str()
            .map(ToOwned::to_owned)
            .ok_or_else(|| expected("a string", value))
    }
}

impl IntoJs for &str {
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(JsValue::String(self.to_owned()))
    }
}

impl IntoJs for Str {
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(JsValue::String(self.as_str().to_owned()))
    }
}

impl FromJs for Str {
    fn from_js(value: &JsValue, _bridge: &Bridge) -> Result<Self, JsError> {
        value
            .as_str()
            .map(|value| Self::from(value.to_owned()))
            .ok_or_else(|| expected("a string", value))
    }
}
