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
macro_rules! ffi_safe {
    ($($ty:ty),*) => {
       $(
            impl $crate::IntoFFI for $ty {
                type FFI = $ty;
                fn into_ffi(self) -> Self::FFI {
                    self
                }
            }


            impl $crate::IntoRust for $ty{
                type Rust=$ty;
                unsafe fn into_rust(self) -> Self::Rust{
                    self
                }
            }
       )*
    };
}

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

        impl_deref!($name, $ty);

        impl $crate::IntoFFI for $ty {
            type FFI = *mut $name;
            fn into_ffi(self) -> Self::FFI {
                alloc::boxed::Box::into_raw(alloc::boxed::Box::new($name(self)))
            }
        }

        impl $crate::IntoFFI for Option<$ty> {
            type FFI = *mut $name;
            fn into_ffi(self) -> Self::FFI {
                if let Some(value) = self {
                    value.into_ffi()
                } else {
                    core::ptr::null::<$name>() as *mut $name
                }
            }
        }

        impl $crate::IntoRust for *mut $name {
            type Rust = $ty;
            unsafe fn into_rust(self) -> Self::Rust {
                unsafe { alloc::boxed::Box::from_raw(self).0 }
            }
        }

        #[no_mangle]
        pub unsafe extern "C" fn $drop(value: *mut $name) {
            let _ = $crate::IntoRust::into_rust(value);
        }
    };
}

macro_rules! impl_computed {
    ($name:ident,$ty:ty,$ffi:ty,$read:ident,$watch:ident,$drop:ident) => {
        ffi_type!($name, waterui::Computed<$ty>, $drop);

        #[no_mangle]
        pub unsafe extern "C" fn $read(computed: *const $name) -> $ffi {
            use waterui::Compute;
            use $crate::IntoFFI;
            (*computed).compute().into_ffi()
        }

        #[no_mangle]
        pub unsafe extern "C" fn $watch(
            computed: *const $name,
            watcher: $crate::watcher::waterui_watcher<$ffi>,
        ) -> *mut $crate::waterui_watcher_guard {
            use waterui::Compute;
            use $crate::IntoFFI;
            let guard = (*computed).watch(waterui_reactive::watcher::Watcher::new(
                move |v: $ty, metadata| watcher.call(v.into_ffi(), metadata),
            ));
            guard.into_ffi()
        }
    };
}

#[macro_export]
macro_rules! impl_binding {
    ($name:ident,$ty:ty,$ffi:ty,$read:ident,$set:ident,$watch:ident,$drop:ident) => {
        ffi_type!($name, waterui::Binding<$ty>, $drop);
        #[no_mangle]
        pub unsafe extern "C" fn $read(binding: *const $name) -> $ffi {
            use waterui::Compute;
            use $crate::IntoFFI;
            (*binding).compute().into_ffi()
        }

        #[no_mangle]
        pub unsafe extern "C" fn $set(binding: *mut $name, value: $ffi) {
            use $crate::IntoRust;
            (*binding).set(value.into_rust());
        }

        #[no_mangle]
        pub unsafe extern "C" fn $watch(
            binding: *const $name,
            watcher: $crate::watcher::waterui_watcher<$ffi>,
        ) -> *mut $crate::waterui_watcher_guard {
            use waterui::Compute;
            use $crate::IntoFFI;
            let guard = (*binding).watch(waterui_reactive::watcher::Watcher::new(
                move |v: $ty, metadata| watcher.call(v.into_ffi(), metadata),
            ));
            guard.into_ffi()
        }
    };
}
