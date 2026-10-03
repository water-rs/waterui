//! Explicit `as`-semantic numeric conversions used across the backend.
//!
//! Layout, measure and draw code converts between the platform's `f64`
//! geometry types and the renderer's `f32`/integer pixel types at hundreds of
//! sites. Each helper is the single place that performs a given `as`-style
//! conversion, documents exactly what it does to the value, and carries the
//! `#[expect]` for the narrowing it intentionally performs, so a changed call
//! site can no longer silently change its conversion semantics.
//!
//! Semantics follow `as` exactly:
//! - `f64`/`f32` → integer: saturating truncation toward zero (`NaN` → 0,
//!   out-of-range → the type's bound).
//! - integer → integer: wraps modulo the destination width.
//! - `f64` → `f32`: rounds to nearest, saturating to ±∞ outside `f32` range.
//! - integer → float: rounds to nearest (may lose low bits past 24/53 bits of
//!   mantissa).

/// Narrows a finite `f64` coordinate to `f32`, rounding to nearest; layout
/// values are finite and far inside `f32` range.
#[expect(
    clippy::cast_possible_truncation,
    reason = "hydrolysis layout coordinates are finite and well inside f32 range; the f64->f32 narrowing is the intended conversion"
)]
pub const fn f64_as_f32(v: f64) -> f32 {
    v as f32
}

/// Widens a `usize` count/extent to `f64`, rounding to nearest when the value
/// exceeds the 53-bit mantissa; the counts involved are small.
#[expect(
    clippy::cast_precision_loss,
    reason = "the widened values are element counts and pixel extents, far below 2^53"
)]
pub const fn usize_as_f64(v: usize) -> f64 {
    v as f64
}

/// Widens a `u32` extent to `f32`, rounding to nearest when the value exceeds
/// the 24-bit mantissa; the extents involved are small.
#[expect(
    clippy::cast_precision_loss,
    reason = "the widened values are pixel extents, far below 2^24"
)]
pub const fn u32_as_f32(v: u32) -> f32 {
    v as f32
}

/// Widens a `u64` count to `f64`, rounding to nearest when the value exceeds
/// the 53-bit mantissa; the counts involved are small.
#[expect(
    clippy::cast_precision_loss,
    reason = "the widened values are counts, far below 2^53"
)]
pub const fn u64_as_f64(v: u64) -> f64 {
    v as f64
}

/// Widens a `usize` extent to `f32`, rounding to nearest; the extents are far
/// below the 24-bit mantissa.
#[expect(
    clippy::cast_precision_loss,
    reason = "the widened values are pixel extents, far below 2^24"
)]
#[cfg(test)]
pub const fn usize_as_f32(v: usize) -> f32 {
    v as f32
}

/// Widens an `i32` extent to `f32`, rounding to nearest; the extents are far
/// Converts a `f64` coordinate to `u32` with `as` saturating truncation: `NaN`
/// becomes 0, negatives clamp to 0, fractions truncate toward zero; the call
/// sites produce non-negative layout values.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "layout/pixel coordinates are non-negative and within u32 range; saturating truncation is the intended conversion"
)]
pub const fn f64_as_u32(v: f64) -> u32 {
    v as u32
}

/// Converts a `f32` coordinate to `u32` with `as` saturating truncation — see
/// [`f64_as_u32`].
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "layout/pixel coordinates are non-negative and within u32 range; saturating truncation is the intended conversion"
)]
#[cfg(any(test, not(target_arch = "wasm32")))]
pub const fn f32_as_u32(v: f32) -> u32 {
    v as u32
}

/// Converts a `f32` coordinate to `usize` with `as` saturating truncation —
/// Converts a `f32` channel to `u8` with `as` saturating truncation — see
/// [`f64_as_u32`].
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "colour channels are non-negative and within u8 range; saturating truncation is the intended conversion"
)]
pub const fn f32_as_u8(v: f32) -> u8 {
    v as u8
}

/// Converts a `f64` coordinate to `usize` with `as` saturating truncation —
/// see [`f64_as_u32`].
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "layout/pixel coordinates are non-negative and within usize range; saturating truncation is the intended conversion"
)]
pub const fn f64_as_usize(v: f64) -> usize {
    v as usize
}

/// Converts a `f32` extent to `i32` with `as` saturating truncation; extents
/// are within `i32` range.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the values are pixel extents within i32 range; saturating truncation is the intended conversion"
)]
#[cfg(any(test, not(target_arch = "wasm32")))]
pub const fn f32_as_i32(v: f32) -> i32 {
    v as i32
}

/// Converts a `f64` extent to `i32` with `as` saturating truncation; extents
/// are within `i32` range.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the values are pixel extents within i32 range; saturating truncation is the intended conversion"
)]
#[cfg(any(test, feature = "accessibility"))]
pub const fn f64_as_i32(v: f64) -> i32 {
    v as i32
}

/// Converts a `f32` extent to `u64` with `as` saturating truncation — see
/// [`f64_as_u32`].
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the values are non-negative extents within u64 range; saturating truncation is the intended conversion"
)]
pub const fn f32_as_u64(v: f32) -> u64 {
    v as u64
}

/// Truncates a `u64` to `usize`; only narrows on 32-bit targets, and the
/// values are byte counts/sizes that fit the target's address space.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the values are byte counts and sizes that fit the target's usize; truncation can only fire on 32-bit targets"
)]
pub const fn u64_as_usize(v: u64) -> usize {
    v as usize
}

/// Truncates a `u64` to `u32`; the values are sizes that fit `u32`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the values are sizes that fit u32"
)]
#[cfg(test)]
pub const fn u64_as_u32(v: u64) -> u32 {
    v as u32
}

/// Truncates a `u128` to `u64`; the values fit `u64` (hash/identifier widths).
#[expect(clippy::cast_possible_truncation, reason = "the values fit u64")]
pub const fn u128_as_u64(v: u128) -> u64 {
    v as u64
}

/// Truncates a `usize` to `u32`; the values are extents/counts within `u32`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the values are extents and counts within u32 range"
)]
#[cfg(test)]
pub const fn usize_as_u32(v: usize) -> u32 {
    v as u32
}

/// `i32` to `f32` — renderer coordinates where the integer is known to be
/// small (well within `f32`'s exact integer range).
#[expect(
    clippy::cast_precision_loss,
    reason = "i32 inputs here are small coordinate offsets well below 2^24"
)]
#[cfg(test)]
pub const fn i32_as_f32(v: i32) -> f32 {
    v as f32
}

/// Reinterprets a `i32` as `u32` wrapping modulo 2^32 — `as` semantics for
/// signed-to-unsigned; the call sites produce non-negative values.
#[expect(
    clippy::cast_sign_loss,
    reason = "the values are non-negative coordinates; the sign-loss branch is unreachable in practice"
)]
pub const fn i32_as_u32(v: i32) -> u32 {
    v as u32
}

/// Truncates and re-signs a `usize` to `i32` — `as` semantics; the values are
/// within `i32` range.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "the values are extents within i32 range"
)]
#[cfg(test)]
pub const fn usize_as_i32(v: usize) -> i32 {
    v as i32
}

/// `i16` → `u16` re-interpretation used for DOM key/button codes: negative
/// codes become large values that match `Other`, never wrapping to small
/// keys.
#[cfg(all(target_arch = "wasm32", feature = "web"))]
#[expect(
    clippy::cast_sign_loss,
    reason = "the DOM button code is re-interpreted, not signed: negatives become large Other() values"
)]
pub const fn i16_as_u16(v: i16) -> u16 {
    v as u16
}
