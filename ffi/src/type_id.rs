// ============================================================================
// Type ID Strategy
// ============================================================================

/// Type ID as a 128-bit value for O(1) comparison.
///
/// - Normal build: Uses `std::any::TypeId` (guaranteed unique by Rust)
/// - Hot reload: Uses 128-bit FNV-1a hash of `type_name()` (stable across dylib reloads)
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WuiTypeId {
    pub low: u64,
    pub high: u64,
}

impl WuiTypeId {
    /// Creates a type ID from a type parameter.
    #[inline]
    pub fn of<T: 'static>() -> Self {
        #[cfg(waterui_hot_reload_lib)]
        {
            Self::from_type_name(core::any::type_name::<T>())
        }

        #[cfg(not(waterui_hot_reload_lib))]
        {
            Self::from_type_id(core::any::TypeId::of::<T>())
        }
    }

    /// Creates a type ID from a runtime TypeId and type name.
    /// Uses TypeId in normal builds, type name hash in hot reload builds.
    #[inline]
    pub fn from_runtime(type_id: core::any::TypeId, name: &'static str) -> Self {
        #[cfg(waterui_hot_reload_lib)]
        {
            let _ = type_id;
            Self::from_type_name(name)
        }

        #[cfg(not(waterui_hot_reload_lib))]
        {
            let _ = name;
            Self::from_type_id(type_id)
        }
    }

    /// Creates a type ID from a TypeId (normal build only).
    #[cfg(not(waterui_hot_reload_lib))]
    #[inline]
    fn from_type_id(id: core::any::TypeId) -> Self {
        // TypeId is internally a u128 - transmute to access it
        // Safety: TypeId is repr(transparent) over u128 in current Rust
        let value: u128 = unsafe { core::mem::transmute(id) };
        Self {
            low: value as u64,
            high: (value >> 64) as u64,
        }
    }

    /// Creates a type ID from a type name string (hot reload build).
    #[cfg(waterui_hot_reload_lib)]
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
#[cfg(waterui_hot_reload_lib)]
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
