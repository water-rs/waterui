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
#[cfg(feature = "std")]
extern crate std;
#[macro_use]
mod macros;
pub mod action;
pub mod animation;
pub mod array;
pub mod closure;
pub mod color;
pub mod components;
pub mod event;
pub mod gesture;
pub mod id;
pub mod reactive;
pub mod theme;
mod ty;
pub mod views;
#[cfg(all(not(target_arch = "wasm32"), waterui_enable_hot_reload))]
use core::ffi::CStr;
use core::ptr::null_mut;

use alloc::boxed::Box;
use executor_core::{init_global_executor, init_local_executor};
use waterui::{AnyView, Str, View};
use waterui_core::Metadata;

use crate::array::WuiArray;

#[cfg(all(feature = "std", not(target_arch = "wasm32")))]
fn install_panic_hook() {
    // Route panics through tracing; pretty rendering lives in the CLI.
    std::panic::set_hook(Box::new(tracing_panic::panic_hook));
}

#[cfg(any(target_arch = "wasm32", not(feature = "std")))]
fn install_panic_hook() {}
#[macro_export]
macro_rules! export {
    () => {
        /// Initializes a new WaterUI environment
        ///
        /// # Safety
        ///
        /// This function must be called on main thread.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn waterui_init() -> *mut $crate::WuiEnv {
            unsafe {
                $crate::__init();
            }
            let env: waterui::Environment = init();
            $crate::IntoFFI::into_ffi(env)
        }

        /// Creates the main view for the WaterUI application
        ///
        /// # Safety
        /// This function must be called on main thread.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn waterui_main() -> *mut $crate::WuiAnyView {
            let view = main();

            #[cfg(waterui_enable_hot_reload)]
            let view = waterui::hot_reload::Hotreload::new(view);

            $crate::IntoFFI::into_ffi(AnyView::new(view))
        }
    };
}

/// # Safety
/// You have to ensure this is only called once, and on main thread.
#[doc(hidden)]
#[inline(always)]
pub unsafe fn __init() {
    install_panic_hook();
    #[cfg(target_os = "android")]
    unsafe {
        native_executor::android::register_android_main_thread()
            .expect("Failed to register Android main thread");
    }
    init_global_executor(native_executor::NativeExecutor::new());
    init_local_executor(native_executor::NativeExecutor::new());
}

/// Defines a trait for converting Rust types to FFI-compatible representations.
///
/// This trait is used to convert Rust types that are not directly FFI-compatible
/// into types that can be safely passed across the FFI boundary. Implementors
/// must specify an FFI-compatible type and provide conversion logic.
///
/// # Examples
///
/// ```ignore
/// impl IntoFFI for MyStruct {
///     type FFI = *mut MyStruct;
///     fn into_ffi(self) -> Self::FFI {
///         Box::into_raw(Box::new(self))
///     }
/// }
/// ```
pub trait IntoFFI: 'static {
    /// The FFI-compatible type that this Rust type converts to.
    type FFI: 'static;

    /// Converts this Rust type into its FFI-compatible representation.
    fn into_ffi(self) -> Self::FFI;
}

pub trait IntoNullableFFI: 'static {
    type FFI: 'static;
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

#[cfg(all(not(target_arch = "wasm32"), waterui_enable_hot_reload))]
#[unsafe(no_mangle)]
pub extern "C" fn waterui_configure_hot_reload_endpoint(host: *const core::ffi::c_char, port: u16) {
    use alloc::string::ToString;
    use core::ffi::CStr;
    if host.is_null() {
        return;
    }

    // SAFETY: host is expected to be a valid null-terminated string supplied by the caller.
    let host = unsafe { CStr::from_ptr(host) };
    if let Ok(value) = host.to_str() {
        waterui::hot_reload::configure_hot_reload_endpoint(value.to_string(), port);
    }
}

#[cfg(any(target_arch = "wasm32", not(waterui_enable_hot_reload)))]
#[unsafe(no_mangle)]
pub extern "C" fn waterui_configure_hot_reload_endpoint(
    _host: *const core::ffi::c_char,
    _port: u16,
) {
}

#[cfg(all(not(target_arch = "wasm32"), waterui_enable_hot_reload))]
#[unsafe(no_mangle)]
pub extern "C" fn waterui_configure_hot_reload_directory(path: *const core::ffi::c_char) {
    use alloc::string::ToString;
    use core::ffi::CStr;
    if path.is_null() {
        return;
    }
    let path = unsafe { CStr::from_ptr(path) };
    if let Ok(value) = path.to_str() {
        waterui::hot_reload::configure_hot_reload_directory(value.to_string());
    }
}

#[cfg(any(target_arch = "wasm32", not(waterui_enable_hot_reload)))]
#[unsafe(no_mangle)]
pub extern "C" fn waterui_configure_hot_reload_directory(_path: *const core::ffi::c_char) {}

/// Defines a marker trait for types that should be treated as opaque when crossing FFI boundaries.
///
/// Opaque types are typically used when the internal structure of a type is not relevant
/// to foreign code and only the Rust side needs to understand the full implementation details.
/// This trait automatically provides implementations of `IntoFFI` and `IntoRust` for
/// any type that implements it, handling conversion to and from raw pointers.
///
/// # Examples
///
/// ```ignore
/// struct MyInternalStruct {
///     data: Vec<u32>,
///     state: String,
/// }
///
/// // By marking this as OpaqueType, foreign code only needs to deal with opaque pointers
/// impl OpaqueType for MyInternalStruct {}
/// ```
pub trait OpaqueType: 'static {}

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
/// ```ignore
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

ffi_safe!(u8, u16, u32, u64, i8, i16, i32, i64, f32, f64, bool);

opaque!(WuiEnv, waterui::Environment, env);

opaque!(WuiAnyView, waterui::AnyView, anyview);

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
    env: *mut WuiEnv,
) -> *mut WuiAnyView {
    unsafe {
        let view = view.into_rust();
        let body = view.body(&*env);

        let body = AnyView::new(body);

        body.into_ffi()
    }
}

/// Gets the id of a view
///
/// # Safety
/// The caller must ensure that `view` is a valid pointer to a properly
/// initialized `WuiAnyView` instance and that it remains valid for the
/// duration of this function call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_id(view: *const WuiAnyView) -> WuiStr {
    unsafe { (&*view).name().into_ffi() }
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

impl IntoFFI for &'static str {
    type FFI = WuiStr;
    fn into_ffi(self) -> Self::FFI {
        WuiStr(WuiArray::new(Str::from_static(self)))
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
pub extern "C" fn waterui_anyview_id() -> WuiStr {
    core::any::type_name::<AnyView>().into_ffi()
}

pub struct WuiMetadata<T> {
    pub content: *mut WuiAnyView,
    pub value: T,
}

impl<T: IntoFFI> IntoFFI for Metadata<T> {
    type FFI = WuiMetadata<T::FFI>;
    fn into_ffi(self) -> Self::FFI {
        WuiMetadata {
            content: self.content.into_ffi(),
            value: self.value.into_ffi(),
        }
    }
}
