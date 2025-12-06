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
mod type_id;
pub use type_id::WuiTypeId;
pub mod id;
pub mod reactive;
pub mod theme;
mod ty;
pub mod views;
use core::ptr::null_mut;

use alloc::boxed::Box;
use executor_core::{init_global_executor, init_local_executor};
use waterui::{AnyView, Str, View};
use waterui_core::Metadata;
use waterui_core::metadata::MetadataKey;

use crate::array::WuiArray;
#[macro_export]
macro_rules! export {
    () => {
        const _: () = {
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

            #[cfg(waterui_hot_reload_lib)]
            ::waterui::debug::hot_reloadable_library!(main);

            /// Creates the main view for the WaterUI application
            ///
            /// # Safety
            /// This function must be called on main thread.
            #[unsafe(no_mangle)]
            #[allow(unexpected_cfgs)]
            pub unsafe extern "C" fn waterui_main() -> *mut $crate::WuiAnyView {
                let view = main();

                #[cfg(all(not(target_arch = "wasm32"), debug_assertions))]
                let view = waterui::debug::hot_reload::Hotreload::with_compile_env(view);
                $crate::IntoFFI::into_ffi(AnyView::new(view))
            }
        };
    };
}

/// # Safety
/// You have to ensure this is only called once, and on main thread.
#[doc(hidden)]
#[inline(always)]
pub unsafe fn __init() {
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

// Hot reload configuration FFI functions are in hot_reload.rs

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

// WuiTypeId is defined in hot_reload.rs and re-exported from crate root

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

impl<T: IntoFFI + MetadataKey> IntoFFI for Metadata<T> {
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

// ========== Metadata<Secure> FFI ==========
// Used to mark views as secure (prevent screenshots)

use waterui::metadata::secure::Secure;

/// C-compatible empty marker struct for Secure metadata.
/// This is needed because `()` (unit type) is not representable in C.
#[repr(C)]
pub struct WuiSecureMarker {
    /// Placeholder field to ensure struct has valid size in C.
    /// The actual value is meaningless - Secure is just a marker type.
    _marker: u8,
}

impl IntoFFI for Secure {
    type FFI = WuiSecureMarker;
    fn into_ffi(self) -> Self::FFI {
        WuiSecureMarker { _marker: 0 }
    }
}

/// Type alias for Metadata<Secure> FFI struct
/// Layout: { content: *mut WuiAnyView, value: WuiSecureMarker }
pub type WuiMetadataSecure = WuiMetadata<WuiSecureMarker>;

// Generate waterui_metadata_secure_id() and waterui_force_as_metadata_secure()
ffi_metadata!(Secure, WuiMetadataSecure, secure);

// ========== Metadata<GestureObserver> FFI ==========
// Used to attach gesture recognizers to views

use crate::gesture::WuiGestureObserver;
use waterui::gesture::GestureObserver;

/// Type alias for Metadata<GestureObserver> FFI struct
pub type WuiMetadataGesture = WuiMetadata<WuiGestureObserver>;

// Generate waterui_metadata_gesture_id() and waterui_force_as_metadata_gesture()
ffi_metadata!(GestureObserver, WuiMetadataGesture, gesture);

// ========== Metadata<OnEvent> FFI ==========
// Used to attach lifecycle event handlers (appear/disappear)

use crate::event::WuiOnEvent;
use waterui_core::event::OnEvent;

/// Type alias for Metadata<OnEvent> FFI struct
pub type WuiMetadataOnEvent = WuiMetadata<WuiOnEvent>;

// Generate waterui_metadata_on_event_id() and waterui_force_as_metadata_on_event()
ffi_metadata!(OnEvent, WuiMetadataOnEvent, on_event);

// ========== Metadata<Background> FFI ==========
// Used to apply background colors or images to views

use crate::color::{WuiColor, WuiResolvedColor};
use crate::reactive::WuiComputed;
use waterui::Color;
use waterui::background::Background;
use waterui_color::ResolvedColor;

/// FFI-safe representation of a background.
#[repr(C)]
pub enum WuiBackground {
    /// A solid color background.
    Color { color: *mut WuiComputed<Color> },
    /// An image background.
    Image { image: *mut WuiComputed<Str> },
}

impl IntoFFI for Background {
    type FFI = WuiBackground;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Background::Color(color) => WuiBackground::Color {
                color: color.into_ffi(),
            },
            Background::Image(image) => WuiBackground::Image {
                image: image.into_ffi(),
            },
            _ => unimplemented!(),
        }
    }
}

/// Type alias for Metadata<Background> FFI struct
pub type WuiMetadataBackground = WuiMetadata<WuiBackground>;

// Generate waterui_metadata_background_id() and waterui_force_as_metadata_background()
ffi_metadata!(Background, WuiMetadataBackground, background);

// ========== Metadata<ForegroundColor> FFI ==========
// Used to set foreground/text color for views

use waterui::background::ForegroundColor;

/// FFI-safe representation of a foreground color.
#[repr(C)]
pub struct WuiForegroundColor {
    /// Pointer to the computed color.
    pub color: *mut WuiComputed<Color>,
}

impl IntoFFI for ForegroundColor {
    type FFI = WuiForegroundColor;
    fn into_ffi(self) -> Self::FFI {
        WuiForegroundColor {
            color: self.color.into_ffi(),
        }
    }
}

/// Type alias for Metadata<ForegroundColor> FFI struct
pub type WuiMetadataForeground = WuiMetadata<WuiForegroundColor>;

// Generate waterui_metadata_foreground_id() and waterui_force_as_metadata_foreground()
ffi_metadata!(ForegroundColor, WuiMetadataForeground, foreground);

// ========== Metadata<Shadow> FFI ==========
// Used to apply shadow effects to views

use waterui::style::Shadow;

/// FFI-safe representation of a shadow.
#[repr(C)]
pub struct WuiShadow {
    /// Shadow color (as opaque pointer - needs environment to resolve).
    pub color: *mut WuiColor,
    /// Horizontal offset.
    pub offset_x: f32,
    /// Vertical offset.
    pub offset_y: f32,
    /// Blur radius.
    pub radius: f32,
}

impl IntoFFI for Shadow {
    type FFI = WuiShadow;
    fn into_ffi(self) -> Self::FFI {
        WuiShadow {
            color: self.color.into_ffi(),
            offset_x: self.offset.x,
            offset_y: self.offset.y,
            radius: self.radius,
        }
    }
}

/// Type alias for Metadata<Shadow> FFI struct
pub type WuiMetadataShadow = WuiMetadata<WuiShadow>;

// Generate waterui_metadata_shadow_id() and waterui_force_as_metadata_shadow()
ffi_metadata!(Shadow, WuiMetadataShadow, shadow);

// ========== Metadata<Focused> FFI ==========
// Used to track focus state for views

use crate::reactive::WuiBinding;
use waterui::component::focu::Focused;

/// FFI-safe representation of focused state.
#[repr(C)]
pub struct WuiFocused {
    /// Binding to the focus state (true = focused).
    pub binding: *mut WuiBinding<bool>,
}

impl IntoFFI for Focused {
    type FFI = WuiFocused;
    fn into_ffi(self) -> Self::FFI {
        WuiFocused {
            binding: self.0.into_ffi(),
        }
    }
}

/// Type alias for Metadata<Focused> FFI struct
pub type WuiMetadataFocused = WuiMetadata<WuiFocused>;

// Generate waterui_metadata_focused_id() and waterui_force_as_metadata_focused()
ffi_metadata!(Focused, WuiMetadataFocused, focused);

// ========== Metadata<IgnoreSafeArea> FFI ==========
// Used to extend views beyond safe area insets

use waterui_layout::IgnoreSafeArea;

/// FFI-safe representation of edge set for safe area.
#[repr(C)]
pub struct WuiEdgeSet {
    /// Ignore safe area on top edge.
    pub top: bool,
    /// Ignore safe area on leading edge.
    pub leading: bool,
    /// Ignore safe area on bottom edge.
    pub bottom: bool,
    /// Ignore safe area on trailing edge.
    pub trailing: bool,
}

impl IntoFFI for waterui_layout::EdgeSet {
    type FFI = WuiEdgeSet;
    fn into_ffi(self) -> Self::FFI {
        WuiEdgeSet {
            top: self.top,
            leading: self.leading,
            bottom: self.bottom,
            trailing: self.trailing,
        }
    }
}

/// FFI-safe representation of IgnoreSafeArea.
#[repr(C)]
pub struct WuiIgnoreSafeArea {
    /// Which edges should ignore safe area.
    pub edges: WuiEdgeSet,
}

impl IntoFFI for IgnoreSafeArea {
    type FFI = WuiIgnoreSafeArea;
    fn into_ffi(self) -> Self::FFI {
        WuiIgnoreSafeArea {
            edges: self.edges.into_ffi(),
        }
    }
}

/// Type alias for Metadata<IgnoreSafeArea> FFI struct
pub type WuiMetadataIgnoreSafeArea = WuiMetadata<WuiIgnoreSafeArea>;

// Generate waterui_metadata_ignore_safe_area_id() and waterui_force_as_metadata_ignore_safe_area()
ffi_metadata!(IgnoreSafeArea, WuiMetadataIgnoreSafeArea, ignore_safe_area);

// ========== Metadata<Retain> FFI ==========
// Used to keep values alive for the lifetime of a view (e.g., watcher guards)

use waterui_core::Retain;

/// FFI-safe representation of Retain metadata.
/// The actual retained value is opaque - renderers just need to keep it alive.
#[repr(C)]
pub struct WuiRetain {
    /// Opaque pointer to the retained value (Box<dyn Any>).
    /// This must be kept alive and dropped when the view is disposed.
    _opaque: *mut (),
}

impl IntoFFI for Retain {
    type FFI = WuiRetain;
    fn into_ffi(self) -> Self::FFI {
        // Leak the Retain to keep the inner value alive
        // The native side will call waterui_drop_retain to clean up
        let boxed = Box::new(self);
        WuiRetain {
            _opaque: Box::into_raw(boxed) as *mut (),
        }
    }
}

/// Type alias for Metadata<Retain> FFI struct
pub type WuiMetadataRetain = WuiMetadata<WuiRetain>;

// Generate waterui_metadata_retain_id() and waterui_force_as_metadata_retain()
ffi_metadata!(Retain, WuiMetadataRetain, retain);

/// Drops the retained value.
///
/// # Safety
/// The caller must ensure that `retain` is a valid pointer returned from
/// `waterui_force_as_metadata_retain` and has not been dropped before.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_retain(retain: WuiRetain) {
    if !retain._opaque.is_null() {
        unsafe {
            drop(Box::from_raw(retain._opaque as *mut Retain));
        }
    }
}
