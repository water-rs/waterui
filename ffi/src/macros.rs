#[macro_export]
/// Declares types as FFI-safe by implementing `IntoFFI` and `IntoRust` traits.
///
/// This macro automatically implements the necessary traits to make the specified types
/// usable across the FFI boundary. It creates trivial implementations where the FFI
/// representation is the same as the Rust representation.
///
/// # Arguments
///
/// * `$ty` - One or more types to make FFI-safe
///
/// # Example
///
/// ```ignore
/// ffi_safe!(u32, i32, bool);
/// ```
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

#[macro_export]
macro_rules! ffi_view {
    ($view:ty,$ffi:ty,$ident:tt) => {
        paste::paste! {
        /// # Safety
        /// This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
        /// The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn [<waterui_force_as_ $ident>](view: *mut $crate::WuiAnyView) -> $ffi {
            unsafe {
                let any: waterui::AnyView = $crate::IntoRust::into_rust(view);
                let view = (*any.downcast_unchecked::<waterui_core::Native<$view>>());
                $crate::IntoFFI::into_ffi(view)
            }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn [<waterui_ $ident _id>]() -> $crate::WuiStr {
            $crate::IntoFFI::into_ffi(core::any::type_name::<waterui_core::Native<$view>>()) // type_name is constant between runs, so it's good to use as a hot reload id.
        }
        }
    };
}

// metadata is a special case of view wrapper that holds both content and value
#[macro_export]
macro_rules! ffi_metadata {
    ($ty:ty,$ffi:ty) => {
        paste::paste! {
            $crate::ffi_view!(waterui_core::Metadata<$ty>,$crate::WuiMetadata<$ffi>,[<metadata_ $ty:snake>]);
        }
    };
}

#[macro_export]
macro_rules! opaque {
    ($name:ident,$ty:ty,$ident:tt) => {
        #[allow(nonstandard_style)]
        pub struct $name(pub(crate) $ty);

        $crate::impl_deref!($name, $ty);

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

        paste::paste! {
            /// # Safety
            /// The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [<waterui_drop_ $ident>](value: *mut $name) {
                unsafe {
                    let _ = $crate::IntoRust::into_rust(value);
                }
            }
        }
    };

    ($name:ident,$ty:ty) => {
        paste::paste! {
            $crate::opaque!($name,$ty,[<$ty:snake>]);
        }
    };
}

/// Derive `IntoFFI` trait for a struct or enum
/// # Example
/// ```ignore
/// into_ffi!{
///   ListConfig,
///   struct WuiList{
///      contents: *mut WuiAnyViews,
///   }
/// }
/// ```
macro_rules! into_ffi {
    ($ty:ty, $(#[$meta:meta])* pub struct $ffi:ident { $($field:ident : $ftype:ty),* $(,)? }) => {
        $(#[$meta])*
        #[repr(C)]
        pub struct $ffi {
            $(pub $field: $ftype),*
        }

        impl $crate::IntoFFI for $ty {
            type FFI = $ffi;
            fn into_ffi(self) -> Self::FFI {
                let value = self;
                $ffi {
                    $($field: $crate::IntoFFI::into_ffi(value.$field)),*
                }
            }
        }
    };

    ($ty:ty, $(#[$meta:meta])* pub enum $ffi:ident { $($variant:ident),* $(,)? }) => {
        $(#[$meta])*
        #[repr(C)]
        pub enum $ffi {
            $($variant),*
        }

        impl $crate::IntoFFI for $ty {
            type FFI = $ffi;
            fn into_ffi(self) -> Self::FFI {
                match self {
                    $(<$ty>::$variant => $ffi::$variant),*
                }
            }
        }

        impl $crate::IntoRust for $ffi {
            type Rust = $ty;
            unsafe fn into_rust(self) -> Self::Rust {
                match self {
                    $( $ffi::$variant => <$ty>::$variant ),*
                }
            }
        }
    };

    // enum which have default variant
    ($ty:ty, $default:ident, $(#[$meta:meta])* pub enum $ffi:ident { $($variant:ident),* $(,)? }) => {
        $(#[$meta])*
        #[repr(C)]
        pub enum $ffi {
            $($variant),*
        }

        impl $crate::IntoFFI for $ty {
            type FFI = $ffi;
            fn into_ffi(self) -> Self::FFI {
                match self {
                    $(<$ty>::$variant => $ffi::$variant),*,
                    _ => $ffi::$default,
                }
            }
        }

        impl $crate::IntoRust for $ffi {
            type Rust = $ty;
            unsafe fn into_rust(self) -> Self::Rust {
                match self {
                    $( $ffi::$variant => <$ty>::$variant ),*
                }
            }
        }
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
