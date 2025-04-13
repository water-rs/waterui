use alloc::sync::Arc;
use uniffi::custom_type;

use crate::{Binding, Computed};
use waterui_str::Str;
use waterui_task::OnceValue;
#[doc(hidden)]
pub type __FFIComputed<T> = OnceValue<Computed<T>>;
#[doc(hidden)]
pub type __FFIBinding<T> = OnceValue<Binding<T>>;
#[doc(hidden)]
pub use paste::paste as __paste;

#[macro_export]
macro_rules! ffi_computed {
    ($ty:ty) => {
        $crate::ffi::__paste! {
            #[derive(uniffi::Object)]
            pub struct [<FFIComputed$ty>]($crate::ffi::__FFIComputed<$ty>);
            #[uniffi::export]
            impl [<FFIComputed$ty>] {
                pub fn compute(&self) -> $ty {
                    let value = self.0.get();
                    $crate::Compute::compute(&*value)
                }
            }

            type [<Computed$ty>] = $crate::Computed<$ty>;
            uniffi::custom_type!([<Computed$ty>], alloc::sync::Arc<[<FFIComputed$ty>]>,{
                remote,
                lower: |value| {alloc::sync::Arc::new([<FFIComputed$ty>](value.into()))},
                try_lift: |value| {Ok(value.0.get().clone())}
            });


        }
    };
}

macro_rules! ffi_computed_local {
    ($ty:ty) => {
        $crate::ffi::__paste! {
            #[derive(uniffi::Object)]
            pub struct [<FFIComputed$ty>]($crate::ffi::__FFIComputed<$ty>);
            #[uniffi::export]
            impl [<FFIComputed$ty>] {
                pub fn compute(&self) -> $ty {
                    let value = self.0.get();
                    $crate::Compute::compute(&*value)
                }
            }

            type [<Computed$ty>] = $crate::Computed<$ty>;
            uniffi::custom_type!([<Computed$ty>], alloc::sync::Arc<[<FFIComputed$ty>]>,{
                lower: |value| {alloc::sync::Arc::new([<FFIComputed$ty>](value.into()))},
                try_lift: |value| {Ok(value.0.get().clone())}
            });


        }
    };
}

ffi_computed_local!(Str);
ffi_computed_local!(i32);
ffi_computed_local!(f32);
ffi_computed_local!(f64);

ffi_computed_local!(bool);

#[macro_export]
macro_rules! ffi_binding {
    ($ty:ty) => {
        $crate::ffi::__paste! {
            #[derive(uniffi::Object)]
            pub struct [<FFIBinding$ty>]($crate::ffi::__FFIBinding<$ty>);
            #[uniffi::export]
            impl [<FFIBinding$ty>] {
                pub fn compute(&self) -> $ty {
                    let value = self.0.get();
                    $crate::Compute::compute(&*value)
                }
            }

            type [<Binding$ty>] = $crate::Binding<$ty>;
            uniffi::custom_type!([<Binding$ty>], alloc::sync::Arc<[<FFIBinding$ty>]>,{
                remote,
                lower: |value| {alloc::sync::Arc::new([<FFIBinding$ty>](value.into()))},
                try_lift: |value| {Ok(value.0.get().clone())}
            });


        }
    };
}
macro_rules! ffi_binding_local {
    ($ty:ty) => {
        $crate::ffi::__paste! {
            #[derive(uniffi::Object)]
            pub struct [<FFIBinding$ty>]($crate::ffi::__FFIBinding<$ty>);
            #[uniffi::export]
            impl [<FFIBinding$ty>] {
                pub fn compute(&self) -> $ty {
                    let value = self.0.get();
                    $crate::Compute::compute(&*value)
                }
            }

            type [<Binding$ty>] = $crate::Binding<$ty>;
            uniffi::custom_type!([<Binding$ty>], alloc::sync::Arc<[<FFIBinding$ty>]>,{
                lower: |value| {alloc::sync::Arc::new([<FFIBinding$ty>](value.into()))},
                try_lift: |value| {Ok(value.0.get().clone())}
            });


        }
    };
}

ffi_binding_local!(i32);
ffi_binding_local!(Str);
ffi_binding_local!(bool);
ffi_binding_local!(f32);
ffi_binding_local!(f64);
