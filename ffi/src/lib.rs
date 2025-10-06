//! # WaterUI FFI
//!
//! This crate provides a set of traits and utilities for safely converting between
//! Rust types and FFI-compatible representations. It is designed to work in `no_std`
//! environments and provides a clean, type-safe interface for FFI operations.
//!
//! The core functionality includes:
//! - `IntoFFI` trait for converting Rust types to FFI-compatible representations
//! - `IntoRust` trait for safely converting FFI types back to Rust types
//! - Support for opaque type handling across FFI boundaries
//! - Array and closure utilities for FFI interactions
//!
//! This library aims to minimize the unsafe code needed when working with FFI while
//! maintaining performance and flexibility.

#![no_std]
extern crate alloc;
#[macro_use]
mod macros;
pub mod action;
pub mod animation;

pub mod array;
pub mod closure;
pub mod color;
pub mod components;
pub mod reactive;
mod ty;
use core::ptr::null_mut;

use alloc::boxed::Box;
use executor_core::{init_global_executor, init_local_executor};
pub use ty::*;
use waterui::{AnyView, Str, View};

use crate::array::WuiArray;
#[macro_export]
macro_rules! export {
    () => {
        /// Initializes a new WaterUI environment
        #[unsafe(no_mangle)]
        pub extern "C" fn waterui_init() -> *mut $crate::WuiEnv {
            $crate::__init_executor();
            let env: waterui::Environment = init();
            $crate::IntoFFI::into_ffi(env)
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn waterui_main_reloadble() -> *mut $crate::WuiAnyView {
            fn _inner() -> impl View {
                main()
            }

            let view = waterui::AnyView::new(_inner());

            $crate::IntoFFI::into_ffi(view)
        }

        /// Creates the main view for the WaterUI application
        #[unsafe(no_mangle)]
        pub extern "C" fn waterui_main() -> *mut $crate::WuiAnyView {
            /*unsafe {
                $crate::IntoFFI::into_ffi(waterui::AnyView::new(waterui::hot_reload!(
                    "waterui_main_reloadble"
                )))
            }*/

            waterui_main_reloadble()
        }
    };
}

#[doc(hidden)]
pub fn __init_executor() {
    init_global_executor(native_executor::NativeExecutor);
    init_local_executor(native_executor::NativeExecutor);
}

/// Defines a trait for converting Rust types to FFI-compatible representations.
///
/// This trait is used to convert Rust types that are not directly FFI-compatible
/// into types that can be safely passed across the FFI boundary. Implementors
/// must specify an FFI-compatible type and provide conversion logic.
///
/// # Examples
///
/// ```
/// impl IntoFFI for MyStruct {
///     type FFI = *mut MyStruct;
///     fn into_ffi(self) -> Self::FFI {
///         Box::into_raw(Box::new(self))
///     }
/// }
/// ```
pub trait IntoFFI {
    /// The FFI-compatible type that this Rust type converts to.
    type FFI;

    /// Converts this Rust type into its FFI-compatible representation.
    fn into_ffi(self) -> Self::FFI;
}

pub trait IntoNullableFFI {
    type FFI;
    fn into_ffi(self) -> Self::FFI;
    fn null() -> Self::FFI;
}

impl<T: IntoNullableFFI> IntoFFI for Option<T> {
    type FFI = T::FFI;

    fn into_ffi(self) -> Self::FFI {
        match self {
            Some(value) => value.into_ffi(),
            None => T::null(),
        }
    }
}

impl<T: IntoNullableFFI> IntoFFI for T {
    type FFI = T::FFI;

    fn into_ffi(self) -> Self::FFI {
        <T as IntoNullableFFI>::into_ffi(self)
    }
}

pub trait InvalidValue {
    fn invalid() -> Self;
}

/// Defines a marker trait for types that should be treated as opaque when crossing FFI boundaries.
///
/// Opaque types are typically used when the internal structure of a type is not relevant
/// to foreign code and only the Rust side needs to understand the full implementation details.
/// This trait automatically provides implementations of `IntoFFI` and `IntoRust` for
/// any type that implements it, handling conversion to and from raw pointers.
///
/// # Examples
///
/// ```
/// struct MyInternalStruct {
///     data: Vec<u32>,
///     state: String,
/// }
///
/// // By marking this as OpaqueType, foreign code only needs to deal with opaque pointers
/// impl OpaqueType for MyInternalStruct {}
/// ```
pub trait OpaqueType {}

impl<T: OpaqueType> IntoNullableFFI for T {
    type FFI = *mut T;
    fn into_ffi(self) -> Self::FFI {
        Box::into_raw(Box::new(self))
    }
    fn null() -> Self::FFI {
        null_mut()
    }
}

impl<T: OpaqueType> IntoRust for *mut T {
    type Rust = Option<T>;
    unsafe fn into_rust(self) -> Self::Rust {
        if self.is_null() {
            None
        } else {
            unsafe { Some(*Box::from_raw(self)) }
        }
    }
}
/// Defines a trait for converting FFI-compatible types back to native Rust types.
///
/// This trait is complementary to `IntoFFI` and is used to convert FFI-compatible
/// representations back into their original Rust types. This is typically used
/// when receiving data from FFI calls that need to be processed in Rust code.
///
/// # Safety
///
/// Implementations of this trait are inherently unsafe as they involve converting
/// raw pointers or other FFI-compatible types into Rust types, which requires
/// ensuring memory safety, proper ownership, and correct type interpretation.
///
/// # Examples
///
/// ```
/// impl IntoRust for *mut MyStruct {
///     type Rust = MyStruct;
///
///     unsafe fn into_rust(self) -> Self::Rust {
///         if self.is_null() {
///             panic!("Null pointer provided");
///         }
///         *Box::from_raw(self)
///     }
/// }
/// ```
pub trait IntoRust {
    /// The native Rust type that this FFI-compatible type converts to.
    type Rust;

    /// Converts this FFI-compatible type into its Rust equivalent.
    ///
    /// # Safety
    /// The caller must ensure that the FFI value being converted is valid and
    /// properly initialized. Improper use may lead to undefined behavior.
    unsafe fn into_rust(self) -> Self::Rust;
}

ffi_safe!(u8, i32, f32, f64, bool);

ffi_type!(WuiEnv, waterui::Environment, waterui_drop_env);

ffi_type!(WuiAnyView, waterui::AnyView, waterui_drop_any_view);

/// Creates a new environment instance
#[unsafe(no_mangle)]
pub extern "C" fn waterui_env_new() -> *mut WuiEnv {
    let env = waterui::Environment::new();
    env.into_ffi()
}

/// Clones an existing environment instance
///
/// # Safety
/// The caller must ensure that `env` is a valid pointer to a properly initialized
/// `waterui::Environment` instance and that the environment remains valid for the
/// duration of this function call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_clone_env(env: *const WuiEnv) -> *mut WuiEnv {
    unsafe { (*env).clone().into_ffi() }
}

/// Gets the body of a view given the environment
///
/// # Safety
/// The caller must ensure that both `view` and `env` are valid pointers to properly
/// initialized instances and that they remain valid for the duration of this function call.
/// The `view` pointer will be consumed and should not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_body(
    view: *mut WuiAnyView,
    env: *mut waterui::Environment,
) -> *mut WuiAnyView {
    unsafe {
        let view = view.into_rust();
        let body = view.body(&*env);

        let body = AnyView::new(body);

        body.into_ffi()
    }
}

/// Gets the type ID of a view
///
/// # Safety
/// The caller must ensure that `view` is a valid pointer to a properly
/// initialized `WuiAnyView` instance and that it remains valid for the
/// duration of this function call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_id(view: *const WuiAnyView) -> WuiTypeId {
    unsafe { (&*view).type_id().into_ffi() }
}

// UTF-8 string represented as a byte array
#[repr(C)]
pub struct WuiStr(WuiArray<u8>);

impl IntoFFI for Str {
    type FFI = WuiStr;
    fn into_ffi(self) -> Self::FFI {
        WuiStr(WuiArray::new(self))
    }
}

impl IntoRust for WuiStr {
    type Rust = Str;
    unsafe fn into_rust(self) -> Self::Rust {
        let bytes = unsafe { self.0.into_rust() };
        // Safety: We assume the input bytes are valid UTF-8
        unsafe { Str::from_utf8_unchecked(bytes) }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn waterui_empty_anyview() -> *mut WuiAnyView {
    AnyView::default().into_ffi()
}

#[unsafe(no_mangle)]
pub extern "C" fn waterui_anyview_id() -> WuiTypeId {
    core::any::TypeId::of::<AnyView>().into_ffi()
}
