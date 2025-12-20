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
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
pub use type_id::WuiTypeId;
pub mod id;
pub mod reactive;
pub mod locale;
pub mod theme;
mod ty;
pub mod views;
use core::ptr::null_mut;

use alloc::boxed::Box;
use executor_core::{init_global_executor, init_local_executor};
use waterui::{AnyView, Str, View};
use waterui_core::Metadata;

pub mod app;
pub mod window;
use waterui_core::metadata::MetadataKey;

use crate::array::WuiArray;
#[macro_export]
macro_rules! export {
    () => {
        const _: () = {
            /// Initializes the WaterUI runtime and creates a default environment.
            ///
            /// Native should:
            /// 1. Call this once at startup
            /// 2. Install theme settings into the returned environment
            /// 3. Pass the environment to `waterui_app()`
            ///
            /// # Safety
            /// This function must be called on main thread, once only.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn waterui_init() -> *mut $crate::WuiEnv {
                unsafe {
                    $crate::__init();
                }
                let env = waterui::Environment::new();
                $crate::IntoFFI::into_ffi(env)
            }

            ::waterui::hot_reloadable_library!(app);

            /// Creates the application from the user's `app(env)` function.
            ///
            /// Takes ownership of the environment (with theme already installed) from native,
            /// calls the user's `app(env: Environment) -> App` function, and returns the App.
            ///
            /// The environment is returned inside the App struct for native to use during rendering.
            ///
            /// # Safety
            /// - `env` must be a valid pointer from `waterui_init()` or native env creation
            /// - This function takes ownership of the environment
            /// - This function must be called on main thread
            #[unsafe(no_mangle)]
            #[allow(unexpected_cfgs)]
            pub unsafe extern "C" fn waterui_app(env: *mut $crate::WuiEnv) -> $crate::app::WuiApp {
                // Take ownership of the environment
                let env: waterui::Environment = unsafe { $crate::IntoRust::into_rust(env) };

                // Call user's app(env: Environment) -> App
                let app: waterui::app::App = app(env);

                $crate::IntoFFI::into_ffi(app)
            }

            #[cfg(target_os = "android")]
            #[unsafe(no_mangle)]
            extern "system" fn JNI_OnLoad(
                vm: *mut core::ffi::c_void,
                _reserved: *mut core::ffi::c_void,
            ) -> i32 {
                unsafe { $crate::__android_init(vm) }
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
    // Forwards panics to tracing
    std::panic::set_hook(Box::new(|info| {
        tracing_panic::panic_hook(info);
    }));

    // Forwards tracing to platform's logging system
    #[cfg(target_os = "android")]
    {
        tracing_subscriber::registry()
            .with(tracing_android::layer("WaterUI").expect("Failed to create Android log layer"))
            .init();
    }

    #[cfg(target_vendor = "apple")]
    {
        tracing_subscriber::registry()
            .with(tracing_oslog::OsLogger::new("dev.waterui", "default"))
            .init();
    }

    #[cfg(not(any(target_os = "android", target_vendor = "apple")))]
    {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
    }

    init_global_executor(native_executor::NativeExecutor::new());
    init_local_executor(native_executor::NativeExecutor::new());
}

#[cfg(target_os = "android")]
pub unsafe fn __android_init(vm: *mut core::ffi::c_void) -> i32 {
    tracing::debug!("Initializing Android context for WaterUI FFI");
    unsafe {
        ndk_context::initialize_android_context(vm, core::ptr::null_mut());
    }

    jni::sys::JNI_VERSION_1_6
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

use crate::color::WuiColor;
use crate::reactive::WuiComputed;
use waterui::Color;
use waterui::background::Background;

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

// ========== Metadata<Transform> FFI ==========
// Used to apply 2D transforms (scale, rotation, translation) to views

use waterui::style::{Anchor, Offset, Rotation, Scale, Transform};

/// FFI-safe representation of a 2D transform.
/// All values are reactive (Computed) and can be animated.
#[repr(C)]
pub struct WuiTransform {
    /// Scale factor along X axis (1.0 = no scale)
    pub scale_x: *mut WuiComputed<f32>,
    /// Scale factor along Y axis (1.0 = no scale)
    pub scale_y: *mut WuiComputed<f32>,
    /// Rotation angle in degrees (positive = clockwise)
    pub rotation: *mut WuiComputed<f32>,
    /// Translation offset along X axis in points
    pub translate_x: *mut WuiComputed<f32>,
    /// Translation offset along Y axis in points
    pub translate_y: *mut WuiComputed<f32>,
}

#[allow(deprecated)]
impl IntoFFI for Transform {
    type FFI = WuiTransform;
    fn into_ffi(self) -> Self::FFI {
        WuiTransform {
            scale_x: self.scale_x.into_ffi(),
            scale_y: self.scale_y.into_ffi(),
            rotation: self.rotation.into_ffi(),
            translate_x: self.translate_x.into_ffi(),
            translate_y: self.translate_y.into_ffi(),
        }
    }
}

/// Type alias for Metadata<Transform> FFI struct
pub type WuiMetadataTransform = WuiMetadata<WuiTransform>;

// Generate waterui_metadata_transform_id() and waterui_force_as_metadata_transform()
#[allow(deprecated)]
ffi_metadata!(Transform, WuiMetadataTransform, transform);

// ========== Metadata<Scale> FFI ==========
// Used to apply scale transforms to views

/// FFI-safe representation of an anchor point.
/// Normalized coordinates: (0.0, 0.0) = top-left, (0.5, 0.5) = center, (1.0, 1.0) = bottom-right.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct WuiAnchor {
    /// X coordinate (0.0 = left, 0.5 = center, 1.0 = right)
    pub x: f32,
    /// Y coordinate (0.0 = top, 0.5 = center, 1.0 = bottom)
    pub y: f32,
}

impl IntoFFI for Anchor {
    type FFI = WuiAnchor;
    fn into_ffi(self) -> Self::FFI {
        WuiAnchor {
            x: self.x,
            y: self.y,
        }
    }
}

/// FFI-safe representation of a scale transform.
/// All values are reactive (Computed) and can be animated.
#[repr(C)]
pub struct WuiScale {
    /// Scale factor along X axis (1.0 = no scale)
    pub x: *mut WuiComputed<f32>,
    /// Scale factor along Y axis (1.0 = no scale)
    pub y: *mut WuiComputed<f32>,
    /// Anchor point for the scale transform
    pub anchor: WuiAnchor,
}

impl IntoFFI for Scale {
    type FFI = WuiScale;
    fn into_ffi(self) -> Self::FFI {
        WuiScale {
            x: self.x.into_ffi(),
            y: self.y.into_ffi(),
            anchor: self.anchor.into_ffi(),
        }
    }
}

/// Type alias for Metadata<Scale> FFI struct
pub type WuiMetadataScale = WuiMetadata<WuiScale>;

// Generate waterui_metadata_scale_id() and waterui_force_as_metadata_scale()
ffi_metadata!(Scale, WuiMetadataScale, scale);

// ========== Metadata<Rotation> FFI ==========
// Used to apply rotation transforms to views

/// FFI-safe representation of a rotation transform.
/// All values are reactive (Computed) and can be animated.
#[repr(C)]
pub struct WuiRotation {
    /// Rotation angle in degrees (positive = clockwise)
    pub angle: *mut WuiComputed<f32>,
    /// Anchor point for the rotation transform
    pub anchor: WuiAnchor,
}

impl IntoFFI for Rotation {
    type FFI = WuiRotation;
    fn into_ffi(self) -> Self::FFI {
        WuiRotation {
            angle: self.angle.into_ffi(),
            anchor: self.anchor.into_ffi(),
        }
    }
}

/// Type alias for Metadata<Rotation> FFI struct
pub type WuiMetadataRotation = WuiMetadata<WuiRotation>;

// Generate waterui_metadata_rotation_id() and waterui_force_as_metadata_rotation()
ffi_metadata!(Rotation, WuiMetadataRotation, rotation);

// ========== Metadata<Offset> FFI ==========
// Used to apply offset (translation) transforms to views

/// FFI-safe representation of an offset transform.
/// All values are reactive (Computed) and can be animated.
#[repr(C)]
pub struct WuiOffset {
    /// Offset along X axis in points
    pub x: *mut WuiComputed<f32>,
    /// Offset along Y axis in points
    pub y: *mut WuiComputed<f32>,
}

impl IntoFFI for Offset {
    type FFI = WuiOffset;
    fn into_ffi(self) -> Self::FFI {
        WuiOffset {
            x: self.x.into_ffi(),
            y: self.y.into_ffi(),
        }
    }
}

/// Type alias for Metadata<Offset> FFI struct
pub type WuiMetadataOffset = WuiMetadata<WuiOffset>;

// Generate waterui_metadata_offset_id() and waterui_force_as_metadata_offset()
ffi_metadata!(Offset, WuiMetadataOffset, offset);

// ========== Metadata<Blur> FFI ==========
// Used to apply blur filter to views

use waterui::filter::{Blur, Brightness, Contrast, Grayscale, HueRotation, Opacity, Saturation};

/// FFI-safe representation of a blur filter.
/// All values are reactive (Computed) and can be animated.
#[repr(C)]
pub struct WuiBlur {
    /// Blur radius in points (0 = no blur).
    pub radius: *mut WuiComputed<f32>,
}

impl IntoFFI for Blur {
    type FFI = WuiBlur;
    fn into_ffi(self) -> Self::FFI {
        WuiBlur {
            radius: self.radius.into_ffi(),
        }
    }
}

/// Type alias for Metadata<Blur> FFI struct
pub type WuiMetadataBlur = WuiMetadata<WuiBlur>;

// Generate waterui_metadata_blur_id() and waterui_force_as_metadata_blur()
ffi_metadata!(Blur, WuiMetadataBlur, blur);

// ========== Metadata<Brightness> FFI ==========
// Used to apply brightness filter to views

/// FFI-safe representation of a brightness filter.
/// All values are reactive (Computed) and can be animated.
#[repr(C)]
pub struct WuiBrightness {
    /// Brightness adjustment (0 = normal, negative = darker, positive = brighter).
    pub amount: *mut WuiComputed<f32>,
}

impl IntoFFI for Brightness {
    type FFI = WuiBrightness;
    fn into_ffi(self) -> Self::FFI {
        WuiBrightness {
            amount: self.amount.into_ffi(),
        }
    }
}

/// Type alias for Metadata<Brightness> FFI struct
pub type WuiMetadataBrightness = WuiMetadata<WuiBrightness>;

// Generate waterui_metadata_brightness_id() and waterui_force_as_metadata_brightness()
ffi_metadata!(Brightness, WuiMetadataBrightness, brightness);

// ========== Metadata<Saturation> FFI ==========
// Used to apply saturation filter to views

/// FFI-safe representation of a saturation filter.
/// All values are reactive (Computed) and can be animated.
#[repr(C)]
pub struct WuiSaturation {
    /// Saturation amount (0 = grayscale, 1 = normal, >1 = oversaturated).
    pub amount: *mut WuiComputed<f32>,
}

impl IntoFFI for Saturation {
    type FFI = WuiSaturation;
    fn into_ffi(self) -> Self::FFI {
        WuiSaturation {
            amount: self.amount.into_ffi(),
        }
    }
}

/// Type alias for Metadata<Saturation> FFI struct
pub type WuiMetadataSaturation = WuiMetadata<WuiSaturation>;

// Generate waterui_metadata_saturation_id() and waterui_force_as_metadata_saturation()
ffi_metadata!(Saturation, WuiMetadataSaturation, saturation);

// ========== Metadata<Contrast> FFI ==========
// Used to apply contrast filter to views

/// FFI-safe representation of a contrast filter.
/// All values are reactive (Computed) and can be animated.
#[repr(C)]
pub struct WuiContrast {
    /// Contrast amount (1 = normal, <1 = less contrast, >1 = more contrast).
    pub amount: *mut WuiComputed<f32>,
}

impl IntoFFI for Contrast {
    type FFI = WuiContrast;
    fn into_ffi(self) -> Self::FFI {
        WuiContrast {
            amount: self.amount.into_ffi(),
        }
    }
}

/// Type alias for Metadata<Contrast> FFI struct
pub type WuiMetadataContrast = WuiMetadata<WuiContrast>;

// Generate waterui_metadata_contrast_id() and waterui_force_as_metadata_contrast()
ffi_metadata!(Contrast, WuiMetadataContrast, contrast);

// ========== Metadata<HueRotation> FFI ==========
// Used to apply hue rotation filter to views

/// FFI-safe representation of a hue rotation filter.
/// All values are reactive (Computed) and can be animated.
#[repr(C)]
pub struct WuiHueRotation {
    /// Hue rotation angle in degrees (0-360).
    pub angle: *mut WuiComputed<f32>,
}

impl IntoFFI for HueRotation {
    type FFI = WuiHueRotation;
    fn into_ffi(self) -> Self::FFI {
        WuiHueRotation {
            angle: self.angle.into_ffi(),
        }
    }
}

/// Type alias for Metadata<HueRotation> FFI struct
pub type WuiMetadataHueRotation = WuiMetadata<WuiHueRotation>;

// Generate waterui_metadata_hue_rotation_id() and waterui_force_as_metadata_hue_rotation()
ffi_metadata!(HueRotation, WuiMetadataHueRotation, hue_rotation);

// ========== Metadata<Grayscale> FFI ==========
// Used to apply grayscale filter to views

/// FFI-safe representation of a grayscale filter.
/// All values are reactive (Computed) and can be animated.
#[repr(C)]
pub struct WuiGrayscale {
    /// Grayscale intensity (0 = full color, 1 = full grayscale).
    pub intensity: *mut WuiComputed<f32>,
}

impl IntoFFI for Grayscale {
    type FFI = WuiGrayscale;
    fn into_ffi(self) -> Self::FFI {
        WuiGrayscale {
            intensity: self.intensity.into_ffi(),
        }
    }
}

/// Type alias for Metadata<Grayscale> FFI struct
pub type WuiMetadataGrayscale = WuiMetadata<WuiGrayscale>;

// Generate waterui_metadata_grayscale_id() and waterui_force_as_metadata_grayscale()
ffi_metadata!(Grayscale, WuiMetadataGrayscale, grayscale);

// ========== Metadata<Opacity> FFI ==========
// Used to apply opacity filter to views

/// FFI-safe representation of an opacity filter.
/// All values are reactive (Computed) and can be animated.
#[repr(C)]
pub struct WuiOpacity {
    /// Opacity value (0 = transparent, 1 = opaque).
    pub value: *mut WuiComputed<f32>,
}

impl IntoFFI for Opacity {
    type FFI = WuiOpacity;
    fn into_ffi(self) -> Self::FFI {
        WuiOpacity {
            value: self.value.into_ffi(),
        }
    }
}

/// Type alias for Metadata<Opacity> FFI struct
pub type WuiMetadataOpacity = WuiMetadata<WuiOpacity>;

// Generate waterui_metadata_opacity_id() and waterui_force_as_metadata_opacity()
ffi_metadata!(Opacity, WuiMetadataOpacity, opacity);

// ========== Metadata<Focused> FFI ==========
// Used to track focus state for views

use crate::reactive::WuiBinding;
use waterui::component::focus::Focused;

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

// ========== Metadata<ClipShape> FFI ==========
// Used to clip views to shapes

use waterui::shape::{ClipShape, FilledShape, PathCommand};

/// FFI-safe representation of a path command.
/// All coordinates are normalized (0.0-1.0) and scale with view bounds.
#[repr(C)]
pub enum WuiPathCommand {
    /// Move to a position without drawing.
    MoveTo { x: f32, y: f32 },
    /// Draw a straight line to a position.
    LineTo { x: f32, y: f32 },
    /// Draw a quadratic bezier curve.
    QuadTo { cx: f32, cy: f32, x: f32, y: f32 },
    /// Draw a cubic bezier curve.
    CubicTo {
        c1x: f32,
        c1y: f32,
        c2x: f32,
        c2y: f32,
        x: f32,
        y: f32,
    },
    /// Draw an arc.
    Arc {
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        start: f32,
        sweep: f32,
    },
    /// Close the current subpath.
    Close,
}

impl IntoFFI for PathCommand {
    type FFI = WuiPathCommand;
    fn into_ffi(self) -> Self::FFI {
        match self {
            PathCommand::MoveTo { x, y } => WuiPathCommand::MoveTo { x, y },
            PathCommand::LineTo { x, y } => WuiPathCommand::LineTo { x, y },
            PathCommand::QuadTo { cx, cy, x, y } => WuiPathCommand::QuadTo { cx, cy, x, y },
            PathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => WuiPathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            },
            PathCommand::Arc {
                cx,
                cy,
                rx,
                ry,
                start,
                sweep,
            } => WuiPathCommand::Arc {
                cx,
                cy,
                rx,
                ry,
                start,
                sweep,
            },
            PathCommand::Close => WuiPathCommand::Close,
        }
    }
}

/// FFI-safe representation of a clip shape.
/// Contains the path commands that define the clipping mask.
#[repr(C)]
pub struct WuiClipShape {
    /// Array of path commands defining the shape.
    pub commands: WuiArray<WuiPathCommand>,
}

impl IntoFFI for ClipShape {
    type FFI = WuiClipShape;
    fn into_ffi(self) -> Self::FFI {
        let commands: alloc::vec::Vec<WuiPathCommand> = self
            .commands()
            .iter()
            .map(|cmd| cmd.into_ffi())
            .collect();
        WuiClipShape {
            commands: WuiArray::new(commands),
        }
    }
}

/// Type alias for Metadata<ClipShape> FFI struct
pub type WuiMetadataClipShape = WuiMetadata<WuiClipShape>;

// Generate waterui_metadata_clip_shape_id() and waterui_force_as_metadata_clip_shape()
ffi_metadata!(ClipShape, WuiMetadataClipShape, clip_shape);

// ========== FilledShape FFI ==========
// Shapes filled with color, rendered as native views

/// FFI-safe representation of a filled shape.
/// Contains the path commands and fill color.
#[repr(C)]
pub struct WuiFilledShape {
    /// Array of path commands defining the shape.
    pub commands: WuiArray<WuiPathCommand>,
    /// Fill color (opaque pointer to Color).
    pub fill: *mut WuiColor,
}

impl IntoFFI for FilledShape {
    type FFI = WuiFilledShape;
    fn into_ffi(self) -> Self::FFI {
        let commands: alloc::vec::Vec<WuiPathCommand> = self
            .commands()
            .iter()
            .map(|cmd| cmd.into_ffi())
            .collect();
        WuiFilledShape {
            commands: WuiArray::new(commands),
            fill: self.fill().clone().into_ffi(),
        }
    }
}

// Generate waterui_filled_shape_id() and waterui_force_as_filled_shape()
ffi_view!(FilledShape, WuiFilledShape, filled_shape);

// ========== Metadata<ContextMenu> FFI ==========
// Used to attach context menus to views

use crate::components::text::WuiText;
use core::cell::RefCell;
use waterui::metadata::context_menu::{ContextMenu, MenuItem};
use waterui_core::handler::{Handler, SharedHandler};

/// Wrapper around SharedHandler that provides interior mutability for FFI.
pub struct SharedActionWrapper(RefCell<SharedHandler<()>>);

opaque!(WuiSharedAction, SharedActionWrapper, shared_action);

/// Call a shared action with the given environment.
///
/// # Safety
/// * `action` must be a valid pointer to a `WuiSharedAction`.
/// * `env` must be a valid pointer to a `WuiEnv`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_shared_action(
    action: *const WuiSharedAction,
    env: *const WuiEnv,
) {
    unsafe {
        // WuiSharedAction wraps SharedActionWrapper which wraps RefCell<SharedHandler>
        // Access: (*action).0 -> SharedActionWrapper, .0 -> RefCell<SharedHandler>
        let wrapper = &(*action).0;
        let shared = wrapper.0.borrow().clone();

        // Use a wrapper struct that implements Handler and calls through to the Rc
        struct SharedHandlerCaller(SharedHandler<()>);
        impl Handler<()> for SharedHandlerCaller {
            fn handle(&mut self, env: &waterui_core::Environment) {
                // The shared handler's inner value is typically a closure that doesn't
                // actually need mutation, so we can safely cast to get &mut
                // This is safe because the underlying closures use Fn, not FnMut with state
                let ptr = alloc::rc::Rc::as_ptr(&self.0) as *mut dyn Handler<()>;
                unsafe { (*ptr).handle(env) };
            }
        }

        let mut caller = SharedHandlerCaller(shared);
        caller.handle(&*env);
    }
}

/// FFI-safe representation of a menu item.
#[repr(C)]
pub struct WuiMenuItem {
    /// The label for the menu item.
    pub label: WuiText,
    /// The action handler pointer (SharedHandler wrapped for FFI).
    pub action: *mut WuiSharedAction,
}

ffi_safe!(WuiMenuItem);

impl IntoFFI for MenuItem {
    type FFI = WuiMenuItem;
    fn into_ffi(self) -> Self::FFI {
        let wrapper = SharedActionWrapper(RefCell::new(self.action));
        WuiMenuItem {
            label: self.label.into_ffi(),
            action: wrapper.into_ffi(),
        }
    }
}

/// FFI-safe representation of a context menu.
#[repr(C)]
pub struct WuiContextMenu {
    /// The menu items as a computed array.
    pub items: *mut WuiComputed<MenuItems>,
}

/// Type alias for Vec<MenuItem> to use with ffi_computed! macro
type MenuItems = alloc::vec::Vec<MenuItem>;

impl IntoFFI for ContextMenu {
    type FFI = WuiContextMenu;
    fn into_ffi(self) -> Self::FFI {
        WuiContextMenu {
            items: self.items.into_ffi(),
        }
    }
}

/// Type alias for Metadata<ContextMenu> FFI struct
pub type WuiMetadataContextMenu = WuiMetadata<WuiContextMenu>;

// Generate waterui_metadata_context_menu_id() and waterui_force_as_metadata_context_menu()
ffi_metadata!(ContextMenu, WuiMetadataContextMenu, context_menu);

// Computed<Vec<MenuItem>> support
ffi_computed!(MenuItems, WuiArray<WuiMenuItem>, menu_items);
