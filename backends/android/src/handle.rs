//! Opaque 64-bit handles crossing the JNI border.
//!
//! A handle is a bit pattern, not a number: arm64 Android tags heap
//! pointers in the top byte (TBI/MTE), so a valid pointer sits above
//! `i64::MAX`. Both directions convert bit-preservingly through this one
//! pair — a checked numeric conversion like `jlong::try_from` rejects
//! exactly the pointers real devices produce.

use jni::sys::jlong;

/// `ptr` → the opaque `jlong` the host holds.
///
/// The pointer's bits are the handle: the top byte may carry an arm64
/// tag, which survives the cast untouched. `*mut T` coerces into the
/// `*const T` the signature asks for.
pub fn pointer_to_jlong<T>(ptr: *const T) -> jlong {
    let bits = ptr as usize as u64;
    bits.cast_signed()
}

/// The `jlong` the host hands back → the pointer it names.
///
/// The reverse of [`pointer_to_jlong`]: the full 64-bit pattern is
/// restored, tag byte and all, on the 64-bit targets this crate runs on.
/// Callers that want `*const T` add `.cast_const()`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a handle is 64-bit and every target this crate builds for — Android's own set plus the x86_64 host the tests run on — has a 64-bit usize"
)]
pub const fn jlong_to_pointer<T>(handle: jlong) -> *mut T {
    handle.cast_unsigned() as usize as *mut T
}

#[cfg(test)]
mod tests {
    use super::{jlong_to_pointer, pointer_to_jlong};

    /// A top-byte-tagged arm64 pointer survives the round trip — the case
    /// `jlong::try_from` rejected on Pixel hardware.
    #[test]
    fn tagged_pointer_round_trip() {
        let tagged = 0xb400_0070_1234_5678_usize as *mut u8;
        assert_eq!(jlong_to_pointer::<u8>(pointer_to_jlong(tagged)), tagged);
    }

    /// An untagged address behaves the same way.
    #[test]
    fn untagged_pointer_round_trip() {
        let plain = 0x0000_0070_1234_5678_usize as *mut u8;
        assert_eq!(jlong_to_pointer::<u8>(pointer_to_jlong(plain)), plain);
    }
}
