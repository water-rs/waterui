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

macro_rules! bindgen_view {
    ($ffi_view:ty,$view:ty,$($name:ident,$ty:ty),*) => {
        #[uniffi::export]
        impl $ffi_view {
            #[allow(clippy::new_ret_no_self)]
            #[uniffi::constructor]
            pub fn new(view: $crate::AnyView) -> $ffi_view {
                let view:$view=*view.downcast::<$view>().unwrap();
                view.into()
            }

            $(
                pub fn $name(&self) -> $ty {
                    self.$name.take()
                }
            )*

        }
    };
}
