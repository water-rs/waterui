//! The value model crossing between Rust and JavaScript.
//!
//! `JsValue` mirrors JavaScript's own types one to one, with two deliberate
//! seams:
//!
//! - **Integers past 2^53 cross as `BigInt`.** JavaScript `number` is an
//!   IEEE-754 double and holds integers exactly only up to
//!   [`MAX_SAFE_INTEGER`] in magnitude; anything larger would lose its low
//!   bits silently. Tagging follows the value, not the Rust type — the same
//!   rule the web view bridge applies — so a `u64` that fits stays a
//!   `number` and only the values that were already being corrupted change
//!   shape.
//! - **Handles, not copies.** A JavaScript function crosses as
//!   [`JsFunction`], a live JavaScript object may be retained as
//!   [`JsObject`], and a Rust value crosses the other way as [`Opaque`] — a
//!   box JavaScript can hold and pass back but cannot inspect.

use std::any::Any;
use std::fmt;
use std::rc::Rc;

use crate::handle::Handle;

/// `Number.MAX_SAFE_INTEGER`: the largest integer a double represents
/// exactly. Integers past it cross the seam as [`BigInt`].
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// An integer that crosses the bridge as a JavaScript `bigint`.
///
/// Equality is numeric — `Signed(5)` equals `Unsigned(5)` — because the two
/// variants describe the *representation*, not distinct values: a `bigint`
/// arriving from JavaScript becomes `Signed` whenever it fits an `i64`.
#[derive(Debug, Clone, Copy, Eq)]
pub enum BigInt {
    /// A value in `i64` range.
    Signed(i64),
    /// A value above `i64::MAX`, up to `u64::MAX`.
    Unsigned(u64),
}

impl BigInt {
    /// The value as `i64`, or `None` when it is out of range.
    #[must_use]
    pub fn as_i64(self) -> Option<i64> {
        match self {
            Self::Signed(value) => Some(value),
            Self::Unsigned(value) => i64::try_from(value).ok(),
        }
    }

    /// The value as `u64`, or `None` when it is out of range.
    #[must_use]
    pub fn as_u64(self) -> Option<u64> {
        match self {
            Self::Signed(value) => u64::try_from(value).ok(),
            Self::Unsigned(value) => Some(value),
        }
    }

    /// The exact value widened to `i128`, where both variants always fit.
    #[must_use]
    fn as_i128(self) -> i128 {
        match self {
            Self::Signed(value) => i128::from(value),
            Self::Unsigned(value) => i128::from(value),
        }
    }
}

impl PartialEq for BigInt {
    fn eq(&self, other: &Self) -> bool {
        self.as_i128() == other.as_i128()
    }
}

impl From<i64> for BigInt {
    fn from(value: i64) -> Self {
        Self::Signed(value)
    }
}

impl From<u64> for BigInt {
    fn from(value: u64) -> Self {
        Self::Unsigned(value)
    }
}

/// A value crossing between Rust and JavaScript, in either direction.
///
/// JavaScript values that have no `JsValue` — `symbol`, a `bigint` beyond
/// `u64::MAX`, an exotic object — fail conversion with a [`JsError`]; nothing
/// silently becomes `undefined`.
///
/// [`JsError`]: crate::JsError
#[derive(Debug, Clone)]
pub enum JsValue {
    /// JavaScript `undefined`.
    Undefined,
    /// JavaScript `null`.
    Null,
    /// JavaScript `boolean`.
    Bool(bool),
    /// JavaScript `number` (an IEEE-754 double).
    Number(f64),
    /// JavaScript `bigint`, for integers a [`Number`](Self::Number) cannot
    /// hold exactly.
    BigInt(BigInt),
    /// JavaScript `string`.
    String(String),
    /// JavaScript `Array`.
    Array(Vec<Self>),
    /// A plain JavaScript object as data: string-keyed entries in insertion
    /// order.
    Object(Vec<(String, Self)>),
    /// A retained reference to a JavaScript function, callable through
    /// [`JsRuntime::call`](crate::JsRuntime::call).
    Function(JsFunction),
    /// A retained reference to a live JavaScript object, held without
    /// copying its contents; see [`JsRuntime::retain`](crate::JsRuntime::retain).
    ObjectRef(JsObject),
    /// An opaque Rust value JavaScript can hold and pass back but cannot
    /// inspect.
    Opaque(Opaque),
}

impl JsValue {
    /// The boolean, or `None` for other kinds.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// The number, or `None` for other kinds.
    #[must_use]
    pub const fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(value) => Some(*value),
            _ => None,
        }
    }

    /// The integer as `i64`: a [`BigInt`](Self::BigInt) in range, or a
    /// [`Number`](Self::Number) holding an exact `i64` — the bit-exact check
    /// rejects fractions, `-0.0`, `NaN` and infinities for free.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "the casts are validated bit-exact by the to_bits comparison"
    )]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::BigInt(value) => value.as_i64(),
            Self::Number(value) => {
                let as_int = *value as i64;
                ((as_int as f64).to_bits() == value.to_bits()).then_some(as_int)
            }
            _ => None,
        }
    }

    /// The integer as `u64`: a [`BigInt`](Self::BigInt) in range, or a
    /// [`Number`](Self::Number) holding an exact `u64`.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "the casts are validated bit-exact by the to_bits comparison"
    )]
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::BigInt(value) => value.as_u64(),
            Self::Number(value) => {
                let as_int = *value as u64;
                ((as_int as f64).to_bits() == value.to_bits()).then_some(as_int)
            }
            _ => None,
        }
    }

    /// The string, or `None` for other kinds.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    /// The array elements, or `None` for other kinds.
    #[must_use]
    pub fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The object's entries in insertion order, or `None` for other kinds.
    #[must_use]
    pub fn as_object(&self) -> Option<&[(String, Self)]> {
        match self {
            Self::Object(entries) => Some(entries),
            _ => None,
        }
    }
}

/// Handles compare by identity, data by value. Numbers compare bitwise, so
/// `NaN` equals `NaN` and `-0.0` differs from `0.0`.
impl PartialEq for JsValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Undefined, Self::Undefined) | (Self::Null, Self::Null) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Number(a), Self::Number(b)) => a.to_bits() == b.to_bits(),
            (Self::BigInt(a), Self::BigInt(b)) => a == b,
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Array(a), Self::Array(b)) => a == b,
            (Self::Object(a), Self::Object(b)) => a == b,
            (Self::Function(a), Self::Function(b)) => a.ptr_eq(b),
            (Self::ObjectRef(a), Self::ObjectRef(b)) => a.ptr_eq(b),
            (Self::Opaque(a), Self::Opaque(b)) => a.ptr_eq(b),
            _ => false,
        }
    }
}

macro_rules! small_int_value {
    ($($ty:ty),*) => {
        $(
            impl From<$ty> for JsValue {
                /// Always a [`Number`](JsValue::Number): a `$ty` is far
                /// inside `±2^53`, so the double is exact.
                fn from(value: $ty) -> Self {
                    Self::Number(f64::from(value))
                }
            }
        )*
    };
}

small_int_value!(i8, i16, i32, u8, u16, u32);

impl From<i64> for JsValue {
    /// An `i64` past `±2^53` becomes a [`BigInt`](JsValue::BigInt) so it
    /// crosses exactly; everything else stays a [`Number`](JsValue::Number).
    #[expect(
        clippy::cast_precision_loss,
        reason = "the magnitude is checked to be at most 2^53 first, so the cast is exact"
    )]
    fn from(value: i64) -> Self {
        if u128::from(value.unsigned_abs()) <= u128::from(MAX_SAFE_INTEGER) {
            Self::Number(value as f64)
        } else {
            Self::BigInt(BigInt::Signed(value))
        }
    }
}

impl From<isize> for JsValue {
    /// An `isize` past `±2^53` becomes a [`BigInt`](JsValue::BigInt) so it
    /// crosses exactly; everything else stays a [`Number`](JsValue::Number).
    #[expect(
        clippy::cast_precision_loss,
        reason = "the magnitude is checked to be at most 2^53 first, so the cast is exact"
    )]
    fn from(value: isize) -> Self {
        if (value as i128).unsigned_abs() <= u128::from(MAX_SAFE_INTEGER) {
            Self::Number(value as f64)
        } else {
            Self::BigInt(BigInt::Signed(value as i64))
        }
    }
}

impl From<u64> for JsValue {
    /// A `u64` past `2^53` becomes a [`BigInt`](JsValue::BigInt) so it crosses
    /// exactly; everything else stays a [`Number`](JsValue::Number).
    #[expect(
        clippy::cast_precision_loss,
        reason = "the magnitude is checked to be at most 2^53 first, so the cast is exact"
    )]
    fn from(value: u64) -> Self {
        if u128::from(value) <= u128::from(MAX_SAFE_INTEGER) {
            Self::Number(value as f64)
        } else {
            Self::BigInt(BigInt::Unsigned(value))
        }
    }
}

impl From<usize> for JsValue {
    /// A `usize` past `2^53` becomes a [`BigInt`](JsValue::BigInt) so it
    /// crosses exactly; everything else stays a [`Number`](JsValue::Number).
    #[expect(
        clippy::cast_precision_loss,
        reason = "the magnitude is checked to be at most 2^53 first, so the cast is exact"
    )]
    fn from(value: usize) -> Self {
        if (value as u128) <= u128::from(MAX_SAFE_INTEGER) {
            Self::Number(value as f64)
        } else {
            Self::BigInt(BigInt::Unsigned(value as u64))
        }
    }
}

impl From<f64> for JsValue {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<f32> for JsValue {
    fn from(value: f32) -> Self {
        Self::Number(f64::from(value))
    }
}

impl From<bool> for JsValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<String> for JsValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for JsValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<Vec<Self>> for JsValue {
    fn from(value: Vec<Self>) -> Self {
        Self::Array(value)
    }
}

impl From<Vec<(String, Self)>> for JsValue {
    fn from(value: Vec<(String, Self)>) -> Self {
        Self::Object(value)
    }
}

impl From<JsFunction> for JsValue {
    fn from(value: JsFunction) -> Self {
        Self::Function(value)
    }
}

impl From<JsObject> for JsValue {
    fn from(value: JsObject) -> Self {
        Self::ObjectRef(value)
    }
}

impl From<Opaque> for JsValue {
    fn from(value: Opaque) -> Self {
        Self::Opaque(value)
    }
}

impl<T: 'static> From<Rc<T>> for JsValue {
    fn from(value: Rc<T>) -> Self {
        Self::Opaque(Opaque::new(value))
    }
}

/// A retained reference to a JavaScript function.
///
/// The handle keeps the function alive across calls without copying it;
/// invoking it goes through [`JsRuntime::call`](crate::JsRuntime::call).
/// Cloning shares the reference. A handle is bound to the engine that
/// produced it — handing it to a different engine is an error there, not
/// here.
#[derive(Clone)]
pub struct JsFunction(Handle);

/// A retained reference to a live JavaScript object.
///
/// Where [`JsValue::Object`] is a copy of an object's entries, a `JsObject`
/// points at the object itself: passing it back into JavaScript hands over
/// the same object, so mutations are shared and identity comparisons hold.
/// Produced by [`JsRuntime::retain`](crate::JsRuntime::retain); cloning
/// shares the reference.
#[derive(Clone)]
pub struct JsObject(Handle);

/// A Rust value JavaScript may hold and pass back but never inspect.
///
/// `AnyView`, materialized signals and other runtime state cross the seam
/// this way: JavaScript receives a box it can hand back to a host function
/// or return from a callback, and the same `Rc` comes out the other side.
#[derive(Clone)]
pub struct Opaque(Rc<dyn Any>);

impl Opaque {
    /// Boxes `value` for the trip into JavaScript.
    pub const fn new<T: 'static>(value: Rc<T>) -> Self {
        Self(value)
    }

    /// The value back, or `None` when the box wraps a different type.
    #[must_use]
    pub fn downcast<T: 'static>(&self) -> Option<Rc<T>> {
        self.0.clone().downcast::<T>().ok()
    }

    /// Whether two boxes wrap the same `Rc`.
    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// The boxed `Rc`, for engines that store it in a registry.
    #[doc(hidden)]
    #[must_use]
    pub const fn inner(&self) -> &Rc<dyn Any> {
        &self.0
    }

    /// A box around an already-erased `Rc`, for engines restoring one from
    /// their registry.
    #[doc(hidden)]
    #[must_use]
    pub const fn from_inner(inner: Rc<dyn Any>) -> Self {
        Self(inner)
    }
}

impl JsFunction {
    /// Engine-facing: wraps the engine's retained function reference.
    #[doc(hidden)]
    #[must_use]
    pub const fn from_handle(handle: Handle) -> Self {
        Self(handle)
    }

    /// Engine-facing: the retained reference.
    #[doc(hidden)]
    #[must_use]
    pub const fn handle(&self) -> &Handle {
        &self.0
    }

    fn ptr_eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}

impl JsObject {
    /// Engine-facing: wraps the engine's retained object reference.
    #[doc(hidden)]
    #[must_use]
    pub const fn from_handle(handle: Handle) -> Self {
        Self(handle)
    }

    /// Engine-facing: the retained reference.
    #[doc(hidden)]
    #[must_use]
    pub const fn handle(&self) -> &Handle {
        &self.0
    }

    fn ptr_eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}

impl fmt::Debug for JsFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JsFunction(..)")
    }
}

impl fmt::Debug for JsObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JsObject(..)")
    }
}

impl fmt::Debug for Opaque {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Opaque(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::{BigInt, JsValue, MAX_SAFE_INTEGER};

    #[test]
    fn big_int_equality_is_numeric() {
        assert_eq!(BigInt::Signed(5), BigInt::Unsigned(5));
        assert_eq!(BigInt::Signed(-1), BigInt::Signed(-1));
        assert_ne!(BigInt::Signed(-1), BigInt::Unsigned(u64::MAX));
    }

    #[test]
    fn integers_at_the_safe_boundary_stay_numbers() {
        assert_eq!(
            JsValue::from(MAX_SAFE_INTEGER),
            JsValue::Number(9_007_199_254_740_991.0)
        );
        assert_eq!(
            JsValue::from(MAX_SAFE_INTEGER + 1),
            JsValue::BigInt(BigInt::Unsigned(MAX_SAFE_INTEGER + 1))
        );
        assert_eq!(
            JsValue::from(i64::MAX),
            JsValue::BigInt(BigInt::Signed(i64::MAX))
        );
        assert_eq!(
            JsValue::from(i64::MIN),
            JsValue::BigInt(BigInt::Signed(i64::MIN))
        );
        assert_eq!(
            JsValue::from(u64::MAX),
            JsValue::BigInt(BigInt::Unsigned(u64::MAX))
        );
    }

    #[test]
    fn big_int_reads_at_the_extremes() {
        assert_eq!(BigInt::Signed(i64::MIN).as_i64(), Some(i64::MIN));
        assert_eq!(BigInt::Signed(i64::MIN).as_u64(), None);
        assert_eq!(BigInt::Unsigned(u64::MAX).as_u64(), Some(u64::MAX));
        assert_eq!(BigInt::Unsigned(u64::MAX).as_i64(), None);
        assert_eq!(
            BigInt::Unsigned(u64::from(u32::MAX)).as_i64(),
            Some(i64::from(u32::MAX))
        );
    }

    #[test]
    fn integer_reads_reject_inexact_numbers() {
        assert_eq!(JsValue::from(42_i64).as_i64(), Some(42));
        assert_eq!(
            JsValue::from(u64::from(u32::MAX)).as_u64(),
            Some(u64::from(u32::MAX))
        );
        assert_eq!(JsValue::Number(1.5).as_i64(), None);
        assert_eq!(JsValue::Number(-0.0).as_i64(), None);
        assert_eq!(JsValue::Number(f64::NAN).as_i64(), None);
        assert_eq!(JsValue::Number(f64::INFINITY).as_i64(), None);
        // 2^53 + 1 as a double rounds to 2^53 + 2 — still exact as an i64,
        // but it arrived as a `number`, which is what `as_i64` reports.
        assert_eq!(
            JsValue::Number(9_007_199_254_740_992.0).as_i64(),
            Some(9_007_199_254_740_992)
        );
        assert_eq!(JsValue::from(String::from("x")).as_i64(), None);
    }
}
