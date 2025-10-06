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
/// Implements a new function for a type to be exposed over FFI.
///
/// This macro generates a C-compatible constructor function for the specified type
/// that creates a new instance by calling its `new()` method and converts it to
/// an FFI-safe representation.
///
/// # Arguments
///
/// * `$ty` - The type for which to implement the constructor
/// * `$new` - The name of the generated constructor function
///
/// # Example
///
/// ```ignore
/// impl_new!(MyStruct, my_struct_new);
/// ```
macro_rules! impl_new {
    ($ty:ty,$new:ident) => {
        #[unsafe(no_mangle)]
        /// Creates a new instance of the specified type.
        ///
        /// This creates a new heap-allocated value of the specified type using its `new()`
        /// constructor method, and returns an FFI-safe pointer to the created value.
        ///
        /// # Returns
        ///
        /// A pointer to the newly created value, which must be freed with the corresponding
        /// drop function to avoid memory leaks.
        pub extern "C" fn $new() -> *mut $ty {
            $crate::IntoFFI::into_ffi($ty::new());
        }
    };
}

#[macro_export]
/// Implements a drop function for an FFI type.
///
/// This macro generates a C-compatible function that safely drops a value
/// that was previously created through FFI.
///
/// # Arguments
///
/// * `$ty` - The FFI type to be dropped
/// * `$drop` - The name of the generated drop function
///
/// # Example
///
/// ```ignore
/// impl_drop!(*mut MyStruct, my_struct_drop);
/// ```
macro_rules! impl_drop {
    ($ty:ty,$drop:ident) => {
        #[unsafe(no_mangle)]
        /// Drops the FFI value.
        ///
        /// # Safety
        ///
        /// If `value` is NULL, this function does nothing. If `value` is not a valid pointer
        /// to a properly initialized value of the expected type, undefined behavior will occur.
        /// The pointer must not be used after this function is called.
        pub unsafe extern "C" fn $drop(value: $ty) {
            unsafe {
                let _ = $crate::IntoRust::into_rust(value);
            }
        }
    };
}

#[macro_export]
/// Implements a drop function for an opaque pointer type.
///
/// This is a convenience wrapper around `impl_drop!` that assumes the type
/// is represented as an opaque pointer (`*mut $ty`).
///
/// # Arguments
///
/// * `$ty` - The underlying type of the opaque pointer
/// * `$drop` - The name of the generated drop function
///
/// # Example
///
/// ```ignore
/// impl_opaque_drop!(MyStruct, my_struct_drop);
/// ```
macro_rules! impl_opaque_drop {
    ($ty:ty,$drop:ident) => {
        $crate::impl_drop!(*mut $ty, $drop);
    };
}

#[macro_export]
/// Implements a clone function for an opaque pointer type.
///
/// This macro generates a C-compatible function that creates a clone of a value
/// represented as an opaque pointer.
///
/// # Arguments
///
/// * `$ty` - The underlying type of the opaque pointer
/// * `$clone` - The name of the generated clone function
///
/// # Example
///
/// ```ignore
/// impl_opaque_clone!(MyStruct, my_struct_clone);
/// ```
macro_rules! impl_opaque_clone {
    ($ty:ty,$clone:ident) => {
        #[unsafe(no_mangle)]
        /// Clones an FFI value.
        ///
        /// # Safety
        ///
        /// If `value` is NULL, the behavior is undefined. The `value` parameter must be
        /// a valid pointer to an instance of the expected type that was previously created
        /// using this API.
        ///
        /// # Returns
        ///
        /// A new pointer to a clone of the value, which must be freed separately
        /// with the corresponding drop function.
        pub unsafe extern "C" fn $clone(value: *mut $ty) -> *mut $ty {
            unsafe { core::clone::Clone::clone(&*value).into_ffi() }
        }
    };
}

#[macro_export]
/// Implements the `IntoFFI` trait for a type by converting each field to its FFI representation.
///
/// This macro generates an implementation of `IntoFFI` where the FFI type is a struct
/// with the same field names as the original type. Each field is converted using its own
/// `IntoFFI` implementation.
///
/// # Arguments
///
/// * `$ty` - The Rust type to implement `IntoFFI` for
/// * `$ffi` - The corresponding FFI type
/// * `$param` - One or more field names to convert
///
/// # Example
///
/// ```ignore
/// ffi_struct!(MyStruct, MyStructFFI, field1, field2);
/// ```
macro_rules! ffi_struct {
    ($ty:ty,$ffi:ty,$($param:ident),*) => {
        impl $crate::IntoFFI for $ty {
            type FFI = $ffi;
            fn into_ffi(self) -> Self::FFI {
                Self::FFI {
                    $(
                        $param: $crate::IntoFFI::into_ffi(self.$param),
                    )*
                }
            }
        }
    };
}

#[macro_export]
macro_rules! ffi_enum {
    ($ty:ty,$ffi:ident,$($param:ident),*) => {
        #[repr(C)]
        pub enum $ffi{
            $(
                $param,
            )*
        }

        impl $crate::IntoFFI for $ty {
            type FFI = $ffi;
            fn into_ffi(self) -> Self::FFI {
                match self{
                    $(
                        <$ty>::$param => $ffi::$param,
                    )*
                }
            }
        }
    };
}

#[macro_export]
macro_rules! ffi_enum_with_default {
    ($ty:ty,$ffi:ident,$default:ident,$($param:ident),*) => {
        #[repr(C)]
        pub enum $ffi{
            $default,
            $(
                $param,
            )*
        }

        impl $crate::IntoFFI for $ty {
            type FFI = $ffi;
            fn into_ffi(self) -> Self::FFI {
                match self{
                    $(
                        <$ty>::$param => $ffi::$param,
                    )*
                    _ => $ffi::$default,
                }
            }
        }
    };
}

#[macro_export]
macro_rules! ffi_view {
    ($view_ty:ty,$ffi_ty:ty,$id:ident,$force_as:ident) => {
        /// # Safety
        /// This function is unsafe because it dereferences a raw pointer and performs unchecked downcasting.
        /// The caller must ensure that `view` is a valid pointer to an `AnyView` that contains the expected view type.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $force_as(view: *mut $crate::WuiAnyView) -> $ffi_ty {
            unsafe {
                let any: waterui::AnyView = $crate::IntoRust::into_rust(view);
                let view = (*any.downcast_unchecked::<$view_ty>());
                $crate::IntoFFI::into_ffi(view)
            }
        }
        $crate::ffi_view!($view_ty, $id);
    };

    ($view_ty:ty,$id:ident) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn $id() -> $crate::WuiTypeId {
            $crate::IntoFFI::into_ffi(core::any::TypeId::of::<$view_ty>())
        }
    };
}

#[macro_export]
macro_rules! ffi_type {
    ($name:ident,$ty:ty,$drop:ident) => {
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

        #[unsafe(no_mangle)]
        /// Drops the FFI value.
        ///
        /// # Safety
        ///
        /// The pointer must be a valid pointer to a properly initialized value
        /// of the expected type, and must not be used after this function is called.
        pub unsafe extern "C" fn $drop(value: *mut $name) {
            unsafe {
                let _ = $crate::IntoRust::into_rust(value);
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
