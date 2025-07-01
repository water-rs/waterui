macro_rules! impl_debug {
    ($ty:ty) => {
        impl core::fmt::Debug for $ty {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(core::any::type_name::<Self>())
            }
        }
    };
}

macro_rules! impl_compute_result {
    ($ty:ty) => {
        impl core::cmp::PartialEq for $ty {
            fn eq(&self, _other: &Self) -> bool {
                false
            }
        }

        impl core::cmp::PartialOrd for $ty {
            fn partial_cmp(&self, _other: &Self) -> Option<core::cmp::Ordering> {
                None
            }
        }
    };
}
