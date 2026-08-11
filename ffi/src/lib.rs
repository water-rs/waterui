//! # `WaterUI` FFI
//!
//! This crate provides a set of traits and utilities for safely converting between
//! Rust types and FFI-compatible representations with a clean, type-safe interface.
//!
//! The core functionality includes:
//! - `IntoFFI` trait for converting Rust types to FFI-compatible representations
//! - `IntoRust` trait for safely converting FFI types back to Rust types
//! - Support for opaque type handling across FFI boundaries
//! - Array and closure utilities for FFI interactions
//!
//! This library aims to minimize the unsafe code needed when working with FFI while
//! maintaining performance and flexibility.

#![cfg_attr(not(feature = "std"), no_std)]
#[cfg(not(feature = "std"))]
compile_error!("waterui-ffi requires the `std` feature.");
#[cfg(all(feature = "c-api", feature = "android-jni"))]
compile_error!("waterui-ffi features `c-api` and `android-jni` are mutually exclusive");
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;
#[macro_use]
mod macros;

// JNI module for Android backend (when android-jni feature is enabled)
#[cfg(feature = "android-jni")]
pub mod jni;

mod bridge;
mod drawing;
mod events;
mod reactivity;
mod runtime;
mod ty;

/// C ABI exports for every bridged `WaterUI` component family.
pub mod components;
pub use bridge::{action, array, closure, locale};
use core::ptr::null_mut;
pub use drawing::{color, gradient, shape};
pub use events::{animation, cursor, drag_drop, event, gesture};
pub use reactivity::reactive;
pub use runtime::{app, id, theme, views, window};
pub use ty::WuiTypeId;

use alloc::boxed::Box;
use executor_core::{init_global_executor, init_local_executor};
#[cfg(target_vendor = "apple")]
use waterkit_audio as _;
use waterui::{AnyView, Str, View};
use waterui_core::Metadata;
pub use waterui_video;
#[cfg(all(not(target_vendor = "apple"), feature = "gpu"))]
pub use waterui_video_gpu;

/// Installs the GPU video pipeline where it is the platform's video backend.
///
/// Called from the `export!`-generated `waterui_init`. This lives in a
/// function (not in the macro body) so the `gpu` feature is resolved against
/// this crate's features rather than the app crate's.
#[doc(hidden)]
#[cfg_attr(
    any(target_vendor = "apple", not(feature = "gpu")),
    expect(
        clippy::missing_const_for_fn,
        reason = "the body is platform/feature-dependent; the non-Apple GPU arm installs at runtime"
    )
)]
pub fn __install_gpu_video(env: &mut waterui::Environment) {
    #[cfg(all(not(target_vendor = "apple"), feature = "gpu"))]
    waterui_video_gpu::install(env);
    #[cfg(any(target_vendor = "apple", not(feature = "gpu")))]
    let _ = env;
}

use waterui_core::metadata::MetadataKey;

use crate::array::WuiArray;

/// Reborrows a raw pointer as a shared reference.
///
/// # Safety
///
/// `ptr` must be non-null and point to a valid, initialized `T` that stays
/// alive and unaliased for the duration of the returned `'a` borrow.
#[inline]
pub const unsafe fn borrow_ffi<'a, T>(ptr: *const T) -> &'a T {
    // SAFETY: this function's own contract, stated above, is exactly the requirement
    // the dereference has: a non-null, initialized `T` alive for the returned borrow.
    unsafe { &*ptr }
}

/// Reborrows a raw pointer as an exclusive reference.
///
/// # Safety
///
/// `ptr` must be non-null and point to a valid, initialized `T` that stays
/// alive and exclusively borrowed for the duration of the returned `'a` borrow.
#[inline]
pub unsafe fn borrow_ffi_mut<'a, T>(ptr: *mut T) -> &'a mut T {
    // SAFETY: as for `borrow_ffi`, with the caller additionally promising the borrow
    // is exclusive for its lifetime.
    unsafe { &mut *ptr }
}

/// Generates the FFI entry points (`waterui_init`, `waterui_app`, and the
/// Android variants) for the application crate that calls it.
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
                let inspector = unsafe { $crate::__init() };
                let mut env = waterui::configure_environment!(waterui::Environment::new());
                waterui::inspector::install(&mut env, inspector);
                $crate::__install_gpu_video(&mut env);
                $crate::__configure_browser_environment(&mut env);
                $crate::IntoFFI::into_ffi(env)
            }

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

                let app: waterui::app::App = app(env);

                $crate::IntoFFI::into_ffi(app)
            }

            /// Android JNI entry point with an ABI that exposes only the two
            /// opaque handles owned by the single-activity backend.
            #[cfg(target_os = "android")]
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn waterui_android_app(
                env: *mut core::ffi::c_void,
            ) -> $crate::app::WuiAndroidAppHandles {
                unsafe { waterui_app(env.cast()) }.into_android_handles()
            }

            /// Android JNI initialization entry point using an opaque handle.
            #[cfg(target_os = "android")]
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn waterui_android_init() -> *mut core::ffi::c_void {
                unsafe { waterui_init() }.cast()
            }

            #[cfg(target_os = "android")]
            #[unsafe(no_mangle)]
            extern "system" fn JNI_OnLoad(
                vm: *mut core::ffi::c_void,
                _reserved: *mut core::ffi::c_void,
            ) -> i32 {
                // Initialize JNI module (cached classes, JavaVM reference)
                // Note: We use __jni_init which is conditionally defined based on
                // the android-jni feature in waterui-ffi crate (not the calling crate).
                unsafe { $crate::__jni_init(vm) }
            }
        };
    };
}

/// Installs optional packaged browser runtimes selected by the generated FFI crate.
#[doc(hidden)]
#[inline]
pub const fn __configure_browser_environment(env: &mut waterui::Environment) {
    #[cfg(any(feature = "cef-runtime", feature = "cef-header"))]
    components::platform::browser_cef::configure_environment(env);
    #[cfg(not(any(feature = "cef-runtime", feature = "cef-header")))]
    let _ = env;
}

/// JNI initialization helper for Android.
/// This is called from JNI_OnLoad in the export! macro.
///
/// The `android-jni` feature is mandatory for Android exports.
///
/// # Safety
/// Must only be called once from JNI_OnLoad.
#[doc(hidden)]
#[cfg(all(target_os = "android", feature = "android-jni"))]
#[inline]
pub unsafe fn __jni_init(vm: *mut core::ffi::c_void) -> i32 {
    unsafe { jni::init(vm as *mut jni::jni::sys::JavaVM) }
}

#[cfg(all(target_os = "android", not(feature = "android-jni")))]
compile_error!("waterui-ffi requires the `android-jni` feature when targeting Android");

/// JNI initialization stub for non-Android platforms.
#[doc(hidden)]
#[cfg(not(target_os = "android"))]
#[inline]
pub const unsafe fn __jni_init(_vm: *mut core::ffi::c_void) -> i32 {
    0x0001_0006
}

/// # Safety
/// You have to ensure this is only called once, and on main thread.
#[doc(hidden)]
#[inline]
#[must_use]
pub unsafe fn __init() -> Option<waterui::inspector::InspectorRuntime> {
    // SAFETY: `__init_impl` is generated by the `export!` macro in the app crate and
    // shares this entry point's contract: it runs once, on the platform main thread,
    // before any other FFI call.
    unsafe { __init_impl() }
}

/// # Safety
/// Must run on the platform main thread exactly once.
unsafe fn __init_impl() -> Option<waterui::inspector::InspectorRuntime> {
    #[cfg(target_os = "android")]
    unsafe {
        native_executor::android::register_android_main_thread()
            .expect("Failed to register Android main thread");
    }
    let inspector = waterui::inspector::maybe_init_from_env();

    #[cfg(feature = "std")]
    {
        // Forwards panics to tracing
        std::panic::set_hook(Box::new(|info| {
            tracing_panic::panic_hook(info);
        }));

        init_tracing(
            inspector
                .as_ref()
                .map(waterui::inspector::InspectorRuntime::tracing_layer),
        );
    }

    init_global_executor(native_executor::NativeExecutor::new());
    // The FFI entry point runs under a real platform main thread (the Apple main
    // queue, or Android's looper after `register_android_main_thread`). Failing
    // here names the missing setup, instead of panicking at the first spawn.
    let main_executor = native_executor::NativeMainExecutor::new().expect(
        "waterui_init requires a platform main thread; on Android call \
         register_android_main_thread on the UI thread first, and on a host that \
         owns its own event loop install that loop's LocalExecutor instead",
    );
    init_local_executor(waterui::task::monitored_local_executor_with_probes(
        main_executor,
        inspector
            .as_ref()
            .map(waterui::inspector::InspectorRuntime::runtime_probe),
    ));

    inspector
}

/// Sends `tracing` records to the platform logger, and to the inspector when one
/// is attached.
///
/// The three platform arms differ only in which logger they attach, so the
/// inspector layer is wired once here rather than in each of them.
#[cfg(feature = "std")]
fn init_tracing(inspector: Option<waterui::inspector::InspectorLayer>) {
    {
        // Forwards tracing to platform's logging system
        #[cfg(target_os = "android")]
        {
            use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

            let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
                .or_else(|_| {
                    tracing_subscriber::EnvFilter::try_new(
                        "error,wgpu_core=error,wgpu_hal=error,naga=error,jni=error",
                    )
                })
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("error"));

            tracing_subscriber::registry()
                .with(env_filter)
                .with(
                    tracing_android::layer("WaterUI").expect("Failed to create Android log layer"),
                )
                .with(inspector)
                .init();
        }

        #[cfg(target_vendor = "apple")]
        {
            use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

            let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
                .or_else(|_| {
                    tracing_subscriber::EnvFilter::try_new(
                        "error,wgpu_core=error,wgpu_hal=error,naga=error,metal=error",
                    )
                })
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("error"));

            tracing_subscriber::registry()
                .with(env_filter)
                .with(tracing_oslog::OsLogger::new("dev.waterui", "default"))
                .with(inspector)
                .init();
        }

        #[cfg(not(any(target_os = "android", target_vendor = "apple")))]
        {
            use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

            tracing_subscriber::registry()
                .with(tracing_subscriber::EnvFilter::from_default_env())
                .with(tracing_subscriber::fmt::layer())
                .with(inspector)
                .init();
        }
    }
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

/// Conversion to an FFI representation that has a dedicated null value,
/// letting `Option<T>` cross the FFI boundary without an extra wrapper.
pub trait IntoNullableFFI: 'static {
    /// The FFI-compatible type that this Rust type converts to.
    type FFI: 'static;
    /// Converts this Rust type into its FFI-compatible representation.
    fn into_ffi(self) -> Self::FFI;
    /// Returns the FFI value representing `None`.
    fn null() -> Self::FFI;
}

impl<T: IntoNullableFFI> IntoFFI for Option<T> {
    type FFI = T::FFI;

    fn into_ffi(self) -> Self::FFI {
        self.map_or_else(T::null, <T as IntoNullableFFI>::into_ffi)
    }
}

impl<T: IntoNullableFFI> IntoFFI for T {
    type FFI = T::FFI;

    fn into_ffi(self) -> Self::FFI {
        <T as IntoNullableFFI>::into_ffi(self)
    }
}

/// Types with a designated sentinel value that the C ABI treats as invalid or
/// absent.
pub trait InvalidValue {
    /// Returns the sentinel value treated as invalid/absent by the FFI.
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
            // SAFETY: the caller contract makes `self` an owning pointer from the
            // matching FFI constructor, so reclaiming the box frees it
            // exactly once.
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

opaque!(WuiAnyView, waterui::AnyView, anyview, any());

/// Visual presentation mode for the label slot of every control.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub enum WuiLabelDisplayMode {
    /// Use the label's own preferred presentation.
    Automatic,
    /// Show both title and icon when an icon is available.
    TitleAndIcon,
    /// Show only the title text.
    TitleOnly,
    /// Show only the icon when an icon is available.
    IconOnly,
    /// Visually hide the label entirely. The accessibility tree still
    /// announces [`WuiLabel::accessibility_label`].
    Hidden,
}

impl crate::IntoFFI for waterui_controls::label::LabelDisplayMode {
    type FFI = WuiLabelDisplayMode;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Self::Automatic => WuiLabelDisplayMode::Automatic,
            Self::TitleAndIcon => WuiLabelDisplayMode::TitleAndIcon,
            Self::TitleOnly => WuiLabelDisplayMode::TitleOnly,
            Self::IconOnly => WuiLabelDisplayMode::IconOnly,
            Self::Hidden => WuiLabelDisplayMode::Hidden,
        }
    }
}

/// FFI surface for the label slot of every control.
///
/// Bundles the three pieces backends need: the visual view to render, the
/// accessibility text to announce regardless of visual mode, and the visual
/// mode itself. The `view` field is always non-null but renders to an empty
/// view when `display_mode` is [`WuiLabelDisplayMode::Hidden`].
#[repr(C)]
#[derive(Debug)]
pub struct WuiLabel {
    /// Visual chrome rendered by the backend. When `display_mode` is
    /// `Hidden` the rendered view collapses to an empty zero-size view.
    pub view: *mut WuiAnyView,
    /// Spoken accessibility text. Always non-null; backends should bind
    /// this to platform accessibility APIs (`VoiceOver`, `TalkBack`, etc.).
    pub accessibility_label: *mut crate::reactive::WuiComputed<waterui_text::styled::StyledStr>,
    /// Visual presentation mode.
    pub display_mode: WuiLabelDisplayMode,
}

impl crate::IntoFFI for waterui_controls::label::Label {
    type FFI = WuiLabel;
    fn into_ffi(self) -> Self::FFI {
        let accessibility_label = self.accessibility_label();
        let display_mode = self.display_mode_preference().into_ffi();
        let view = waterui::AnyView::new(self).into_ffi();
        WuiLabel {
            view,
            accessibility_label: accessibility_label.into_ffi(),
            display_mode,
        }
    }
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
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let env = unsafe { borrow_ffi(env) };
    env.0.clone().into_ffi()
}

/// Returns the disabled state in force at this point in the view tree.
///
/// Disabled state is a scoped subtree attribute installed by `.disabled(...)`,
/// never a field on an individual control's configuration. Every interactive
/// control reads it here, from the same environment it already receives through
/// [`waterui_view_body`], and renders as inactive and ignores input while the
/// signal is `true`.
///
/// Always returns a non-null signal; it reads `false` when no `.disabled(...)`
/// scope encloses the view. Returns a new reference to the signal — the caller
/// must drop it when done.
///
/// # Safety
/// The caller must ensure that `env` is a valid pointer to a properly initialized
/// `waterui::Environment` instance and that the environment remains valid for the
/// duration of this function call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_disabled(
    env: *const WuiEnv,
) -> *mut crate::reactive::WuiComputed<bool> {
    // SAFETY: the caller guarantees `env` is a valid, live environment pointer
    // for the duration of this call (see the function-level safety contract).
    let env = unsafe { borrow_ffi(env) };
    env.0
        .get::<waterui_core::interaction::Disabled>()
        .map_or_else(
            || waterui::Computed::constant(false),
            |scope| scope.signal().clone(),
        )
        .into_ffi()
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
    // SAFETY: the caller contract requires `env` to be a valid handle, alive and not
    // otherwise borrowed for this call; the exclusive borrow ends here.
    let env = unsafe { borrow_ffi_mut(env) };
    // SAFETY: the caller contract makes `view` an owning handle from the matching
    // FFI constructor; it is consumed here and not observed again.
    let view = unsafe { view.into_rust() };
    let body = view.body(env);
    AnyView::new(body).into_ffi()
}

/// Gets the id of a view as a 128-bit value for O(1) comparison.
///
/// Returns the view's `TypeId` (guaranteed unique within a single binary).
///
/// # Safety
/// The caller must ensure that `view` is a valid pointer to a properly
/// initialized `WuiAnyView` instance and that it remains valid for the
/// duration of this function call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_id(view: *const WuiAnyView) -> WuiTypeId {
    // SAFETY: the caller contract requires `view` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let view = unsafe { borrow_ffi(view) };
    WuiTypeId::from_runtime(view.type_id(), view.name())
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
    // SAFETY: the caller contract requires `view` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let view = unsafe { borrow_ffi(view) };
    view.stretch_axis().into()
}

// ============================================================================
// WuiStr - UTF-8 string for FFI
// ============================================================================

/// UTF-8 string represented as a byte array.
#[repr(C)]
#[derive(Debug, Default)]
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

impl WuiStr {
    /// Returns the string as a `&str` without consuming the `WuiStr`.
    ///
    /// # Safety
    /// The caller must ensure the underlying bytes are valid UTF-8.
    #[must_use]
    pub unsafe fn as_str(&self) -> &str {
        // SAFETY: a `WuiStr` is only ever built from a Rust `Str`, which is UTF-8 by
        // construction, so its bytes are always valid UTF-8.
        unsafe { core::str::from_utf8_unchecked(self.0.as_slice()) }
    }
}

/// Moves an owned string into a Rust allocation for nullable FFI payloads.
///
/// The returned pointer must be consumed exactly once by the API receiving it.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub extern "C" fn waterui_str_box(value: WuiStr) -> *mut WuiStr {
    Box::into_raw(Box::new(value))
}

impl IntoRust for WuiStr {
    type Rust = Str;
    unsafe fn into_rust(self) -> Self::Rust {
        let bytes = self.0.as_slice().to_vec();
        self.0.consume();
        // Safety: We assume the input bytes are valid UTF-8
        unsafe { Str::from_utf8_unchecked(bytes) }
    }
}

/// Generic FFI payload for `Metadata<T>` views: the wrapped content plus the
/// attached metadata value.
#[repr(C)]
#[derive(Debug)]
pub struct WuiMetadata<T> {
    /// The view content wrapped by this metadata node.
    pub content: *mut WuiAnyView,
    /// The metadata value attached to `content`.
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
/// Layout: { content: *mut `WuiAnyView`, value: *mut `WuiEnv` }
pub type WuiMetadataEnv = WuiMetadata<*mut WuiEnv>;

// Generate waterui_metadata_env_id() and waterui_force_as_metadata_env()
ffi_metadata!(waterui::Environment, WuiMetadataEnv, env);

// ========== Navigation transition metadata FFI ==========

use crate::id::WuiId;
use waterui_navigation::{NavigationTransitionDestination, NavigationTransitionSource};

impl IntoFFI for NavigationTransitionSource {
    type FFI = WuiId;

    fn into_ffi(self) -> Self::FFI {
        self.id().into_ffi()
    }
}

impl IntoFFI for NavigationTransitionDestination {
    type FFI = WuiId;

    fn into_ffi(self) -> Self::FFI {
        self.id().into_ffi()
    }
}

/// Source geometry metadata for native navigation transitions.
pub type WuiMetadataNavigationTransitionSource = WuiMetadata<WuiId>;

/// Destination geometry metadata for native navigation transitions.
pub type WuiMetadataNavigationTransitionDestination = WuiMetadata<WuiId>;

ffi_metadata!(
    NavigationTransitionSource,
    WuiMetadataNavigationTransitionSource,
    navigation_transition_source
);
ffi_metadata!(
    NavigationTransitionDestination,
    WuiMetadataNavigationTransitionDestination,
    navigation_transition_destination
);

// ========== Metadata<Secure> FFI ==========
// Used to mark views as secure (prevent screenshots)

use waterui::metadata::secure::{HighDynamicRange, Secure, StandardDynamicRange};

/// C-compatible empty marker struct for Secure metadata.
/// This is needed because `()` (unit type) is not representable in C.
#[repr(C)]
#[derive(Debug)]
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
/// Layout: { content: *mut `WuiAnyView`, value: `WuiSecureMarker` }
pub type WuiMetadataSecure = WuiMetadata<WuiSecureMarker>;

// Generate waterui_metadata_secure_id() and waterui_force_as_metadata_secure()
ffi_metadata!(Secure, WuiMetadataSecure, secure);

// ========== Metadata<StandardDynamicRange>/Metadata<HighDynamicRange> FFI ==========
// Used to toggle HDR color handling for a subtree.

/// C-compatible empty marker struct for dynamic range metadata.
#[repr(C)]
#[derive(Debug)]
pub struct WuiDynamicRangeMarker {
    /// Placeholder field so the struct has a valid size in C; the value is
    /// meaningless.
    _marker: u8,
}

impl IntoFFI for StandardDynamicRange {
    type FFI = WuiDynamicRangeMarker;
    fn into_ffi(self) -> Self::FFI {
        WuiDynamicRangeMarker { _marker: 0 }
    }
}

impl IntoFFI for HighDynamicRange {
    type FFI = WuiDynamicRangeMarker;
    fn into_ffi(self) -> Self::FFI {
        WuiDynamicRangeMarker { _marker: 0 }
    }
}

/// Type alias for Metadata<StandardDynamicRange> FFI struct.
pub type WuiMetadataStandardDynamicRange = WuiMetadata<WuiDynamicRangeMarker>;

/// Type alias for Metadata<HighDynamicRange> FFI struct.
pub type WuiMetadataHighDynamicRange = WuiMetadata<WuiDynamicRangeMarker>;

// Generate waterui_metadata_standard_dynamic_range_id()
// and waterui_force_as_metadata_standard_dynamic_range().
ffi_metadata!(
    StandardDynamicRange,
    WuiMetadataStandardDynamicRange,
    standard_dynamic_range
);

// Generate waterui_metadata_high_dynamic_range_id()
// and waterui_force_as_metadata_high_dynamic_range().
ffi_metadata!(
    HighDynamicRange,
    WuiMetadataHighDynamicRange,
    high_dynamic_range
);

// ========== Metadata<GestureObserver> FFI ==========
// Used to attach gesture recognizers to views

use crate::gesture::WuiGestureObserver;
use waterui::gesture::GestureObserver;

/// Type alias for Metadata<GestureObserver> FFI struct
pub type WuiMetadataGesture = WuiMetadata<WuiGestureObserver>;

// Generate waterui_metadata_gesture_id() and waterui_force_as_metadata_gesture()
ffi_metadata!(GestureObserver, WuiMetadataGesture, gesture);

// ========== Metadata<OnEvent> FFI ==========
// Used to attach lifecycle event handlers (appear/disappear) - one-time handlers

use crate::event::{WuiLifecycleHook, WuiOnEvent};
use waterui_core::event::{LifeCycleHook, OnEvent};

/// Type alias for `Metadata<LifeCycleHook>` FFI struct.
pub type WuiMetadataLifecycleHook = WuiMetadata<WuiLifecycleHook>;

// Generate waterui_metadata_lifecycle_hook_id() and waterui_force_as_metadata_lifecycle_hook()
ffi_metadata!(LifeCycleHook, WuiMetadataLifecycleHook, lifecycle_hook);

// Used to attach interaction event handlers (hover enter/exit) - repeatable handlers

/// Type alias for Metadata<OnEvent> FFI struct
pub type WuiMetadataOnEvent = WuiMetadata<WuiOnEvent>;

// Generate waterui_metadata_on_event_id() and waterui_force_as_metadata_on_event()
ffi_metadata!(OnEvent, WuiMetadataOnEvent, on_event);

// ========== Metadata<Cursor> FFI ==========
// Used to set cursor style when hovering over views

use crate::cursor::WuiCursor;
use waterui::cursor::Cursor;

/// Type alias for Metadata<Cursor> FFI struct
pub type WuiMetadataCursor = WuiMetadata<WuiCursor>;

// Generate waterui_metadata_cursor_id() and waterui_force_as_metadata_cursor()
ffi_metadata!(Cursor, WuiMetadataCursor, cursor);

// ========== Common imports for metadata FFI ==========
use crate::color::WuiColor;
use crate::reactive::WuiComputed;

// ========== WuiMaterial FFI ==========
// Used for material blur effects (window backgrounds, etc.)

use waterui::background::Material;

/// FFI-safe representation of a material blur style.
///
/// Maps to `SwiftUI`'s Material types on Apple platforms.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub enum WuiMaterial {
    /// Ultra-thin blur, most transparent.
    UltraThin = 0,
    /// Thin blur.
    Thin = 1,
    /// Regular blur (default).
    Regular = 2,
    /// Thick blur.
    Thick = 3,
    /// Ultra-thick blur, most opaque.
    UltraThick = 4,
}

impl IntoFFI for Material {
    type FFI = WuiMaterial;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Self::UltraThin => WuiMaterial::UltraThin,
            Self::Thin => WuiMaterial::Thin,
            Self::Regular => WuiMaterial::Regular,
            Self::Thick => WuiMaterial::Thick,
            Self::UltraThick => WuiMaterial::UltraThick,
        }
    }
}

// ========== Metadata<Shadow> FFI ==========
// Used to apply shadow effects to views

use waterui::style::Shadow;

/// FFI-safe representation of a shadow.
#[repr(C)]
#[derive(Debug)]
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

// ========== Metadata<Border> FFI ==========
// Used to apply border effects to views

use waterui::border::Border;

/// FFI-safe representation of a border.
#[repr(C)]
#[derive(Debug)]
pub struct WuiBorder {
    /// Border color (as opaque pointer - needs environment to resolve).
    pub color: *mut WuiColor,
    /// Border width in points.
    pub width: f32,
    /// Corner radius in points (0 = square corners).
    pub corner_radius: f32,
    /// Which edges to draw the border on.
    pub edges: WuiEdgeSet,
}

impl IntoFFI for Border {
    type FFI = WuiBorder;
    fn into_ffi(self) -> Self::FFI {
        WuiBorder {
            color: self.color.into_ffi(),
            width: self.width,
            corner_radius: self.corner_radius,
            edges: self.edges.into_ffi(),
        }
    }
}

/// Type alias for Metadata<Border> FFI struct
pub type WuiMetadataBorder = WuiMetadata<WuiBorder>;

// Generate waterui_metadata_border_id() and waterui_force_as_metadata_border()
ffi_metadata!(Border, WuiMetadataBorder, border);

use waterui::style::{Anchor, Offset, Rotation, Scale};
// ========== Metadata<Scale> FFI ==========
// Used to apply scale transforms to views

/// FFI-safe representation of an anchor point.
/// Normalized coordinates: (0.0, 0.0) = top-left, (0.5, 0.5) = center, (1.0, 1.0) = bottom-right.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
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
#[derive(Debug)]
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
#[derive(Debug)]
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
#[derive(Debug)]
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

use waterui::filter::Opacity;

// ========== Metadata<Opacity> FFI ==========
// Used to apply compositor opacity metadata to views

/// FFI-safe representation of compositor opacity metadata.
/// All values are reactive (`Computed`) and can be animated.
#[repr(C)]
#[derive(Debug)]
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
#[derive(Debug)]
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
#[derive(Debug)]
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

/// FFI-safe representation of `IgnoreSafeArea`.
#[repr(C)]
#[derive(Debug)]
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
#[derive(Debug)]
pub struct WuiRetain {
    /// Opaque pointer to the retained value (Box<dyn Any>).
    /// This must be kept alive and dropped when the view is disposed.
    _opaque: *mut (),
}

#[cfg(feature = "android-jni")]
impl WuiRetain {
    pub(crate) fn opaque_ptr(&self) -> *mut () {
        self._opaque
    }

    pub(crate) fn from_ptr(ptr: *mut ()) -> Self {
        Self { _opaque: ptr }
    }
}

impl IntoFFI for Retain {
    type FFI = WuiRetain;
    fn into_ffi(self) -> Self::FFI {
        // Leak the Retain to keep the inner value alive
        // The native side will call waterui_drop_retain to clean up
        let boxed = Box::new(self);
        WuiRetain {
            _opaque: Box::into_raw(boxed).cast::<()>(),
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
#[expect(
    clippy::used_underscore_binding,
    reason = "the `_opaque` field name is part of the checked-in C ABI; the underscore signals native code must treat it as opaque"
)]
pub unsafe extern "C" fn waterui_drop_retain(retain: WuiRetain) {
    // SAFETY: the caller contract makes `_opaque` the boxed `Retain` handed out by the
    // matching constructor, released exactly once here.
    unsafe {
        drop(Box::from_raw(retain._opaque.cast::<Retain>()));
    }
}

// ========== Metadata<ClipShape> FFI ==========
// Used to clip views to shapes

use waterui::shape::{ClipShape, PathCommand};

/// FFI-safe representation of a path command.
/// All coordinates are normalized (0.0-1.0) and scale with view bounds.
#[repr(C)]
#[derive(Debug)]
pub enum WuiPathCommand {
    /// Move to a position without drawing.
    MoveTo {
        /// Target X coordinate.
        x: f32,
        /// Target Y coordinate.
        y: f32,
    },
    /// Draw a straight line to a position.
    LineTo {
        /// Target X coordinate.
        x: f32,
        /// Target Y coordinate.
        y: f32,
    },
    /// Draw a quadratic bezier curve.
    QuadTo {
        /// Control point X coordinate.
        cx: f32,
        /// Control point Y coordinate.
        cy: f32,
        /// End point X coordinate.
        x: f32,
        /// End point Y coordinate.
        y: f32,
    },
    /// Draw a cubic bezier curve.
    CubicTo {
        /// First control point X coordinate.
        c1x: f32,
        /// First control point Y coordinate.
        c1y: f32,
        /// Second control point X coordinate.
        c2x: f32,
        /// Second control point Y coordinate.
        c2y: f32,
        /// End point X coordinate.
        x: f32,
        /// End point Y coordinate.
        y: f32,
    },
    /// Draw an arc.
    Arc {
        /// Center X coordinate.
        cx: f32,
        /// Center Y coordinate.
        cy: f32,
        /// Radius along the X axis.
        rx: f32,
        /// Radius along the Y axis.
        ry: f32,
        /// Start angle in radians.
        start: f32,
        /// Sweep angle in radians.
        sweep: f32,
    },
    /// Close the current subpath.
    Close,
}

impl IntoFFI for PathCommand {
    type FFI = WuiPathCommand;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Self::MoveTo { x, y } => WuiPathCommand::MoveTo { x, y },
            Self::LineTo { x, y } => WuiPathCommand::LineTo { x, y },
            Self::QuadTo { cx, cy, x, y } => WuiPathCommand::QuadTo { cx, cy, x, y },
            Self::CubicTo {
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
            Self::Arc {
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
            Self::Close => WuiPathCommand::Close,
        }
    }
}

/// FFI-safe representation of a clip shape.
/// Contains the path commands that define the clipping mask.
#[repr(C)]
#[derive(Debug)]
pub struct WuiClipShape {
    /// Array of path commands defining the shape.
    pub commands: WuiArray<WuiPathCommand>,
}

impl IntoFFI for ClipShape {
    type FFI = WuiClipShape;
    fn into_ffi(self) -> Self::FFI {
        let commands: alloc::vec::Vec<WuiPathCommand> =
            self.commands().iter().map(|cmd| cmd.into_ffi()).collect();
        WuiClipShape {
            commands: WuiArray::new(commands),
        }
    }
}

/// Type alias for Metadata<ClipShape> FFI struct
pub type WuiMetadataClipShape = WuiMetadata<WuiClipShape>;

// Generate waterui_metadata_clip_shape_id() and waterui_force_as_metadata_clip_shape()
ffi_metadata!(ClipShape, WuiMetadataClipShape, clip_shape);

// Filled shapes are represented by `ResolvedShape` raw views via ffi/src/shape.rs.

// ========== Metadata<ContextMenu> FFI ==========
// Used to attach context menus to views

use crate::components::icon::WuiSystemIcon;
use crate::components::text::WuiText;
use crate::views::{WuiAnyViews, signal_vec_views};
use waterui::metadata::context_menu::ResolvedContextMenu;
use waterui_controls::menu::{ResolvedMenu, ResolvedMenuItem, Shortcut, ShortcutModifiers};
use waterui_core::handler::SharedAction;
use waterui_icon::SystemIcon;
use waterui_text::styled::StyledStr;

opaque!(WuiSharedAction, SharedAction<()>, shared_action);

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
    // SAFETY: the caller contract requires `action` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let action = unsafe { borrow_ffi(action) }.0.clone();
    // SAFETY: the caller contract requires `env` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let env = unsafe { borrow_ffi(env) }.0.clone();
    action.call(&env);
}

/// FFI-safe menu item tag.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiMenuItemTag {
    /// A leaf command.
    Command = 0,
    /// A divider separating adjacent items.
    Divider = 1,
    /// A nested menu.
    Menu = 2,
}

ffi_safe!(WuiMenuItemTag);

/// FFI-safe shortcut modifier flags.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WuiShortcutModifiers {
    /// Command modifier on Apple platforms.
    pub command: bool,
    /// Shift modifier.
    pub shift: bool,
    /// Option/alt modifier.
    pub option: bool,
    /// Control modifier.
    pub control: bool,
}

ffi_safe!(WuiShortcutModifiers);

impl IntoFFI for ShortcutModifiers {
    type FFI = WuiShortcutModifiers;

    fn into_ffi(self) -> Self::FFI {
        WuiShortcutModifiers {
            command: self.command(),
            shift: self.shift(),
            option: self.option(),
            control: self.control(),
        }
    }
}

/// FFI-safe keyboard shortcut payload.
#[repr(C)]
#[derive(Debug)]
pub struct WuiShortcut {
    /// The key equivalent.
    pub key: WuiStr,
    /// The shortcut modifiers.
    pub modifiers: WuiShortcutModifiers,
}

impl IntoFFI for Shortcut {
    type FFI = WuiShortcut;

    fn into_ffi(self) -> Self::FFI {
        WuiShortcut {
            key: self.key.into_ffi(),
            modifiers: self.modifiers.into_ffi(),
        }
    }
}

/// FFI-safe representation of a menu item.
#[repr(C)]
#[derive(Debug)]
pub struct WuiMenuItem {
    /// The menu node kind.
    pub tag: WuiMenuItemTag,
    /// The resolved label for commands and nested menus.
    pub label: *mut WuiText,
    /// Optional icon shown alongside the label.
    pub icon: *mut WuiSystemIcon,
    /// The action handler pointer for commands.
    pub action: *mut WuiSharedAction,
    /// Reactive disabled state for commands.
    pub disabled: *mut WuiComputed<bool>,
    /// Reactive selected/checkmark state for commands.
    pub selected: *mut WuiComputed<bool>,
    /// Optional keyboard shortcut metadata for commands.
    pub shortcut: *mut WuiShortcut,
    /// Nested menu items.
    pub items: *mut WuiAnyViews,
}

/// Takes the label value from an owned menu-item label allocation.
///
/// # Safety
///
/// `label` must be consumed exactly once.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_menu_item_take_label(label: *mut WuiText) -> WuiText {
    // SAFETY: the caller contract makes `label` an owning pointer from the matching
    // FFI constructor, so reclaiming the box frees it exactly once.
    unsafe { *Box::from_raw(label) }
}

/// Takes the icon value from an owned menu-item icon allocation.
///
/// # Safety
///
/// `icon` must be consumed exactly once.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_menu_item_take_icon(icon: *mut WuiSystemIcon) -> WuiSystemIcon {
    // SAFETY: the caller contract makes `icon` an owning pointer from the matching
    // FFI constructor, so reclaiming the box frees it exactly once.
    unsafe { *Box::from_raw(icon) }
}

/// Takes the shortcut value from an owned menu-item shortcut allocation.
///
/// # Safety
///
/// `shortcut` must be consumed exactly once.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_menu_item_take_shortcut(
    shortcut: *mut WuiShortcut,
) -> WuiShortcut {
    // SAFETY: the caller contract makes `shortcut` an owning pointer from the
    // matching FFI constructor, so reclaiming the box frees it exactly once.
    unsafe { *Box::from_raw(shortcut) }
}

#[inline]
fn menu_text(label: waterui_text::TextConfig) -> *mut WuiText {
    Box::into_raw(Box::new(label.into_ffi()))
}

#[inline]
fn optional_menu_icon(icon: Option<SystemIcon>) -> *mut WuiSystemIcon {
    icon.map_or_else(null_mut, |icon| Box::into_raw(Box::new(icon.into_ffi())))
}

#[inline]
fn optional_shortcut(shortcut: Option<Shortcut>) -> *mut WuiShortcut {
    shortcut.map_or_else(null_mut, |shortcut| {
        Box::into_raw(Box::new(shortcut.into_ffi()))
    })
}

/// Raw semantic menu item stored in the framework's identity-aware collection.
#[doc(hidden)]
#[derive(Debug)]
pub struct ResolvedMenuItemView(ResolvedMenuItem);

waterui_core::raw_view!(ResolvedMenuItemView);

impl IntoFFI for ResolvedMenuItemView {
    type FFI = WuiMenuItem;

    fn into_ffi(self) -> Self::FFI {
        match self.0 {
            ResolvedMenuItem::Command(command) => WuiMenuItem {
                tag: WuiMenuItemTag::Command,
                label: menu_text(command.label),
                icon: optional_menu_icon(command.icon),
                action: command.action.into_ffi(),
                disabled: command.disabled.into_ffi(),
                selected: command.selected.into_ffi(),
                shortcut: optional_shortcut(command.shortcut),
                items: null_mut(),
            },
            ResolvedMenuItem::Divider => WuiMenuItem {
                tag: WuiMenuItemTag::Divider,
                label: null_mut(),
                icon: null_mut(),
                action: null_mut(),
                disabled: null_mut(),
                selected: null_mut(),
                shortcut: null_mut(),
                items: null_mut(),
            },
            ResolvedMenuItem::Menu(menu) => WuiMenuItem {
                tag: WuiMenuItemTag::Menu,
                label: menu_text(menu.label),
                icon: optional_menu_icon(menu.icon),
                action: null_mut(),
                disabled: null_mut(),
                selected: null_mut(),
                shortcut: null_mut(),
                items: menu_items_views(menu.items),
            },
        }
    }
}

ffi_view!(ResolvedMenuItemView, WuiMenuItem, menu_item);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum MenuItemIdentity {
    Semantic(usize),
    Divider(usize),
}

fn menu_item_identity(items: &[ResolvedMenuItem], index: usize) -> MenuItemIdentity {
    match &items[index] {
        ResolvedMenuItem::Command(command) => MenuItemIdentity::Semantic(command.semantic_id()),
        ResolvedMenuItem::Menu(menu) => MenuItemIdentity::Semantic(menu.semantic_id()),
        ResolvedMenuItem::Divider => MenuItemIdentity::Divider(
            items[..=index]
                .iter()
                .filter(|item| matches!(item, ResolvedMenuItem::Divider))
                .count(),
        ),
    }
}

pub(crate) fn menu_items_views(
    items: nami::Computed<alloc::vec::Vec<ResolvedMenuItem>>,
) -> *mut WuiAnyViews {
    signal_vec_views(items, menu_item_identity, |item| {
        AnyView::new(ResolvedMenuItemView(item))
    })
}

/// FFI-safe representation of a context menu.
#[repr(C)]
#[derive(Debug)]
pub struct WuiContextMenu {
    /// Identity-aware reactive menu items.
    pub items: *mut WuiAnyViews,
}

impl IntoFFI for ResolvedContextMenu {
    type FFI = WuiContextMenu;
    fn into_ffi(self) -> Self::FFI {
        WuiContextMenu {
            items: menu_items_views(self.items),
        }
    }
}

/// Type alias for Metadata<ContextMenu> FFI struct
pub type WuiMetadataContextMenu = WuiMetadata<WuiContextMenu>;

// Generate waterui_metadata_context_menu_id() and waterui_force_as_metadata_context_menu()
ffi_metadata!(ResolvedContextMenu, WuiMetadataContextMenu, context_menu);

// ========== Menu FFI ==========
// Menu component that displays a dropdown menu when tapped

/// FFI-safe representation of a Menu component.
#[repr(C)]
#[derive(Debug)]
pub struct WuiMenu {
    /// The label view displayed on the menu button.
    pub label: *mut WuiAnyView,
    /// Identity-aware reactive menu items.
    pub items: *mut WuiAnyViews,
    /// Semantic accessibility label for the menu trigger.
    pub accessibility_label: *mut WuiComputed<StyledStr>,
}

impl IntoFFI for ResolvedMenu {
    type FFI = WuiMenu;
    fn into_ffi(self) -> Self::FFI {
        WuiMenu {
            label: self.label.into_ffi(),
            items: menu_items_views(self.items),
            accessibility_label: self.accessibility_label.into_ffi(),
        }
    }
}

// Generate waterui_menu_id() and waterui_force_as_menu()
ffi_view!(ResolvedMenu, WuiMenu, menu);

// ========== Drag and Drop FFI ==========

// Used to make views draggable or drop destinations

use crate::drag_drop::{WuiDraggable, WuiDropDestination};
#[cfg(feature = "c-api")]
use waterui::drag_drop::{Draggable, DropDestination};

/// Type alias for Metadata<Draggable> FFI struct
pub type WuiMetadataDraggable = WuiMetadata<WuiDraggable>;

// Generate waterui_metadata_draggable_id() and waterui_force_as_metadata_draggable()
#[cfg(feature = "c-api")]
ffi_metadata!(Draggable, WuiMetadataDraggable, draggable);

/// Type alias for Metadata<DropDestination> FFI struct
pub type WuiMetadataDropDestination = WuiMetadata<WuiDropDestination>;

// Generate waterui_metadata_drop_destination_id() and waterui_force_as_metadata_drop_destination()
#[cfg(feature = "c-api")]
ffi_metadata!(
    DropDestination,
    WuiMetadataDropDestination,
    drop_destination
);

// ========== IgnorableMetadata<MaterialBackground> FFI ==========
// Used to apply native blur effects on supported platforms (Apple)

#[cfg(feature = "c-api")]
use waterui::background::MaterialBackground;

/// FFI-safe representation of `IgnorableMetadata`<MaterialBackground>
#[repr(C)]
#[derive(Debug)]
#[cfg(feature = "c-api")]
pub struct WuiIgnorableMetadataMaterialBackground {
    /// The view content wrapped by this metadata
    pub content: *mut WuiAnyView,
    /// The material type for the blur effect
    pub material: WuiMaterial,
}

#[cfg(feature = "c-api")]
impl IntoFFI for waterui_core::IgnorableMetadata<MaterialBackground> {
    type FFI = WuiIgnorableMetadataMaterialBackground;

    fn into_ffi(self) -> Self::FFI {
        WuiIgnorableMetadataMaterialBackground {
            content: self.content.into_ffi(),
            material: self.value.0.into_ffi(),
        }
    }
}

// Generate waterui_ignorable_metadata_material_background_id() and
// waterui_force_as_ignorable_metadata_material_background()
#[cfg(feature = "c-api")]
ffi_ignorable_metadata!(
    MaterialBackground,
    WuiIgnorableMetadataMaterialBackground,
    material_background
);

// ========== Metadata<Hittable> FFI ==========
// Controls whether a view responds to hit testing (touch/click events)

use waterui::interaction::Hittable;

/// FFI-safe representation of Hittable metadata.
#[repr(C)]
#[derive(Debug)]
pub struct WuiHittable {
    /// Whether hit testing is enabled (reactive).
    pub enabled: *mut WuiComputed<bool>,
}

impl IntoFFI for Hittable {
    type FFI = WuiHittable;
    fn into_ffi(self) -> Self::FFI {
        WuiHittable {
            enabled: self.enabled.into_ffi(),
        }
    }
}

/// Type alias for Metadata<Hittable> FFI struct
pub type WuiMetadataHittable = WuiMetadata<WuiHittable>;

// Generate waterui_metadata_hittable_id() and waterui_force_as_metadata_hittable()
ffi_metadata!(Hittable, WuiMetadataHittable, hittable);

#[cfg(all(test, feature = "c-api"))]
mod tests {
    use alloc::rc::Rc;
    use alloc::sync::Arc;
    use core::cell::Cell;
    use core::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use waterui_core::handler::SharedAction;

    #[test]
    fn shared_action_callback_executes() {
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_for_action = Arc::clone(&hits);
        let action_ptr = SharedAction::new(move || {
            hits_for_action.fetch_add(1, Ordering::SeqCst);
        })
        .into_ffi();
        let env_ptr = waterui::Environment::new().into_ffi();

        // SAFETY: both pointers are the live handles this test created above.
        unsafe {
            waterui_call_shared_action(action_ptr, env_ptr);
        }

        assert_eq!(hits.load(Ordering::SeqCst), 1);
        // SAFETY: both handles are still live and are each released once here.
        unsafe {
            waterui_drop_shared_action(action_ptr);
            let _: waterui::Environment = IntoRust::into_rust(env_ptr);
        }
    }

    #[test]
    fn shared_action_survives_reentrant_native_drop_until_callback_returns() {
        let action_ptr = Rc::new(Cell::new(core::ptr::null_mut::<WuiSharedAction>()));
        let callback_finished = Rc::new(Cell::new(false));
        let action_ptr_for_callback = Rc::clone(&action_ptr);
        let callback_finished_for_callback = Rc::clone(&callback_finished);
        let action = SharedAction::new(move || {
            // SAFETY: the cell holds the action handle the test installed before
            // invoking this callback, and the callback runs once.
            unsafe { waterui_drop_shared_action(action_ptr_for_callback.get()) };
            callback_finished_for_callback.set(true);
        })
        .into_ffi();
        action_ptr.set(action);
        let env = WuiEnv(waterui::Environment::new());

        // SAFETY: `action` is the handle built just above and `env` a live local.
        unsafe { waterui_call_shared_action(action, &raw const env) };

        assert!(callback_finished.get());
    }
}
