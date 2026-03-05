//! JNI bindings for WaterUI Android backend.
//!
//! This module provides pure Rust JNI bindings using jni-rs, replacing the
//! previous C++ JNI layer (waterui_jni.cpp).
//!
//! The module is only compiled when the `android-jni` feature is enabled.

extern crate std;

pub mod android_runtime;
pub mod components;
pub mod convert;
pub mod ffi_bridge;
pub mod navigation;
pub mod reactive;
pub mod watcher;
pub mod webview_bridge;

pub use convert::{FromJava, JniPrimitive, ToJava};
pub use watcher::JavaWatcher;

// Re-export jni types needed by macros
pub use jni;
pub use jni::objects::{GlobalRef, JClass, JObject, JValue};
pub use jni::sys::{JNI_VERSION_1_6, jboolean, jdouble, jfloat, jint, jlong, jobject};
pub use jni::{JNIEnv, JavaVM};

use std::sync::OnceLock;

/// Global reference to the JavaVM for attaching threads.
static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();

/// Global reference to commonly used Java classes.
static JAVA_CLASSES: OnceLock<JavaClasses> = OnceLock::new();

/// Cached Java class references for performance.
#[derive(Debug)]
pub struct JavaClasses {
    /// java.lang.Boolean
    pub boolean_class: GlobalRef,
    pub boolean_value_of: jni::sys::jmethodID,
    /// java.lang.Integer
    pub integer_class: GlobalRef,
    pub integer_value_of: jni::sys::jmethodID,
    /// java.lang.Long
    pub long_class: GlobalRef,
    pub long_value_of: jni::sys::jmethodID,
    /// java.lang.Float
    pub float_class: GlobalRef,
    pub float_value_of: jni::sys::jmethodID,
    /// java.lang.Double
    pub double_class: GlobalRef,
    pub double_value_of: jni::sys::jmethodID,
    /// dev.waterui.android.runtime.TypeIdStruct
    pub type_id_struct_class: GlobalRef,
    pub type_id_struct_ctor: jni::sys::jmethodID,
    /// dev.waterui.android.runtime.WatcherStruct
    pub watcher_struct_class: GlobalRef,
    pub watcher_struct_ctor: jni::sys::jmethodID,
    /// dev.waterui.android.reactive.WuiWatcherMetadata
    pub watcher_metadata_class: GlobalRef,
    pub watcher_metadata_ctor: jni::sys::jmethodID,
    /// dev.waterui.android.runtime.ResolvedColorStruct
    pub resolved_color_struct_class: GlobalRef,
    pub resolved_color_struct_ctor: jni::sys::jmethodID,
    /// dev.waterui.android.runtime.ResolvedFontStruct
    pub resolved_font_struct_class: GlobalRef,
    pub resolved_font_struct_ctor: jni::sys::jmethodID,
    /// dev.waterui.android.runtime.VideoStruct
    pub video_struct_class: GlobalRef,
    pub video_struct_ctor: jni::sys::jmethodID,
    /// dev.waterui.android.runtime.LivePhotoSourceStruct
    pub live_photo_source_struct_class: GlobalRef,
    pub live_photo_source_struct_ctor: jni::sys::jmethodID,
    /// dev.waterui.android.runtime.DateStruct
    pub date_struct_class: GlobalRef,
    pub date_struct_ctor: jni::sys::jmethodID,
    /// dev.waterui.android.runtime.RegionStruct
    pub region_struct_class: GlobalRef,
    pub region_struct_ctor: jni::sys::jmethodID,
    /// dev.waterui.android.runtime.AnnotationStruct
    pub annotation_struct_class: GlobalRef,
    pub annotation_struct_ctor: jni::sys::jmethodID,
    /// dev.waterui.android.runtime.TableColumnStruct
    pub table_column_struct_class: GlobalRef,
    pub table_column_struct_ctor: jni::sys::jmethodID,
    /// dev.waterui.android.runtime.PickerItemStruct
    pub picker_item_struct_class: GlobalRef,
    pub picker_item_struct_ctor: jni::sys::jmethodID,
    /// dev.waterui.android.runtime.TextStyleStruct
    pub text_style_struct_class: GlobalRef,
    pub text_style_struct_ctor: jni::sys::jmethodID,
    /// dev.waterui.android.runtime.StyledChunkStruct
    pub styled_chunk_struct_class: GlobalRef,
    pub styled_chunk_struct_ctor: jni::sys::jmethodID,
    /// dev.waterui.android.runtime.StyledStrStruct
    pub styled_str_struct_class: GlobalRef,
    pub styled_str_struct_ctor: jni::sys::jmethodID,
}

// Safety: JavaClasses contains GlobalRefs which are thread-safe by JNI spec
unsafe impl Send for JavaClasses {}
unsafe impl Sync for JavaClasses {}

/// Get the global JavaVM instance.
pub fn java_vm() -> &'static JavaVM {
    JAVA_VM
        .get()
        .expect("JavaVM not initialized - JNI_OnLoad not called")
}

/// Get cached Java classes.
pub fn java_classes() -> &'static JavaClasses {
    JAVA_CLASSES.get().expect("JavaClasses not initialized")
}

/// Initialize the JNI module. Called from JNI_OnLoad.
///
/// # Safety
/// Must only be called once from JNI_OnLoad.
pub unsafe fn init(vm: *mut jni::sys::JavaVM) -> jint {
    let vm = unsafe { JavaVM::from_raw(vm).unwrap_unchecked() };

    let mut env = unsafe { vm.get_env().unwrap_unchecked() };

    // Cache commonly used classes
    let classes = init_classes(&mut env);

    assert!(!(JAVA_VM.set(vm).is_err()), "JavaVM already initialized");
    assert!(
        !(JAVA_CLASSES.set(classes).is_err()),
        "JavaClasses already initialized"
    );

    tracing::debug!("WaterUI JNI module initialized");

    JNI_VERSION_1_6
}

fn init_classes(env: &mut JNIEnv) -> JavaClasses {
    // Boolean
    let boolean_class = env
        .find_class("java/lang/Boolean")
        .expect("Boolean class not found");
    let boolean_class = env
        .new_global_ref(boolean_class)
        .expect("Failed to create global ref");
    let boolean_value_of = env
        .get_static_method_id(&boolean_class, "valueOf", "(Z)Ljava/lang/Boolean;")
        .expect("Boolean.valueOf not found")
        .into_raw();

    // Integer
    let integer_class = env
        .find_class("java/lang/Integer")
        .expect("Integer class not found");
    let integer_class = env
        .new_global_ref(integer_class)
        .expect("Failed to create global ref");
    let integer_value_of = env
        .get_static_method_id(&integer_class, "valueOf", "(I)Ljava/lang/Integer;")
        .expect("Integer.valueOf not found")
        .into_raw();

    // Long
    let long_class = env
        .find_class("java/lang/Long")
        .expect("Long class not found");
    let long_class = env
        .new_global_ref(long_class)
        .expect("Failed to create global ref");
    let long_value_of = env
        .get_static_method_id(&long_class, "valueOf", "(J)Ljava/lang/Long;")
        .expect("Long.valueOf not found")
        .into_raw();

    // Float
    let float_class = env
        .find_class("java/lang/Float")
        .expect("Float class not found");
    let float_class = env
        .new_global_ref(float_class)
        .expect("Failed to create global ref");
    let float_value_of = env
        .get_static_method_id(&float_class, "valueOf", "(F)Ljava/lang/Float;")
        .expect("Float.valueOf not found")
        .into_raw();

    // Double
    let double_class = env
        .find_class("java/lang/Double")
        .expect("Double class not found");
    let double_class = env
        .new_global_ref(double_class)
        .expect("Failed to create global ref");
    let double_value_of = env
        .get_static_method_id(&double_class, "valueOf", "(D)Ljava/lang/Double;")
        .expect("Double.valueOf not found")
        .into_raw();

    // TypeIdStruct
    let type_id_struct_class = env
        .find_class("dev/waterui/android/runtime/TypeIdStruct")
        .expect("TypeIdStruct class not found");
    let type_id_struct_class = env
        .new_global_ref(type_id_struct_class)
        .expect("Failed to create global ref");
    let type_id_struct_ctor = env
        .get_method_id(&type_id_struct_class, "<init>", "(JJ)V")
        .expect("TypeIdStruct constructor not found")
        .into_raw();

    // WatcherStruct
    let watcher_struct_class = env
        .find_class("dev/waterui/android/runtime/WatcherStruct")
        .expect("WatcherStruct class not found");
    let watcher_struct_class = env
        .new_global_ref(watcher_struct_class)
        .expect("Failed to create global ref");
    let watcher_struct_ctor = env
        .get_method_id(&watcher_struct_class, "<init>", "(JJJ)V")
        .expect("WatcherStruct constructor not found")
        .into_raw();

    // WuiWatcherMetadata
    let watcher_metadata_class = env
        .find_class("dev/waterui/android/reactive/WuiWatcherMetadata")
        .expect("WuiWatcherMetadata class not found");
    let watcher_metadata_class = env
        .new_global_ref(watcher_metadata_class)
        .expect("Failed to create global ref");
    let watcher_metadata_ctor = env
        .get_method_id(&watcher_metadata_class, "<init>", "(J)V")
        .expect("WuiWatcherMetadata constructor not found")
        .into_raw();

    // ResolvedColorStruct
    let resolved_color_struct_class = env
        .find_class("dev/waterui/android/runtime/ResolvedColorStruct")
        .expect("ResolvedColorStruct class not found");
    let resolved_color_struct_class = env
        .new_global_ref(resolved_color_struct_class)
        .expect("Failed to create global ref");
    let resolved_color_struct_ctor = env
        .get_method_id(&resolved_color_struct_class, "<init>", "(FFFFF)V")
        .expect("ResolvedColorStruct constructor not found")
        .into_raw();

    // ResolvedFontStruct
    let resolved_font_struct_class = env
        .find_class("dev/waterui/android/runtime/ResolvedFontStruct")
        .expect("ResolvedFontStruct class not found");
    let resolved_font_struct_class = env
        .new_global_ref(resolved_font_struct_class)
        .expect("Failed to create global ref");
    let resolved_font_struct_ctor = env
        .get_method_id(
            &resolved_font_struct_class,
            "<init>",
            "(FILjava/lang/String;)V",
        )
        .expect("ResolvedFontStruct constructor not found")
        .into_raw();

    // VideoStruct
    let video_struct_class = env
        .find_class("dev/waterui/android/runtime/VideoStruct")
        .expect("VideoStruct class not found");
    let video_struct_class = env
        .new_global_ref(video_struct_class)
        .expect("Failed to create global ref");
    let video_struct_ctor = env
        .get_method_id(&video_struct_class, "<init>", "(Ljava/lang/String;)V")
        .expect("VideoStruct constructor not found")
        .into_raw();

    // LivePhotoSourceStruct
    let live_photo_source_struct_class = env
        .find_class("dev/waterui/android/runtime/LivePhotoSourceStruct")
        .expect("LivePhotoSourceStruct class not found");
    let live_photo_source_struct_class = env
        .new_global_ref(live_photo_source_struct_class)
        .expect("Failed to create global ref");
    let live_photo_source_struct_ctor = env
        .get_method_id(
            &live_photo_source_struct_class,
            "<init>",
            "(Ljava/lang/String;Ljava/lang/String;)V",
        )
        .expect("LivePhotoSourceStruct constructor not found")
        .into_raw();

    // DateStruct
    let date_struct_class = env
        .find_class("dev/waterui/android/runtime/DateStruct")
        .expect("DateStruct class not found");
    let date_struct_class = env
        .new_global_ref(date_struct_class)
        .expect("Failed to create global ref");
    let date_struct_ctor = env
        .get_method_id(&date_struct_class, "<init>", "(III)V")
        .expect("DateStruct constructor not found")
        .into_raw();

    // RegionStruct
    let region_struct_class = env
        .find_class("dev/waterui/android/runtime/RegionStruct")
        .expect("RegionStruct class not found");
    let region_struct_class = env
        .new_global_ref(region_struct_class)
        .expect("Failed to create global ref");
    let region_struct_ctor = env
        .get_method_id(&region_struct_class, "<init>", "(DDDD)V")
        .expect("RegionStruct constructor not found")
        .into_raw();

    // AnnotationStruct
    let annotation_struct_class = env
        .find_class("dev/waterui/android/runtime/AnnotationStruct")
        .expect("AnnotationStruct class not found");
    let annotation_struct_class = env
        .new_global_ref(annotation_struct_class)
        .expect("Failed to create global ref");
    let annotation_struct_ctor = env
        .get_method_id(
            &annotation_struct_class,
            "<init>",
            "(DDLjava/lang/String;Ljava/lang/String;)V",
        )
        .expect("AnnotationStruct constructor not found")
        .into_raw();

    // TableColumnStruct
    let table_column_struct_class = env
        .find_class("dev/waterui/android/runtime/TableColumnStruct")
        .expect("TableColumnStruct class not found");
    let table_column_struct_class = env
        .new_global_ref(table_column_struct_class)
        .expect("Failed to create global ref");
    let table_column_struct_ctor = env
        .get_method_id(&table_column_struct_class, "<init>", "(JJ)V")
        .expect("TableColumnStruct constructor not found")
        .into_raw();

    // PickerItemStruct
    let picker_item_struct_class = env
        .find_class("dev/waterui/android/runtime/PickerItemStruct")
        .expect("PickerItemStruct class not found");
    let picker_item_struct_class = env
        .new_global_ref(picker_item_struct_class)
        .expect("Failed to create global ref");
    let picker_item_struct_ctor = env
        .get_method_id(
            &picker_item_struct_class,
            "<init>",
            "(ILdev/waterui/android/runtime/StyledStrStruct;)V",
        )
        .expect("PickerItemStruct constructor not found")
        .into_raw();

    // TextStyleStruct
    let text_style_struct_class = env
        .find_class("dev/waterui/android/runtime/TextStyleStruct")
        .expect("TextStyleStruct class not found");
    let text_style_struct_class = env
        .new_global_ref(text_style_struct_class)
        .expect("Failed to create global ref");
    let text_style_struct_ctor = env
        .get_method_id(&text_style_struct_class, "<init>", "(JZZZJJ)V")
        .expect("TextStyleStruct constructor not found")
        .into_raw();

    // StyledChunkStruct
    let styled_chunk_struct_class = env
        .find_class("dev/waterui/android/runtime/StyledChunkStruct")
        .expect("StyledChunkStruct class not found");
    let styled_chunk_struct_class = env
        .new_global_ref(styled_chunk_struct_class)
        .expect("Failed to create global ref");
    let styled_chunk_struct_ctor = env
        .get_method_id(
            &styled_chunk_struct_class,
            "<init>",
            "(Ljava/lang/String;Ldev/waterui/android/runtime/TextStyleStruct;)V",
        )
        .expect("StyledChunkStruct constructor not found")
        .into_raw();

    // StyledStrStruct
    let styled_str_struct_class = env
        .find_class("dev/waterui/android/runtime/StyledStrStruct")
        .expect("StyledStrStruct class not found");
    let styled_str_struct_class = env
        .new_global_ref(styled_str_struct_class)
        .expect("Failed to create global ref");
    let styled_str_struct_ctor = env
        .get_method_id(
            &styled_str_struct_class,
            "<init>",
            "([Ldev/waterui/android/runtime/StyledChunkStruct;)V",
        )
        .expect("StyledStrStruct constructor not found")
        .into_raw();

    JavaClasses {
        boolean_class,
        boolean_value_of,
        integer_class,
        integer_value_of,
        long_class,
        long_value_of,
        float_class,
        float_value_of,
        double_class,
        double_value_of,
        type_id_struct_class,
        type_id_struct_ctor,
        watcher_struct_class,
        watcher_struct_ctor,
        watcher_metadata_class,
        watcher_metadata_ctor,
        resolved_color_struct_class,
        resolved_color_struct_ctor,
        resolved_font_struct_class,
        resolved_font_struct_ctor,
        video_struct_class,
        video_struct_ctor,
        live_photo_source_struct_class,
        live_photo_source_struct_ctor,
        date_struct_class,
        date_struct_ctor,
        region_struct_class,
        region_struct_ctor,
        annotation_struct_class,
        annotation_struct_ctor,
        table_column_struct_class,
        table_column_struct_ctor,
        picker_item_struct_class,
        picker_item_struct_ctor,
        text_style_struct_class,
        text_style_struct_ctor,
        styled_chunk_struct_class,
        styled_chunk_struct_ctor,
        styled_str_struct_class,
        styled_str_struct_ctor,
    }
}

/// Convert a WuiTypeId to a Java TypeIdStruct object.
pub fn type_id_to_java<'local>(
    env: &mut JNIEnv<'local>,
    type_id: crate::WuiTypeId,
) -> JObject<'local> {
    let classes = java_classes();

    // TypeIdStruct is (low, high) to match Kotlin WuiTypeId(low, high).
    let low = type_id.low as jlong;
    let high = type_id.high as jlong;

    unsafe {
        let obj = env
            .new_object_unchecked(
                &classes.type_id_struct_class,
                jni::objects::JMethodID::from_raw(classes.type_id_struct_ctor),
                &[JValue::Long(low).as_jni(), JValue::Long(high).as_jni()],
            )
            .expect("Failed to create TypeIdStruct");
        obj
    }
}

/// Convert a Rust pointer to a Java long (jlong).
#[inline]
pub fn ptr_to_jlong<T>(ptr: *mut T) -> jlong {
    ptr as jlong
}

/// Convert a Java long (jlong) to a Rust pointer.
///
/// # Safety
/// The jlong must have been created from a valid pointer.
#[inline]
pub unsafe fn jlong_to_ptr<T>(val: jlong) -> *mut T {
    val as *mut T
}

/// Attach the current thread to the JVM and run a closure.
///
/// This is useful for callbacks from Rust into Java on non-JNI threads.
pub fn with_jni_env<F, R>(f: F) -> R
where
    F: FnOnce(&mut JNIEnv) -> R,
{
    let vm = java_vm();
    let mut env = vm
        .attach_current_thread()
        .expect("Failed to attach thread to JVM");
    f(&mut env)
}

/// Extracts (data_ptr, call_ptr, drop_ptr) from a Java WatcherStruct object.
///
/// This function is used by the generated JNI watch functions in `ffi_computed!`
/// and `ffi_binding!` macros to extract the watcher callback pointers.
pub fn extract_watcher_struct(env: &mut JNIEnv, watcher: &JObject) -> (jlong, jlong, jlong) {
    let data_ptr = env
        .get_field(watcher, "dataPtr", "J")
        .expect("Failed to get dataPtr")
        .j()
        .expect("dataPtr is not a long");
    let call_ptr = env
        .get_field(watcher, "callPtr", "J")
        .expect("Failed to get callPtr")
        .j()
        .expect("callPtr is not a long");
    let drop_ptr = env
        .get_field(watcher, "dropPtr", "J")
        .expect("Failed to get dropPtr")
        .j()
        .expect("dropPtr is not a long");
    (data_ptr, call_ptr, drop_ptr)
}

// Note: JNI_OnLoad is generated by the export!() macro in lib.rs
// to ensure all initialization happens in the correct order.
