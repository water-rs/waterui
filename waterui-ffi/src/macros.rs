#[macro_export]
macro_rules! ffi_view {
    ($view:ty,$ffi:ty,$force_as:ident,$id:ident) => {
        #[no_mangle]
        unsafe extern "C" fn $force_as(view: *mut $crate::waterui_anyview) -> $ffi {
            let any: waterui::AnyView = $crate::IntoRust::into_rust(view);
            let view = unsafe { (*any.downcast_unchecked::<$view>()) };
            $crate::IntoFFI::into_ffi(view)
        }

        #[no_mangle]
        extern "C" fn $id() -> $crate::waterui_type_id {
            $crate::IntoFFI::into_ffi(core::any::TypeId::of::<$view>())
        }
    };
}

#[macro_export]
macro_rules! ffi_metadata {
    ($metadata:ty,$ffi:ty,$force_as:ident,$id:ident) => {
        #[no_mangle]
        unsafe extern "C" fn $force_as(
            view: *mut $crate::waterui_anyview,
        ) -> $crate::component::metadata::waterui_metadata<$ffi> {
            let any: waterui::AnyView = $crate::IntoRust::into_rust(view);
            let view = unsafe {
                (*any.downcast_unchecked::<waterui::component::metadata::Metadata<$metadata>>())
            };
            $crate::IntoFFI::into_ffi(view)
        }

        #[no_mangle]
        extern "C" fn $id() -> $crate::waterui_type_id {
            $crate::IntoFFI::into_ffi(core::any::TypeId::of::<
                waterui::component::metadata::Metadata<$metadata>,
            >())
        }
    };
}

#[macro_export]
macro_rules! ffi_safe {
    ($($ty:ty),*) => {
       $(
            impl IntoFFI for $ty {
                type FFI = $ty;
                fn into_ffi(self) -> Self::FFI {
                    self
                }
            }


            impl IntoRust for $ty{
                type Rust=$ty;
                unsafe fn into_rust(self) -> Self::Rust{
                    self
                }
            }
       )*
    };
}

#[macro_export]
macro_rules! ffi_type {
    ($name:ident,$ty:ty) => {
        pub struct $name(pub(crate) $ty);

        impl core::ops::Deref for $name {
            type Target = $ty;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl core::ops::DerefMut for $name {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl $crate::IntoFFI for $ty {
            type FFI = *mut $name;
            fn into_ffi(self) -> Self::FFI {
                alloc::boxed::Box::into_raw(alloc::boxed::Box::new($name(self)))
            }
        }

        impl $crate::IntoRust for *mut $name {
            type Rust = $ty;
            unsafe fn into_rust(self) -> Self::Rust {
                alloc::boxed::Box::from_raw(self).0
            }
        }
    };
}
