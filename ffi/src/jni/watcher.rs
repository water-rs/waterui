//! Java Watcher callback wrapper for the reactive system.
//!
//! This module provides the bridge between Rust's reactive `Watcher` system
//! and Java callbacks. When a reactive value changes, the Rust side calls
//! into Java through the callback stored in `JavaWatcher`.

extern crate alloc;
extern crate std;

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::marker::PhantomData;

use jni::objects::{GlobalRef, JMethodID, JValue};
use jni::signature::{Primitive, ReturnType};
use jni::sys::jlong;

use super::with_jni_env;
use crate::jni::convert::ToJava;

/// A watcher that calls back into Java when reactive values change.
///
/// This struct holds a global reference to a Java callback object and
/// invokes its `onUpdate` method when the watched value changes.
pub struct JavaWatcher<T> {
    /// Global reference to the Java callback object.
    /// Must implement `WatcherCallback<T>` interface.
    callback: GlobalRef,
    /// Cached method ID for the callback method.
    method_id: jni::sys::jmethodID,
    /// Phantom data for the Rust type.
    _phantom: PhantomData<fn(T)>,
}

// Safety: GlobalRef is thread-safe per JNI spec
unsafe impl<T> Send for JavaWatcher<T> {}
unsafe impl<T> Sync for JavaWatcher<T> {}

impl<T> JavaWatcher<T> {
    /// Create a new JavaWatcher from a Java callback object.
    ///
    /// # Arguments
    /// * `callback` - Global reference to the Java callback object
    /// * `method_id` - Method ID for the `onUpdate` method
    ///
    /// # Safety
    /// The callback must implement the expected interface with the correct
    /// `onUpdate` signature for type T.
    pub unsafe fn new(callback: GlobalRef, method_id: jni::sys::jmethodID) -> Self {
        Self {
            callback,
            method_id,
            _phantom: PhantomData,
        }
    }

    /// Get the raw callback global ref (for storing in watcher structs).
    pub fn callback_ptr(&self) -> jlong {
        // We need to pass the GlobalRef's underlying pointer
        // This is a bit tricky - we'll use a boxed Arc for sharing
        self.callback.as_raw() as jlong
    }
}

impl<T> JavaWatcher<T> {
    /// Call the Java callback with a new value.
    ///
    /// This method attaches to the JVM (if not already attached), converts
    /// the Rust value to its Java representation, and calls the callback.
    ///
    /// Note: The actual implementation of type conversion will be done
    /// when we implement specific watcher types for each reactive value type.
    pub fn call_raw(&self, _metadata_ptr: jlong) {
        with_jni_env(|env| {
            // Call the callback's onUpdate method with null value for now
            // Signature: void onUpdate(Object value, long metadataPtr)
            // SAFETY: JNI call with valid method ID and callback
            unsafe {
                let result = env.call_method_unchecked(
                    &self.callback,
                    JMethodID::from_raw(self.method_id),
                    ReturnType::Primitive(Primitive::Void),
                    &[
                        JValue::Object(&jni::objects::JObject::null()).as_jni(),
                        JValue::Long(_metadata_ptr).as_jni(),
                    ],
                );

                if let Err(e) = result {
                    tracing::error!("JavaWatcher callback failed: {:?}", e);
                    // Clear any pending exception
                    if env.exception_check().unwrap_or(false) {
                        env.exception_clear().ok();
                    }
                }
            }
        });
    }
}

/// Watcher data structure passed from Java.
///
/// This matches the `WatcherStruct` in Kotlin:
/// ```kotlin
/// data class WatcherStruct(
///     val data: Long,      // Pointer to opaque data
///     val call: Long,      // Function pointer for callback
///     val drop: Long       // Function pointer for cleanup
/// )
/// ```
#[repr(C)]
pub struct WatcherStruct {
    /// Opaque data pointer (passed to call and drop functions).
    pub data: jlong,
    /// Callback function pointer.
    pub call: jlong,
    /// Drop function pointer for cleanup.
    pub drop: jlong,
}

impl WatcherStruct {
    /// Create a new WatcherStruct with the given components.
    pub fn new(data: jlong, call: jlong, drop: jlong) -> Self {
        Self { data, call, drop }
    }
}

/// A guard that cleans up a watcher when dropped.
///
/// This wraps a `WatcherStruct` and ensures the drop function is called
/// when the guard goes out of scope.
pub struct WatcherGuard {
    data: jlong,
    drop_fn: unsafe extern "C" fn(jlong),
}

impl WatcherGuard {
    /// Create a new WatcherGuard from a WatcherStruct.
    ///
    /// # Safety
    /// The WatcherStruct must contain valid function pointers.
    pub unsafe fn from_struct(watcher: WatcherStruct) -> Self {
        // SAFETY: Caller guarantees watcher.drop is a valid function pointer
        Self {
            data: watcher.data,
            drop_fn: unsafe { core::mem::transmute(watcher.drop) },
        }
    }
}

impl Drop for WatcherGuard {
    fn drop(&mut self) {
        unsafe {
            (self.drop_fn)(self.data);
        }
    }
}

/// Native watcher callback type for calling back into Java.
///
/// This matches the function signature expected by the FFI watcher system.
pub type NativeWatcherCallback<T> = unsafe extern "C" fn(
    data: *mut (),   // Opaque data pointer
    value: T,        // The new value
    metadata: jlong, // Metadata pointer
);

/// Native watcher drop function type.
pub type NativeWatcherDrop = unsafe extern "C" fn(data: *mut ());

/// Create a Rust watcher that calls into a Java callback.
///
/// This is used to convert a `WatcherStruct` from Java into a Rust
/// watcher function that can be passed to the reactive system.
///
/// # Safety
/// The WatcherStruct must contain valid function pointers.
pub unsafe fn create_rust_watcher<T, F>(
    watcher: WatcherStruct,
    converter: F,
) -> impl Fn(T, jlong) + Send + 'static
where
    T: 'static,
    F: Fn(T) -> T + Send + 'static,
{
    // Store the watcher data in an Arc so we can clone it for the closure
    let data = watcher.data;
    // SAFETY: Caller guarantees watcher.call and watcher.drop are valid function pointers
    let call_fn: NativeWatcherCallback<T> = unsafe { core::mem::transmute(watcher.call) };
    let drop_fn: NativeWatcherDrop = unsafe { core::mem::transmute(watcher.drop) };

    // Create a shared guard that will clean up when all references are dropped
    struct SharedWatcher {
        data: jlong,
        drop_fn: NativeWatcherDrop,
    }

    impl Drop for SharedWatcher {
        fn drop(&mut self) {
            unsafe {
                (self.drop_fn)(self.data as *mut ());
            }
        }
    }

    // Safety: The watcher data and functions are provided by Java and assumed valid
    unsafe impl Send for SharedWatcher {}
    unsafe impl Sync for SharedWatcher {}

    let shared = Arc::new(SharedWatcher { data, drop_fn });

    move |value: T, metadata: jlong| {
        let _guard = shared.clone(); // Keep the shared watcher alive
        let converted = converter(value);
        unsafe {
            call_fn(data as *mut (), converted, metadata);
        }
    }
}
