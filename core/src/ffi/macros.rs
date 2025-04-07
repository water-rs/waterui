#[macro_export]
macro_rules! ffi_view {
    ($view:ty,$ffi:ty,$force_as:ident,$id:ident) => {
        #[unsafe(no_mangle)]
        unsafe extern "C" fn $force_as(view: *mut $crate::waterui_anyview) -> $ffi {
            let any: waterui::AnyView = $crate::IntoRust::into_rust(view);
            let view = unsafe { (*any.downcast_unchecked::<$view>()) };
            $crate::IntoFFI::into_ffi(view)
        }

        #[unsafe(no_mangle)]
        extern "C" fn $id() -> $crate::waterui_type_id {
            $crate::IntoFFI::into_ffi(core::any::TypeId::of::<$view>())
        }
    };
}

#[macro_export]
macro_rules! ffi_metadata {
    ($metadata:ty,$ffi:ty,$force_as:ident,$id:ident) => {
        $crate::ffi_view!(
            waterui::component::Metadata<$metadata>,
            $crate::component::metadata::waterui_metadata<$ffi>,
            $force_as,
            $id
        );
    };
}

#[macro_export]
macro_rules! into_ffi {
    ($ty:ty,$ffi:ty,$($param:ident),*) => {
        impl $crate::IntoFFI for $ty {
            type FFI = $ffi;
            fn into_ffi(self) -> Self::FFI {
                Self::FFI {
                    $(
                        $param: self.$param.into_ffi(),
                    )*
                }
            }
        }

    };
}

#[macro_export]
macro_rules! native_view {
    ($config:ty,$ffi:ty,$force_as:ident,$id:ident) => {
        $crate::ffi_view!(waterui::component::Native<$config>, $ffi, $force_as, $id);
    };
}

#[macro_export]
macro_rules! impl_deref {
    ($ty:ty,$target:ty) => {
        impl core::ops::Deref for $ty {
            type Target = $target;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl core::ops::DerefMut for $ty {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
    };
}

#[macro_export]
macro_rules! ffi_type {
    ($name:ident,$ty:ty,$drop:ident) => {
        pub struct $name(pub(crate) $ty);

        $crate::impl_deref!($name, $ty);

        impl $crate::ffi::IntoFFI for $ty {
            type FFI = *mut $name;
            fn into_ffi(self) -> Self::FFI {
                alloc::boxed::Box::into_raw(alloc::boxed::Box::new($name(self)))
            }
        }

        impl $crate::ffi::IntoFFI for Option<$ty> {
            type FFI = *mut $name;
            fn into_ffi(self) -> Self::FFI {
                if let Some(value) = self {
                    value.into_ffi()
                } else {
                    core::ptr::null::<$name>() as *mut $name
                }
            }
        }

        impl $crate::ffi::IntoRust for *mut $name {
            type Rust = $ty;
            unsafe fn into_rust(self) -> Self::Rust {
                unsafe { alloc::boxed::Box::from_raw(self).0 }
            }
        }

        #[unsafe(no_mangle)]
        /// Drops the FFI value.
        ///
        /// # Safety
        ///
        /// The pointer must be a valid pointer to a properly initialized value
        /// of the expected type, and must not be used after this function is called.
        pub unsafe extern "C" fn $drop(value: *mut $name) {
            unsafe {
                let _ = $crate::ffi::IntoRust::into_rust(value);
            }
        }
    };
}
