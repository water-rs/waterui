#[macro_export]
macro_rules! impl_debug {
    ($ty:ty) => {
        impl core::fmt::Debug for $ty {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(core::any::type_name::<Self>())
            }
        }
    };
}
#[macro_export]
macro_rules! raw_view {
    ($ty:ty) => {
        impl $crate::View for $ty {
            fn body(self, _env: &$crate::Environment) -> impl $crate::View {
                panic!("You cannot call `body` for a raw view, may you need to handle this view `{}` manually", core::any::type_name::<$ty>());
            }
        }
    };
}

#[macro_export]
macro_rules! configurable {
    ($view:ident,$config:ty) => {
        #[derive(Debug)]
        pub struct $view($config);

        impl $crate::view::ConfigurableView for $view {
            type Config = $config;

            fn config(self) -> Self::Config {
                self.0
            }
        }

        impl From<$config> for $view {
            fn from(value: $config) -> Self {
                Self(value)
            }
        }

        impl $crate::view::View for $view {
            fn body(self, env: &$crate::Environment) -> impl $crate::View {
                use $crate::view::ConfigurableView;
                if let Some(modifier) = env.get::<$crate::view::Modifier<Self>>() {
                    $crate::components::AnyView::new(
                        modifier.clone().modify(env.clone(), self.config()),
                    )
                } else {
                    $crate::components::AnyView::new($crate::components::native::Native(self.0))
                }
            }
        }
    };
}

macro_rules! tuples {
    ($macro:ident) => {
        $macro!();
        $macro!(T0);
        $macro!(T0, T1);
        $macro!(T0, T1, T2);
        $macro!(T0, T1, T2, T3);
        $macro!(T0, T1, T2, T3, T4);
        $macro!(T0, T1, T2, T3, T4, T5);
        $macro!(T0, T1, T2, T3, T4, T5, T6);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13);
        $macro!(
            T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14
        );
    };
}

#[macro_export]
macro_rules! impl_extractor {
    ($ty:ty) => {
        impl $crate::extract::Extractor for $ty {
            fn extract(env: &$crate::Environment) -> core::result::Result<Self, $crate::Error> {
                $crate::extract::Extractor::extract(env)
                    .map(|value: $crate::extract::Use<$ty>| value.0)
            }
        }
    };
}

#[macro_export]
macro_rules! ffi_view {
    ($view_ty:ty,$ffi_ty:ty,$id:ident,$force_as:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $force_as(view: *mut $crate::AnyView) -> $ffi_ty {
            unsafe {
                let any: $crate::AnyView = $crate::ffi::IntoRust::into_rust(view).unwrap();
                let view = (*any.downcast_unchecked::<$view_ty>());
                $crate::ffi::IntoFFI::into_ffi(view)
            }
        }
        $crate::ffi_view!($view_ty, $id);
    };

    ($view_ty:ty,$id:ident) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn $id() -> $crate::ffi::WuiTypeId {
            $crate::ffi::IntoFFI::into_ffi(core::any::TypeId::of::<$view_ty>())
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
