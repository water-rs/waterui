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

#[cfg(all(
    feature = "std",
    not(target_arch = "wasm32"),
    not(waterui_enable_hot_reload)
))]
fn install_panic_hook() {
    // For non-hot-reload builds, initialize a simple tracing subscriber.
    // Hot reload builds use their own subscriber in hot_reload.rs.
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt::Subscriber::builder()
        .with_env_filter(filter)
        .without_time()
        .with_target(false)
        .try_init();

    // Route panics through tracing
    std::panic::set_hook(Box::new(tracing_panic::panic_hook));
}

#[cfg(all(
    feature = "std",
    not(target_arch = "wasm32"),
    waterui_enable_hot_reload
))]
fn install_panic_hook() {
    // Hot reload mode: Do NOT initialize subscriber here.
    // The subscriber is initialized in hot_reload.rs (install_tracing_forwarder)
    // with CLI forwarding support.
    //
    // We only set the panic hook to route panics through tracing.
    // The actual tracing subscriber will be set up by hot_reload later.
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

/// Gets the id of the anyview type as a 128-bit value for O(1) comparison.
#[unsafe(no_mangle)]
pub extern "C" fn waterui_anyview_id() -> WuiTypeId {
    WuiTypeId::of::<AnyView>()
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

/// Gets the id of a view as a 128-bit value for O(1) comparison.
///
/// - Normal build: Returns the view's `TypeId` (guaranteed unique)
/// - Hot reload: Returns 128-bit hash of `type_name()` (stable across dylibs)
///
/// # Safety
/// The caller must ensure that `view` is a valid pointer to a properly
/// initialized `WuiAnyView` instance and that it remains valid for the
/// duration of this function call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_id(view: *const WuiAnyView) -> WuiTypeId {
    unsafe {
        let view = &*view;
        WuiTypeId::from_runtime(view.type_id(), view.name())
    }
}

/// Gets the stretch axis of a view.
///
/// Returns the `StretchAxis` that indicates how this view stretches to fill
/// available space. For native views, this returns the layout behavior defined
/// by the `NativeView` trait. For non-native views, this will panic.
///
/// # Safety
/// The caller must ensure that `view` is a valid pointer to a properly
/// initialized `WuiAnyView` instance and that it remains valid for the
/// duration of this function call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_stretch_axis(
    view: *const WuiAnyView,
) -> crate::components::layout::WuiStretchAxis {
    unsafe { (&*view).stretch_axis().into() }
}

// ============================================================================
// WuiTypeId - Optimized type identifier for O(1) comparison
// ============================================================================

/// Type ID as a 128-bit value for O(1) comparison.
///
/// - Normal build: Uses `std::any::TypeId` (guaranteed unique by Rust)
/// - Hot reload: Uses 128-bit FNV-1a hash of `type_name()` (stable across dylib reloads)
///
/// The choice is controlled by the `waterui_enable_hot_reload` cfg flag.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WuiTypeId {
    pub low: u64,
    pub high: u64,
}

impl WuiTypeId {
    /// Creates a type ID from a type parameter.
    #[inline]
    pub fn of<T: 'static>() -> Self {
        #[cfg(waterui_enable_hot_reload)]
        {
            // Hash type_name for hot reload compatibility
            Self::from_type_name(core::any::type_name::<T>())
        }

        #[cfg(not(waterui_enable_hot_reload))]
        {
            // Use TypeId directly (128-bit, guaranteed unique)
            Self::from_type_id(core::any::TypeId::of::<T>())
        }
    }

    /// Creates a type ID from a TypeId (normal build only).
    #[cfg(not(waterui_enable_hot_reload))]
    #[inline]
    fn from_type_id(id: core::any::TypeId) -> Self {
        // TypeId is internally a u128 - transmute to access it
        // Safety: TypeId is repr(transparent) over u128 in current Rust
        let value: u128 = unsafe { core::mem::transmute(id) };
        Self {
            low: value as u64,
            high: (value >> 64) as u64,
        }
    }

    /// Creates a type ID from a type name string (hot reload build).
    #[cfg(waterui_enable_hot_reload)]
    #[inline]
    pub fn from_type_name(name: &str) -> Self {
        let hash = fnv1a_128(name.as_bytes());
        Self {
            low: hash as u64,
            high: (hash >> 64) as u64,
        }
    }

    /// Creates a type ID from a runtime TypeId and type name.
    /// Uses TypeId in normal builds, type name hash in hot reload builds.
    #[inline]
    pub fn from_runtime(type_id: core::any::TypeId, name: &'static str) -> Self {
        #[cfg(waterui_enable_hot_reload)]
        {
            let _ = type_id; // unused in hot reload
            Self::from_type_name(name)
        }

        #[cfg(not(waterui_enable_hot_reload))]
        {
            let _ = name; // unused in normal build
            Self::from_type_id(type_id)
        }
    }
}

/// 128-bit FNV-1a hash function.
///
/// FNV-1a is fast and has good distribution properties.
/// Using 128-bit output virtually eliminates collision risk
/// (birthday paradox threshold: ~10^19 entries).
#[cfg(waterui_enable_hot_reload)]
const fn fnv1a_128(bytes: &[u8]) -> u128 {
    // FNV-1a 128-bit constants
    const FNV_OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const FNV_PRIME: u128 = 0x0000000001000000000000000000013b;

    let mut hash = FNV_OFFSET;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u128;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}

// ============================================================================
// WuiStr - UTF-8 string for FFI
// ============================================================================

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

#[repr(C)]
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

// ========== Metadata<Environment> FFI ==========
// Used by WithEnv to pass a new environment to child views

/// Type alias for Metadata<Environment> FFI struct
/// Layout: { content: *mut WuiAnyView, value: *mut WuiEnv }
pub type WuiMetadataEnv = WuiMetadata<*mut WuiEnv>;

// Generate waterui_metadata_env_id() and waterui_force_as_metadata_env()
ffi_metadata!(waterui::Environment, WuiMetadataEnv, env);
