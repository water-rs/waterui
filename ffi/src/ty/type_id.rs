// ============================================================================
// Type ID Strategy
// ============================================================================

/// Type ID as a 128-bit value for O(1) comparison.
///
/// Uses 128-bit FNV-1a hash of `type_name()` for stability across dylib boundaries,
/// which is required for the preview system that loads user code as a dylib.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct WuiTypeId {
    /// The low 64 bits of the 128-bit FNV-1a type hash.
    pub low: u64,
    /// The high 64 bits of the 128-bit FNV-1a type hash.
    pub high: u64,
}

impl WuiTypeId {
    /// Creates a type ID from a type parameter.
    ///
    /// Always uses `type_name` hash to ensure consistency with `from_runtime()`,
    /// which handles views that may come from dynamically loaded dylibs.
    #[inline]
    #[must_use]
    pub fn of<T: 'static>() -> Self {
        Self::from_type_name(core::any::type_name::<T>())
    }

    /// Creates a type ID from a runtime `TypeId` and type name.
    ///
    /// Always uses `type_name` hash because this is called at runtime with views
    /// that may come from dynamically loaded dylibs. `TypeId` is not stable across
    /// dylib boundaries, but `type_name` is.
    #[inline]
    #[must_use]
    pub const fn from_runtime(_type_id: core::any::TypeId, name: &'static str) -> Self {
        Self::from_type_name(name)
    }

    /// Creates a type ID from a type name string.
    ///
    /// Uses 128-bit FNV-1a hash for virtually zero collision risk.
    #[inline]
    #[must_use]
    pub const fn from_type_name(name: &str) -> Self {
        let hash = fnv1a_128(name.as_bytes());
        #[expect(
            clippy::cast_possible_truncation,
            reason = "deliberately keeps only the low 64 bits of the 128-bit hash; the high 64 bits are captured separately below, so no bits are lost"
        )]
        let low = hash as u64;
        Self {
            low,
            high: (hash >> 64) as u64,
        }
    }
}

/// 128-bit FNV-1a hash function.
///
/// FNV-1a is fast and has good distribution properties.
/// Using 128-bit output virtually eliminates collision risk
/// (birthday paradox threshold: ~10^19 entries).
const fn fnv1a_128(bytes: &[u8]) -> u128 {
    const FNV_OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const FNV_PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;

    let mut hash = FNV_OFFSET;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u128;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}
