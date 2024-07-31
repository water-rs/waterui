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
macro_rules! impl_drop {
    ($name:ident,$ty:ty) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name(value: *mut $ty) {
            core::ptr::drop_in_place(value);
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
                unsafe { alloc::boxed::Box::from_raw(self).0 }
            }
        }
    };

    ($name:ident,$ty:ty,$drop:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $drop(value: *mut $name) {
            let _ = $crate::IntoRust::into_rust(value);
        }

        $crate::ffi_type!($name, $ty);
    };
}

// WARNING: Computed<T> must be called on the Rust thread!!!
macro_rules! impl_computed {
    ($computed:ty,$ffi:ty,$read:ident,$watch:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $read(computed: *const $computed) -> $ffi {
            use waterui::Compute;
            use $crate::IntoFFI;
            (*computed).compute().into_ffi()
        }

        #[no_mangle]
        pub unsafe extern "C" fn $watch(
            computed: *const $computed,
            watcher: $crate::closure::waterui_fn<$ffi>,
        ) -> *mut $crate::waterui_watcher_guard {
            use waterui::ComputeExt;
            let guard = (*computed)
                .watch(move |v: <$ffi as $crate::IntoRust>::Rust| watcher.call(v.into_ffi()));
            guard.into_ffi()
        }
    };
}
