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
    pub low: u64,
    pub high: u64,
}

impl WuiTypeId {
    /// Creates a type ID from a type parameter.
    ///
    /// Always uses type_name hash to ensure consistency with `from_runtime()`,
    /// which handles views that may come from dynamically loaded dylibs.
    #[inline]
    pub fn of<T: 'static>() -> Self {
        Self::from_type_name(core::any::type_name::<T>())
    }

    /// Creates a type ID from a runtime TypeId and type name.
    ///
    /// Always uses type_name hash because this is called at runtime with views
    /// that may come from dynamically loaded dylibs. TypeId is not stable across
    /// dylib boundaries, but type_name is.
    #[inline]
    pub fn from_runtime(_type_id: core::any::TypeId, name: &'static str) -> Self {
        Self::from_type_name(name)
    }

    /// Creates a type ID from a type name string.
    ///
    /// Uses 128-bit FNV-1a hash for virtually zero collision risk.
    #[inline]
    pub fn from_type_name(name: &str) -> Self {
        let hash = fnv1a_128(name.as_bytes());
        Self {
            low: hash as u64,
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
    const FNV_OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const FNV_PRIME: u128 = 0x0000000001000000000000000000013b;

    let mut hash = FNV_OFFSET;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u128;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}
