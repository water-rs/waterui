use core::ops::Deref;

#[cfg(feature = "c-api")]
use crate::WuiAnyView;
use crate::array::WuiArray;
use crate::components::text::WuiStyledStr;
use crate::{IntoFFI, IntoRust, OpaqueType, WuiStr};
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
#[cfg(feature = "c-api")]
use nami::watcher::WatcherGuard;
use nami::watcher::{Context, Watcher};
use nami::{Computed, Signal, watcher};
#[cfg(feature = "c-api")]
use waterui::AnyView;
use waterui::Str;
use waterui::reactive::watcher::BoxWatcherGuard;
use waterui::reactive::watcher::Metadata;
use waterui_text::styled::StyledStr;
opaque!(WuiWatcherMetadata, Metadata, watcher_metadata, any());

opaque!(WuiWatcherGuard, BoxWatcherGuard, box_watcher_guard, any());

#[repr(transparent)]
pub struct WuiComputed<T>(pub(crate) waterui::Computed<T>);

impl<T> WuiComputed<T>
where
    T: IntoFFI,
    T::FFI: IntoRust<Rust = T>,
{
    /// Creates a new FFI-computed signal using native callbacks.
    ///
    /// # Safety
    /// The caller must ensure that the provided function pointers are valid and adhere to the expected signatures
    pub unsafe fn new(
        data: *mut (),
        get: unsafe extern "C" fn(*const ()) -> T::FFI,
        watch: unsafe extern "C" fn(*const (), *mut WuiWatcher<T>) -> *mut WuiWatcherGuard,
        drop: unsafe extern "C" fn(*mut ()),
    ) -> WuiComputed<T>
    where
        T: IntoFFI + 'static,
    {
        unsafe { WuiComputed(Computed::new(FFIComputed::new(data, get, watch, drop))) }
    }
}

struct FFIComputed<T: IntoFFI> {
    data: Rc<NativeComputedData>,
    get: unsafe extern "C" fn(*const ()) -> T::FFI,
    watch: unsafe extern "C" fn(*const (), *mut WuiWatcher<T>) -> *mut WuiWatcherGuard,
}

struct NativeComputedData {
    ptr: *mut (),
    drop: unsafe extern "C" fn(*mut ()),
}

impl Drop for NativeComputedData {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.ptr) };
    }
}

impl<T: IntoFFI> Signal for FFIComputed<T>
where
    T::FFI: IntoRust<Rust = T>,
{
    type Output = T;
    type Guard = BoxWatcherGuard;
    fn get(&self) -> Self::Output {
        unsafe { (self.get)(self.data.ptr.cast_const()).into_rust() }
    }

    fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {
        let watcher: Watcher<Self::Output> = Rc::new(watcher);
        let watcher = watcher.into_ffi();

        unsafe {
            let guard_ptr = (self.watch)(self.data.ptr.cast_const(), watcher);
            let guard = Box::from_raw(guard_ptr);
            let WuiWatcherGuard(guard) = *guard;
            guard
        }
    }
}

impl<T: IntoFFI> FFIComputed<T> {
    pub unsafe fn new(
        data: *mut (),
        get: unsafe extern "C" fn(*const ()) -> T::FFI,
        watch: unsafe extern "C" fn(*const (), *mut WuiWatcher<T>) -> *mut WuiWatcherGuard,
        drop: unsafe extern "C" fn(*mut ()),
    ) -> Self {
        Self {
            data: Rc::new(NativeComputedData { ptr: data, drop }),
            get,
            watch,
        }
    }
}

impl<T: IntoFFI> Clone for FFIComputed<T> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            get: self.get,
            watch: self.watch,
        }
    }
}

impl<T: 'static> IntoFFI for waterui::Computed<T> {
    type FFI = *mut WuiComputed<T>;

    fn into_ffi(self) -> Self::FFI {
        Box::into_raw(Box::new(WuiComputed(self)))
    }
}

impl<T: 'static> IntoFFI for Option<waterui::Computed<T>> {
    type FFI = *mut WuiComputed<T>;

    fn into_ffi(self) -> Self::FFI {
        match self {
            Some(value) => value.into_ffi(),
            None => core::ptr::null_mut(),
        }
    }
}

impl<T> IntoFFI for waterui::Binding<T> {
    type FFI = *mut WuiBinding<T>;

    fn into_ffi(self) -> Self::FFI {
        Box::into_raw(Box::new(WuiBinding(self)))
    }
}

impl<T: 'static> OpaqueType for WuiComputed<T> {}

impl<T> Deref for WuiComputed<T> {
    type Target = waterui::Computed<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> Deref for WuiBinding<T> {
    type Target = waterui::Binding<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[repr(transparent)]
pub struct WuiBinding<T: 'static>(pub(crate) waterui::Binding<T>);

impl<T> OpaqueType for WuiBinding<T> {}

/// Generates the C constructor for a native watcher.
///
/// Invoke this directly for binding-only types. [`ffi_computed!`] invokes it
/// automatically for computed types.
#[macro_export]
macro_rules! ffi_watcher {
    ($ty:ty, $ffi:ty, $ident:tt) => {
        pastey::paste! {
            #[cfg(feature = "c-api")]
            /// Creates a watcher from native callbacks.
            ///
            /// # Safety
            ///
            /// All function pointers must be valid and `data` must remain valid
            /// until `drop` is called exactly once.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [<waterui_new_watcher_ $ident>](
                data: *mut (),
                call: unsafe extern "C" fn(
                    *mut (),
                    $ffi,
                    *mut $crate::reactive::WuiWatcherMetadata,
                ),
                drop: unsafe extern "C" fn(*mut ()),
            ) -> *mut $crate::reactive::WuiWatcher<$ty>
            where
                $ty: $crate::IntoFFI + 'static,
            {
                let watcher = unsafe {
                    $crate::reactive::WuiWatcher::<$ty>::new(data, call, drop)
                };
                alloc::boxed::Box::into_raw(alloc::boxed::Box::new(watcher))
            }
        }
    };

    ($ty:ty, $ffi:ty) => {
        pastey::paste! {
            $crate::ffi_watcher!($ty, $ffi, [<$ty:snake>]);
        }
    };
}

/// Generates computed FFI support for read-only reactive types.
///
/// When `c-api` feature is enabled, generates C FFI functions.
/// When `android-jni` feature is enabled, generates JNI functions.
///
/// # Generated Functions (C-API)
/// - `waterui_read_computed_{ident}` - read current value
/// - `waterui_watch_computed_{ident}` - subscribe to changes
/// - `waterui_drop_computed_{ident}` - cleanup
/// - `waterui_new_watcher_{ident}` - create watcher
///
/// For types that also need native-controlled signal constructors,
/// additionally invoke `ffi_computed_ctor!`.
#[macro_export]
macro_rules! ffi_computed {
    ($ty:ty,$ffi:ty, $ident:tt) => {
        pastey::paste!{
            // ========== C-API (for Apple/GTK backends) ==========
            #[cfg(feature = "c-api")]
            /// Reads the current value from a computed
            /// # Safety
            /// The computed pointer must be valid and point to a properly initialized computed object.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_read_computed_ $ident >](computed: *const $crate::reactive::WuiComputed<$ty>) -> $ffi {
                use waterui::Signal;
                unsafe { $crate::IntoFFI::into_ffi((&(*computed)).get()) }
            }

            #[cfg(feature = "c-api")]
            /// Watches for changes in a computed
            /// # Safety
            /// The computed pointer must be valid and point to a properly initialized computed object.
            /// The watcher pointer will be consumed and freed when the returned guard is dropped.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_watch_computed_ $ident >](
                computed: *const $crate::reactive::WuiComputed<$ty>,
                watcher: *mut $crate::reactive::WuiWatcher<$ty>,
            ) -> *mut $crate::reactive::WuiWatcherGuard {
                use waterui::Signal;
                unsafe {
                    let watcher = (*alloc::boxed::Box::from_raw(watcher)).into_inner();
                    let guard = (&*computed).watch(move |ctx| {
                        let metadata: waterui::reactive::watcher::Metadata = ctx.metadata().clone();
                        let value = ctx.into_value();
                        let callback = alloc::rc::Rc::clone(&watcher);
                        callback(nami::watcher::Context::new(value, metadata));
                    });
                    $crate::IntoFFI::into_ffi(guard)
                }
            }

            #[cfg(feature = "c-api")]
            /// Drops a computed
            /// # Safety
            /// The caller must ensure that `computed` is a valid pointer.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_drop_computed_ $ident >](computed: *mut $crate::reactive::WuiComputed<$ty>) {
                unsafe { drop(alloc::boxed::Box::from_raw(computed)); }
            }

            // ========== Android JNI ==========
            #[cfg(feature = "android-jni")]
            /// JNI: Drops a computed
            #[unsafe(no_mangle)]
            pub extern "system" fn [<Java_dev_waterui_android_ffi_WatcherJni_dropComputed $ident:camel>]<'local>(
                _env: $crate::jni::JNIEnv<'local>,
                _class: $crate::jni::JClass<'local>,
                computed_ptr: $crate::jni::jlong,
            ) {
                unsafe { drop(alloc::boxed::Box::from_raw(computed_ptr as *mut $crate::reactive::WuiComputed<$ty>)) };
            }

            #[cfg(feature = "android-jni")]
            /// JNI: Watches for changes in a computed
            #[unsafe(no_mangle)]
            pub extern "system" fn [<Java_dev_waterui_android_ffi_WatcherJni_watchComputed $ident:camel>]<'local>(
                mut env: $crate::jni::JNIEnv<'local>,
                _class: $crate::jni::JClass<'local>,
                computed_ptr: $crate::jni::jlong,
                watcher: $crate::jni::JObject<'local>,
            ) -> $crate::jni::jlong {
                use waterui::Signal;
                use alloc::boxed::Box;
                use alloc::rc::Rc;
                use $crate::IntoFFI;

                $crate::jni::with_env(&mut env, |env| {
                let (data_ptr, call_ptr, drop_ptr) = $crate::jni::extract_watcher_struct(env, &watcher);

                // Cast function pointers
                let call_fn: unsafe extern "C" fn(*mut (), $ffi, *mut $crate::reactive::WuiWatcherMetadata) =
                    unsafe { core::mem::transmute(call_ptr as *const ()) };
                let drop_fn: unsafe extern "C" fn(*mut ()) =
                    unsafe { core::mem::transmute(drop_ptr as *const ()) };

                // Get the computed reference
                let computed = unsafe { &*(computed_ptr as *const $crate::reactive::WuiComputed<$ty>) };

                // Create a cleaner to ensure drop_fn is called when the watcher is dropped
                struct Cleaner {
                    data: *mut (),
                    drop_fn: unsafe extern "C" fn(*mut ()),
                }
                impl Drop for Cleaner {
                    fn drop(&mut self) {
                        unsafe { (self.drop_fn)(self.data) }
                    }
                }
                let cleaner = Rc::new(Cleaner {
                    data: data_ptr as *mut (),
                    drop_fn,
                });
                let cleaner_clone = cleaner.clone();

                // Register the watcher with the computed
                let guard = computed.watch(move |ctx: nami::watcher::Context<$ty>| {
                    let cleaner = Rc::clone(&cleaner_clone);
                    let metadata: waterui::reactive::watcher::Metadata = ctx.metadata().clone();
                    let value: $ty = ctx.into_value();
                    unsafe {
                        call_fn(cleaner.data, value.into_ffi(), metadata.into_ffi());
                    }
                });

                // Return the guard as a pointer
                let guard_box = Box::new($crate::reactive::WuiWatcherGuard(Box::new(guard)));
                Box::into_raw(guard_box) as $crate::jni::jlong
                })
            }

            $crate::ffi_watcher!($ty, $ffi, $ident);

            // ========== Android JNI ==========
            // Note: JNI reactive bindings require more complex struct conversions
            // and are implemented in ffi/src/jni/reactive.rs with helper functions.
            // The macros here generate stub functions that delegate to those helpers.
        }
    };

    ($ty:ty,$ffi:ty) => {
        pastey::paste! {
            $crate::ffi_computed!($ty, $ffi, [<$ty:snake>]);
        }
    }
}

/// Generates the native-controlled computed constructor.
///
/// Generates `waterui_new_computed_{ident}` for creating signals from native callbacks.
/// Requires the FFI type to implement `IntoRust`.
///
/// Only generated for `c-api` feature - JNI uses a different approach.
#[macro_export]
macro_rules! ffi_computed_ctor {
    ($ty:ty,$ffi:ty, $ident:tt) => {
        pastey::paste!{
            #[cfg(feature = "c-api")]
            /// Creates a computed signal from native callbacks.
            /// # Safety
            /// All function pointers must be valid and follow the expected calling conventions.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_new_computed_ $ident >](
                data: *mut (),
                get: unsafe extern "C" fn(*const ()) -> $ffi,
                watch: unsafe extern "C" fn(*const (), *mut $crate::reactive::WuiWatcher<$ty>) -> *mut $crate::reactive::WuiWatcherGuard,
                drop: unsafe extern "C" fn(*mut ()),
            ) -> *mut $crate::reactive::WuiComputed<$ty>
            where
                $ty: $crate::IntoFFI + 'static,
                <$ty as $crate::IntoFFI>::FFI: $crate::IntoRust<Rust = $ty>,
            {
                let computed = unsafe { $crate::reactive::WuiComputed::new(data, get, watch, drop) };
                alloc::boxed::Box::into_raw(alloc::boxed::Box::new(computed))
            }
        }
    };

    ($ty:ty,$ffi:ty) => {
        pastey::paste! {
            $crate::ffi_computed_ctor!($ty, $ffi, [<$ty:snake>]);
        }
    }
}

/// Generates binding FFI support for mutable reactive types.
///
/// When `c-api` feature is enabled, generates C FFI functions.
/// When `android-jni` feature is enabled, generates JNI functions.
///
/// # Generated Functions (C-API)
/// - `waterui_read_binding_{ident}` - read current value
/// - `waterui_set_binding_{ident}` - set value
/// - `waterui_watch_binding_{ident}` - subscribe to changes
/// - `waterui_drop_binding_{ident}` - cleanup
#[macro_export]
macro_rules! ffi_binding {
    ($ty:ty,$ffi:ty, $ident:tt) => {
        pastey::paste!{
            // ========== C-API (for Apple/GTK backends) ==========
            #[cfg(feature = "c-api")]
            /// Reads the current value from a binding
            /// # Safety
            /// The binding pointer must be valid and point to a properly initialized binding object.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_read_binding_ $ident >](binding: *const $crate::reactive::WuiBinding<$ty>) -> $ffi {
                unsafe { (*binding).get().into_ffi() }
            }

            #[cfg(feature = "c-api")]
            /// Sets the value of a binding
            /// # Safety
            /// The binding pointer must be valid and point to a properly initialized binding object.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_set_binding_ $ident >](binding: *mut $crate::reactive::WuiBinding<$ty>, value: $ffi) {
                unsafe {
                    (*binding).set($crate::IntoRust::into_rust(value));
                }
            }

            #[cfg(feature = "c-api")]
            /// Watches for changes in a binding
            /// # Safety
            /// The binding pointer must be valid and point to a properly initialized binding object.
            /// The watcher pointer will be consumed and freed when the returned guard is dropped.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_watch_binding_ $ident >](
                binding: *const $crate::reactive::WuiBinding<$ty>,
                watcher: *mut $crate::reactive::WuiWatcher<$ty>,
            ) -> *mut $crate::reactive::WuiWatcherGuard {
                use waterui::Signal;

                unsafe {
                    let watcher = (*alloc::boxed::Box::from_raw(watcher)).into_inner();
                    let guard = (*binding).watch(move |ctx| {
                        let metadata: waterui::reactive::watcher::Metadata = ctx.metadata().clone();
                        let value = ctx.into_value();
                        let callback = alloc::rc::Rc::clone(&watcher);
                        callback(nami::watcher::Context::new(value, metadata));
                    });
                    guard.into_ffi()
                }
            }

            #[cfg(feature = "c-api")]
            /// Drops a binding
            /// # Safety
            /// The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_drop_binding_ $ident >](binding: *mut $crate::reactive::WuiBinding<$ty>) {
                unsafe {
                    drop(alloc::boxed::Box::from_raw(binding));
                }
            }

            // ========== Android JNI ==========
            #[cfg(feature = "android-jni")]
            /// JNI: Drops a binding
            #[unsafe(no_mangle)]
            pub extern "system" fn [<Java_dev_waterui_android_ffi_WatcherJni_dropBinding $ident:camel>]<'local>(
                _env: $crate::jni::JNIEnv<'local>,
                _class: $crate::jni::JClass<'local>,
                binding_ptr: $crate::jni::jlong,
            ) {
                unsafe { drop(alloc::boxed::Box::from_raw(binding_ptr as *mut $crate::reactive::WuiBinding<$ty>)) };
            }

            #[cfg(feature = "android-jni")]
            /// JNI: Watches for changes in a binding
            #[unsafe(no_mangle)]
            pub extern "system" fn [<Java_dev_waterui_android_ffi_WatcherJni_watchBinding $ident:camel>]<'local>(
                mut env: $crate::jni::JNIEnv<'local>,
                _class: $crate::jni::JClass<'local>,
                binding_ptr: $crate::jni::jlong,
                watcher: $crate::jni::JObject<'local>,
            ) -> $crate::jni::jlong {
                use waterui::Signal;
                use alloc::boxed::Box;
                use alloc::rc::Rc;
                use $crate::IntoFFI;

                $crate::jni::with_env(&mut env, |env| {
                let (data_ptr, call_ptr, drop_ptr) = $crate::jni::extract_watcher_struct(env, &watcher);

                // Cast function pointers
                let call_fn: unsafe extern "C" fn(*mut (), $ffi, *mut $crate::reactive::WuiWatcherMetadata) =
                    unsafe { core::mem::transmute(call_ptr as *const ()) };
                let drop_fn: unsafe extern "C" fn(*mut ()) =
                    unsafe { core::mem::transmute(drop_ptr as *const ()) };

                // Get the binding reference
                let binding = unsafe { &*(binding_ptr as *const $crate::reactive::WuiBinding<$ty>) };

                // Create a cleaner to ensure drop_fn is called when the watcher is dropped
                struct Cleaner {
                    data: *mut (),
                    drop_fn: unsafe extern "C" fn(*mut ()),
                }
                impl Drop for Cleaner {
                    fn drop(&mut self) {
                        unsafe { (self.drop_fn)(self.data) }
                    }
                }
                let cleaner = Rc::new(Cleaner {
                    data: data_ptr as *mut (),
                    drop_fn,
                });
                let cleaner_clone = cleaner.clone();

                // Register the watcher with the binding
                let guard = binding.watch(move |ctx: nami::watcher::Context<$ty>| {
                    let cleaner = Rc::clone(&cleaner_clone);
                    let metadata: waterui::reactive::watcher::Metadata = ctx.metadata().clone();
                    let value: $ty = ctx.into_value();
                    unsafe {
                        call_fn(cleaner.data, value.into_ffi(), metadata.into_ffi());
                    }
                });

                // Return the guard as a pointer
                let guard_box = Box::new($crate::reactive::WuiWatcherGuard(Box::new(guard)));
                Box::into_raw(guard_box) as $crate::jni::jlong
                })
            }
        }
    };

    ($ty:ty,$ffi:ty) =>{
        pastey::paste!{
            $crate::ffi_binding!($ty,$ffi,[<$ty:snake>]);
        }
    }
}

/// Generates both binding and computed FFI support.
///
/// Use this for types that need two-way reactive binding support.
#[macro_export]
macro_rules! ffi_reactive {
    ($ty:ty,$ffi:ty, $ident:tt) => {
        $crate::ffi_binding!($ty, $ffi, $ident);
        $crate::ffi_computed!($ty, $ffi, $ident);
    };

    ($ty:ty,$ffi:ty) => {
        pastey::paste! {
            $crate::ffi_reactive!($ty, $ffi, [<$ty:snake>]);
        }
    };
}

ffi_binding!(Str, WuiStr);
#[cfg(feature = "c-api")]
ffi_computed!(Str, WuiStr);
ffi_binding!(StyledStr, WuiStyledStr, styled_str);

#[cfg(feature = "c-api")]
/// Sets a `Binding<StyledStr>` using borrowed UTF-8 bytes.
///
/// Native text controls can pass their current editor string without first
/// constructing an owned FFI `WuiStr`.
///
/// # Safety
/// `binding` must be a valid pointer to `WuiBinding<StyledStr>`. `bytes` must
/// point to `len` valid UTF-8 bytes unless `len` is zero.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_set_binding_styled_str_utf8(
    binding: *mut WuiBinding<StyledStr>,
    bytes: *const u8,
    len: usize,
) {
    assert!(
        len == 0 || !bytes.is_null(),
        "waterui_set_binding_styled_str_utf8 received null bytes with non-zero length"
    );
    let bytes = if len == 0 {
        &[]
    } else {
        unsafe { core::slice::from_raw_parts(bytes, len) }
    };
    let plain = unsafe { Str::from_utf8_unchecked(bytes.to_vec()) };
    unsafe { (*binding).set(StyledStr::plain(plain)) };
}

#[cfg(feature = "c-api")]
ffi_watcher!(AnyView, *mut WuiAnyView, any_view);

#[cfg(feature = "android-jni")]
ffi_binding!(i32, i32, int);
ffi_computed!(i32, i32, i32);

ffi_reactive!(bool, bool);

ffi_computed!(f32, f32, f32);

#[cfg(feature = "android-jni")]
ffi_binding!(f64, f64, double);
ffi_computed!(f64, f64, f64);

// The C ABI uses Rust primitive names consistently.
#[cfg(feature = "c-api")]
ffi_binding!(i32, i32, i32);
#[cfg(feature = "c-api")]
ffi_binding!(f32, f32, f32);
#[cfg(feature = "c-api")]
ffi_binding!(f64, f64, f64);

// ============================================================================
// JNI Primitive Reactive Macros
// ============================================================================
//
// The `JniPrimitive` trait is defined in `jni/convert.rs` and re-exported via `jni` module.
// These macros generate read/set functions for types implementing that trait.

/// Generates JNI read/set functions for primitive binding types.
///
/// Uses the `JniPrimitive` trait (from jni::convert) for type-safe conversion.
#[macro_export]
macro_rules! jni_binding_primitive {
    ($rust_ty:ty, $binding_name:tt) => {
        pastey::paste! {
            #[cfg(feature = "android-jni")]
            #[unsafe(no_mangle)]
            pub extern "system" fn [<Java_dev_waterui_android_ffi_WatcherJni_readBinding $binding_name:camel>]<'local>(
                _env: $crate::jni::JNIEnv<'local>,
                _class: $crate::jni::JClass<'local>,
                binding_ptr: $crate::jni::jlong,
            ) -> <$rust_ty as $crate::jni::JniPrimitive>::Jni {
                use $crate::jni::JniPrimitive;
                let binding = unsafe { &*(binding_ptr as *const $crate::reactive::WuiBinding<$rust_ty>) };
                binding.get().to_jni()
            }

            #[cfg(feature = "android-jni")]
            #[unsafe(no_mangle)]
            pub extern "system" fn [<Java_dev_waterui_android_ffi_WatcherJni_setBinding $binding_name:camel>]<'local>(
                _env: $crate::jni::JNIEnv<'local>,
                _class: $crate::jni::JClass<'local>,
                binding_ptr: $crate::jni::jlong,
                value: <$rust_ty as $crate::jni::JniPrimitive>::Jni,
            ) {
                use $crate::jni::JniPrimitive;
                let binding = unsafe { &*(binding_ptr as *const $crate::reactive::WuiBinding<$rust_ty>) };
                binding.set(<$rust_ty>::from_jni(value));
            }
        }
    };
}

/// Generates JNI read functions for primitive computed types.
#[macro_export]
macro_rules! jni_computed_primitive {
    ($rust_ty:ty, $computed_name:tt) => {
        pastey::paste! {
            #[cfg(feature = "android-jni")]
            #[unsafe(no_mangle)]
            pub extern "system" fn [<Java_dev_waterui_android_ffi_WatcherJni_readComputed $computed_name:camel>]<'local>(
                _env: $crate::jni::JNIEnv<'local>,
                _class: $crate::jni::JClass<'local>,
                computed_ptr: $crate::jni::jlong,
            ) -> <$rust_ty as $crate::jni::JniPrimitive>::Jni {
                use waterui::Signal;
                use $crate::jni::JniPrimitive;
                let computed = unsafe { &*(computed_ptr as *const $crate::reactive::WuiComputed<$rust_ty>) };
                computed.get().to_jni()
            }
        }
    };
}

// Generate JNI read/set for primitive bindings
// Note: Kotlin naming convention differs - Binding uses Java names (Int, Double, Float)
jni_binding_primitive!(bool, bool);
jni_binding_primitive!(i32, int);
jni_binding_primitive!(f64, double);

// Generate JNI read for primitive computed
// Note: Kotlin naming convention differs - Computed uses Rust names (I32, F64, F32)
jni_computed_primitive!(bool, bool);
jni_computed_primitive!(i32, i32);
jni_computed_primitive!(f32, f32);
jni_computed_primitive!(f64, f64);

// Date reactive bindings (using WuiDate FFI representation)
use crate::components::form::{WuiDate, WuiDateTime};
use jiff::civil::{Date, DateTime};
ffi_binding!(DateTime, WuiDateTime, date_time);
#[cfg(feature = "c-api")]
ffi_watcher!(DateTime, WuiDateTime, date_time);
ffi_reactive!(Vec<Date>, WuiArray<WuiDate>, date_vec);

pub struct WuiWatcher<T: IntoFFI>(watcher::Watcher<T>);

impl<T: IntoFFI> WuiWatcher<T> {
    /// Creates a new FFI watcher using C-style function pointers.
    ///
    /// # Safety
    /// The caller must ensure that the provided function pointers are valid and adhere to the expected signatures
    pub unsafe fn new(
        data: *mut (),
        call: unsafe extern "C" fn(*mut (), T::FFI, *mut WuiWatcherMetadata),
        drop: unsafe extern "C" fn(*mut ()),
    ) -> Self {
        struct Cleaner {
            data: *mut (),
            drop: unsafe extern "C" fn(*mut ()),
        }

        impl Drop for Cleaner {
            fn drop(&mut self) {
                unsafe { (self.drop)(self.data) }
            }
        }
        let cleaner = Rc::new(Cleaner { data, drop });
        WuiWatcher(Rc::new(move |ctx| {
            let cleaner = Rc::clone(&cleaner);
            let metadata: waterui::reactive::watcher::Metadata = ctx.metadata().clone();
            let value = ctx.into_value();
            unsafe {
                call(cleaner.data, value.into_ffi(), metadata.into_ffi());
            }
        }))
    }

    pub(crate) fn into_inner(self) -> Watcher<T> {
        self.0
    }

    pub(crate) fn call(&self, value: T, metadata: Metadata) {
        let watcher = Rc::clone(&self.0);
        watcher(Context::new(value, metadata));
    }
}

impl<T: IntoFFI> IntoFFI for Watcher<T> {
    type FFI = *mut WuiWatcher<T>;
    fn into_ffi(self) -> Self::FFI {
        Box::into_raw(Box::new(WuiWatcher(self)))
    }
}

/// Creates a new watcher guard from raw data and a drop function.
///
/// # Safety
/// The caller must ensure that the provided data pointer and drop function are valid.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub extern "C" fn waterui_new_watcher_guard(
    data: *mut (),
    drop: unsafe extern "C" fn(*mut ()),
) -> *mut WuiWatcherGuard {
    struct Cleaner {
        data: *mut (),
        drop: unsafe extern "C" fn(*mut ()),
    }

    impl Drop for Cleaner {
        fn drop(&mut self) {
            unsafe { (self.drop)(self.data) }
        }
    }

    let cleaner = Cleaner { data, drop };
    impl WatcherGuard for Cleaner {}
    Box::into_raw(Box::new(WuiWatcherGuard(Box::new(cleaner))))
}

// Custom Secure binding implementation
// Secure uses WuiStr for FFI, but converts to/from Secure on the Rust side
#[cfg(feature = "c-api")]
use waterui_form::secure::Secure;

/// Reads the current value from a Secure binding
/// # Safety
/// The binding pointer must be valid and point to a properly initialized binding object.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_read_binding_secure(binding: *const WuiBinding<Secure>) -> WuiStr {
    use alloc::string::String;
    unsafe {
        let secure = (*binding).get();
        // Create an owned String, then convert to Str
        let owned_string = String::from(secure.expose());
        Str::from(owned_string).into_ffi()
    }
}

/// Sets the value of a Secure binding
/// # Safety
/// The binding pointer must be valid and point to a properly initialized binding object.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_set_binding_secure(
    binding: *mut WuiBinding<Secure>,
    value: WuiStr,
) {
    unsafe {
        let str_value: Str = value.into_rust();
        (*binding).set(Secure::new(str_value.into_string()));
    }
}

/// Watches for changes in a Secure binding
/// # Safety
/// The binding pointer must be valid and point to a properly initialized binding object.
/// The watcher pointer will be consumed and freed when the returned guard is dropped.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_watch_binding_secure(
    binding: *const WuiBinding<Secure>,
    watcher: *mut WuiWatcher<Secure>,
) -> *mut WuiWatcherGuard {
    use waterui::Signal;

    unsafe {
        let watcher = (*Box::from_raw(watcher)).into_inner();
        let guard = (*binding).watch(move |ctx| {
            let metadata: waterui::reactive::watcher::Metadata = ctx.metadata().clone();
            let value = ctx.into_value();
            let callback = Rc::clone(&watcher);
            callback(Context::new(value, metadata));
        });
        guard.into_ffi()
    }
}

/// Drops a Secure binding
/// # Safety
/// The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_binding_secure(binding: *mut WuiBinding<Secure>) {
    unsafe {
        drop(alloc::boxed::Box::from_raw(binding));
    }
}

/// Creates a watcher from native callbacks for Secure
/// # Safety
/// All function pointers must be valid.
#[cfg(feature = "c-api")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_new_watcher_secure(
    data: *mut (),
    call: unsafe extern "C" fn(*mut (), WuiStr, *mut WuiWatcherMetadata),
    drop: unsafe extern "C" fn(*mut ()),
) -> *mut WuiWatcher<Secure> {
    use alloc::boxed::Box;
    let watcher = unsafe { WuiWatcher::new(data, call, drop) };
    Box::into_raw(Box::new(watcher))
}

#[cfg(test)]
mod tests {
    use super::{WuiWatcher, WuiWatcherMetadata};
    use alloc::boxed::Box;
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::RefCell;
    use nami::watcher::{Context, Metadata};

    struct NativeWatcherData {
        events: Rc<RefCell<Vec<&'static str>>>,
    }

    unsafe extern "C" fn record_call(
        data: *mut (),
        _value: u32,
        metadata: *mut WuiWatcherMetadata,
    ) {
        let data = unsafe { &*data.cast::<NativeWatcherData>() };
        data.events.borrow_mut().push("call");
        unsafe { super::waterui_drop_watcher_metadata(metadata) };
    }

    unsafe extern "C" fn record_drop(data: *mut ()) {
        let data = unsafe { Box::from_raw(data.cast::<NativeWatcherData>()) };
        data.events.borrow_mut().push("drop");
    }

    #[test]
    fn native_watcher_data_lives_until_the_last_callback_owner_drops() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let data = Box::into_raw(Box::new(NativeWatcherData {
            events: Rc::clone(&events),
        }))
        .cast();

        let watcher = unsafe { WuiWatcher::<u32>::new(data, record_call, record_drop) };
        let callback = watcher.into_inner();

        callback(Context::new(42, Metadata::new()));
        assert_eq!(&*events.borrow(), &["call"]);

        drop(callback);
        assert_eq!(&*events.borrow(), &["call", "drop"]);
    }
}
