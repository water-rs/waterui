//! Type conversion traits for Rust ↔ Java interop.
//!
//! These traits provide bidirectional conversion between Rust types and
//! Java types across the JNI boundary.

extern crate alloc;
extern crate std;

use alloc::vec::Vec;
use alloc::{boxed::Box, string::String};
use jni::JNIEnv;
use jni::objects::{JCharArray, JObject, JString, JValue};
use jni::sys::{jboolean, jdouble, jfloat, jint, jlong};

/// Copies a Java string through its UTF-16 representation without lossy replacement.
pub(crate) fn string_from_java<'local>(env: &mut JNIEnv<'local>, java: &JString<'local>) -> String {
    let chars = env
        .call_method(java, "toCharArray", "()[C", &[])
        .expect("Java String.toCharArray failed")
        .l()
        .expect("String.toCharArray returned a non-array value");
    let chars: &JCharArray<'local> = (&chars).into();
    let len = usize::try_from(
        env.get_array_length(chars)
            .expect("failed to read Java string length"),
    )
    .expect("Java string length must fit usize");
    let mut utf16 = alloc::vec![0; len];
    env.get_char_array_region(chars, 0, &mut utf16)
        .expect("failed to copy Java string");
    String::from_utf16(&utf16).expect("Java string contains an unpaired UTF-16 surrogate")
}

/// Releases an owned FFI array after its elements have been copied or
/// transferred into Java owner objects.
///
/// # Safety
///
/// `array` must be the sole owning FFI array and must not be consumed again.
unsafe fn consume_ffi_array<T>(array: &crate::array::WuiArray<T>) {
    unsafe { core::ptr::read(array).consume() };
}

// ============================================================================
// JniPrimitive trait for macro-based code generation
// ============================================================================

/// Trait for types that can be directly passed across JNI without heap allocation.
///
/// This trait is used by `jni_binding_primitive!` and `jni_computed_primitive!` macros
/// to generate read/set functions for reactive bindings.
///
pub trait JniPrimitive: Sized + 'static {
    /// The corresponding JNI primitive type (jboolean, jint, jfloat, jdouble, jlong)
    type Jni;
    /// Convert from Rust to JNI
    fn to_jni(self) -> Self::Jni;
    /// Convert from JNI to Rust
    fn from_jni(val: Self::Jni) -> Self;
}

impl JniPrimitive for bool {
    type Jni = jboolean;
    fn to_jni(self) -> Self::Jni {
        if self { 1 } else { 0 }
    }
    fn from_jni(val: Self::Jni) -> Self {
        val != 0
    }
}

impl JniPrimitive for i32 {
    type Jni = jint;
    fn to_jni(self) -> Self::Jni {
        self
    }
    fn from_jni(val: Self::Jni) -> Self {
        val
    }
}

impl JniPrimitive for f32 {
    type Jni = jfloat;
    fn to_jni(self) -> Self::Jni {
        self
    }
    fn from_jni(val: Self::Jni) -> Self {
        val
    }
}

impl JniPrimitive for f64 {
    type Jni = jdouble;
    fn to_jni(self) -> Self::Jni {
        self
    }
    fn from_jni(val: Self::Jni) -> Self {
        val
    }
}

// ============================================================================
// Pointer conversions
// ============================================================================

/// Convert jlong back to a raw pointer.
///
/// # Safety
/// The jlong must have been created from a valid pointer of type T.
#[inline]
pub unsafe fn jlong_to_ptr<T>(val: jlong) -> *const T {
    val as *const T
}

/// Convert jlong back to a mutable raw pointer.
///
/// # Safety
/// The jlong must have been created from a valid pointer of type T.
#[inline]
pub unsafe fn jlong_to_ptr_mut<T>(val: jlong) -> *mut T {
    val as *mut T
}

// ============================================================================
// Struct to Java conversion
// ============================================================================

/// Trait for FFI structs that can be converted to Java objects.
pub trait ToJavaStruct {
    /// Convert this FFI struct to a Java object.
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local>;
}

/// Convert an FFI struct to a Java object.
///
/// This uses the `ToJavaStruct` trait to perform type-specific conversions.
pub fn struct_to_java<'local, T: ToJavaStruct>(
    env: &mut JNIEnv<'local>,
    value: &T,
) -> JObject<'local> {
    value.to_java_struct(env)
}

// ============================================================================
// ToJavaStruct implementations for Metadata types
// ============================================================================

use crate::{WuiEnv, WuiMetadata};

/// MetadataEnvStruct(contentPtr: Long, envPtr: Long)
impl ToJavaStruct for WuiMetadata<*mut WuiEnv> {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataEnvStruct")
            .expect("MetadataEnvStruct class not found");
        env.new_object(
            &class,
            "(JJ)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Long(self.value as jlong),
            ],
        )
        .expect("Failed to create MetadataEnvStruct")
    }
}

/// MetadataNavigationTransitionStruct(contentPtr: Long, id: Int)
impl ToJavaStruct for WuiMetadata<crate::id::WuiId> {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataNavigationTransitionStruct")
            .expect("MetadataNavigationTransitionStruct class not found");
        env.new_object(
            &class,
            "(JI)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Int(self.value.inner),
            ],
        )
        .expect("Failed to create MetadataNavigationTransitionStruct")
    }
}

/// MetadataSecureStruct(contentPtr: Long)
/// Note: WuiSecureMarker is just a marker type, no additional data needed
impl ToJavaStruct for crate::WuiMetadataSecure {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataSecureStruct")
            .expect("MetadataSecureStruct class not found");
        env.new_object(&class, "(J)V", &[JValue::Long(self.content as jlong)])
            .expect("Failed to create MetadataSecureStruct")
    }
}

/// MetadataLifecycleHookStruct(contentPtr: Long, lifecycleType: Int, handlerPtr: Long)
impl ToJavaStruct for crate::WuiMetadataLifecycleHook {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataLifecycleHookStruct")
            .expect("MetadataLifecycleHookStruct class not found");
        env.new_object(
            &class,
            "(JIJ)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Int(self.value.lifecycle as i32),
                JValue::Long(self.value.handler as jlong),
            ],
        )
        .expect("Failed to create MetadataLifecycleHookStruct")
    }
}

/// MetadataGestureStruct(contentPtr: Long, gestureType: Int, gestureData: GestureDataStruct, actionPtr: Long)
/// Note: WuiGestureObserver has `gesture: WuiGesture` and `action: *mut WuiAction`
fn gesture_parts(gesture: &crate::gesture::WuiGesture) -> (i32, i32, i32, f32, f32, f32, i64, i64) {
    match gesture {
        crate::gesture::WuiGesture::Tap { count } => (
            0i32,
            *count as i32,
            0i32,
            0.0f32,
            0.0f32,
            0.0f32,
            0i64,
            0i64,
        ),
        crate::gesture::WuiGesture::LongPress { duration } => (
            1i32,
            0i32,
            *duration as i32,
            0.0f32,
            0.0f32,
            0.0f32,
            0i64,
            0i64,
        ),
        crate::gesture::WuiGesture::Drag { min_distance } => {
            (2i32, 0i32, 0i32, *min_distance, 0.0f32, 0.0f32, 0i64, 0i64)
        }
        crate::gesture::WuiGesture::Magnification { initial_scale } => {
            (3i32, 0i32, 0i32, 0.0f32, *initial_scale, 0.0f32, 0i64, 0i64)
        }
        crate::gesture::WuiGesture::Rotation { initial_angle } => {
            (4i32, 0i32, 0i32, 0.0f32, 0.0f32, *initial_angle, 0i64, 0i64)
        }
        crate::gesture::WuiGesture::Then { first, then } => (
            5i32,
            0i32,
            0i32,
            0.0f32,
            0.0f32,
            0.0f32,
            *first as jlong,
            *then as jlong,
        ),
        crate::gesture::WuiGesture::Simultaneous { first, second } => (
            6i32,
            0i32,
            0i32,
            0.0f32,
            0.0f32,
            0.0f32,
            *first as jlong,
            *second as jlong,
        ),
        crate::gesture::WuiGesture::Exclusive { first, second } => (
            7i32,
            0i32,
            0i32,
            0.0f32,
            0.0f32,
            0.0f32,
            *first as jlong,
            *second as jlong,
        ),
    }
}

fn gesture_data_to_java<'local>(
    env: &mut JNIEnv<'local>,
    gesture_data_class: &jni::objects::JClass<'local>,
    gesture: &crate::gesture::WuiGesture,
) -> JObject<'local> {
    let (
        _gesture_type,
        tap_count,
        long_press_duration,
        drag_min_distance,
        magnification_scale,
        rotation_angle,
        first_ptr,
        second_ptr,
    ) = gesture_parts(gesture);

    env.new_object(
        gesture_data_class,
        "(IIFFFJJ)V",
        &[
            JValue::Int(tap_count),
            JValue::Int(long_press_duration),
            JValue::Float(drag_min_distance),
            JValue::Float(magnification_scale),
            JValue::Float(rotation_angle),
            JValue::Long(first_ptr),
            JValue::Long(second_ptr),
        ],
    )
    .expect("Failed to create GestureDataStruct")
}

impl ToJavaStruct for crate::gesture::WuiGesture {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let gesture_data_class = env
            .find_class("dev/waterui/android/runtime/GestureDataStruct")
            .expect("GestureDataStruct class not found");
        let class = env
            .find_class("dev/waterui/android/runtime/GestureStruct")
            .expect("GestureStruct class not found");
        let (gesture_type, _, _, _, _, _, _, _) = gesture_parts(self);
        let gesture_data = gesture_data_to_java(env, &gesture_data_class, self);
        env.new_object(
            &class,
            "(ILdev/waterui/android/runtime/GestureDataStruct;)V",
            &[JValue::Int(gesture_type), JValue::Object(&gesture_data)],
        )
        .expect("Failed to create GestureStruct")
    }
}

impl ToJavaStruct for crate::WuiMetadataGesture {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        // Create GestureDataStruct first based on gesture type
        let gesture_data_class = env
            .find_class("dev/waterui/android/runtime/GestureDataStruct")
            .expect("GestureDataStruct class not found");
        let (gesture_type, _, _, _, _, _, _, _) = gesture_parts(&self.value.gesture);
        let gesture_data = gesture_data_to_java(env, &gesture_data_class, &self.value.gesture);

        let class = env
            .find_class("dev/waterui/android/runtime/MetadataGestureStruct")
            .expect("MetadataGestureStruct class not found");
        env.new_object(
            &class,
            "(JILdev/waterui/android/runtime/GestureDataStruct;J)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Int(gesture_type),
                JValue::Object(&gesture_data),
                JValue::Long(self.value.action as jlong),
            ],
        )
        .expect("Failed to create MetadataGestureStruct")
    }
}

/// MetadataOnEventStruct(contentPtr: Long, eventType: Int, handlerPtr: Long)
impl ToJavaStruct for crate::WuiMetadataOnEvent {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataOnEventStruct")
            .expect("MetadataOnEventStruct class not found");
        env.new_object(
            &class,
            "(JIJ)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Int(self.value.event as i32),
                JValue::Long(self.value.handler as jlong),
            ],
        )
        .expect("Failed to create MetadataOnEventStruct")
    }
}

/// MetadataCursorStruct(contentPtr: Long, stylePtr: Long)
impl ToJavaStruct for crate::WuiMetadataCursor {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataCursorStruct")
            .expect("MetadataCursorStruct class not found");
        env.new_object(
            &class,
            "(JJ)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Long(self.value.style as jlong),
            ],
        )
        .expect("Failed to create MetadataCursorStruct")
    }
}

/// MetadataShadowStruct(contentPtr: Long, colorPtr: Long, offsetX: Float, offsetY: Float, radius: Float)
impl ToJavaStruct for crate::WuiMetadataShadow {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataShadowStruct")
            .expect("MetadataShadowStruct class not found");
        env.new_object(
            &class,
            "(JJFFF)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Long(self.value.color as jlong),
                JValue::Float(self.value.offset_x),
                JValue::Float(self.value.offset_y),
                JValue::Float(self.value.radius),
            ],
        )
        .expect("Failed to create MetadataShadowStruct")
    }
}

/// MetadataBorderStruct(contentPtr: Long, colorPtr: Long, width: Float, cornerRadius: Float, edges: Int)
impl ToJavaStruct for crate::WuiMetadataBorder {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataBorderStruct")
            .expect("MetadataBorderStruct class not found");
        // Pack edges into a bitmask: top=1, bottom=2, leading=4, trailing=8
        let edges_bits = (if self.value.edges.top { 1i32 } else { 0 })
            | (if self.value.edges.bottom { 2i32 } else { 0 })
            | (if self.value.edges.leading { 4i32 } else { 0 })
            | (if self.value.edges.trailing { 8i32 } else { 0 });
        env.new_object(
            &class,
            "(JJFFI)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Long(self.value.color as jlong),
                JValue::Float(self.value.width),
                JValue::Float(self.value.corner_radius),
                JValue::Int(edges_bits),
            ],
        )
        .expect("Failed to create MetadataBorderStruct")
    }
}

/// MetadataScaleStruct(contentPtr: Long, scaleXPtr: Long, scaleYPtr: Long, anchorX: Float, anchorY: Float)
impl ToJavaStruct for crate::WuiMetadataScale {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataScaleStruct")
            .expect("MetadataScaleStruct class not found");
        env.new_object(
            &class,
            "(JJJFF)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Long(self.value.x as jlong),
                JValue::Long(self.value.y as jlong),
                JValue::Float(self.value.anchor.x),
                JValue::Float(self.value.anchor.y),
            ],
        )
        .expect("Failed to create MetadataScaleStruct")
    }
}

/// MetadataRotationStruct(contentPtr: Long, anglePtr: Long, anchorX: Float, anchorY: Float)
impl ToJavaStruct for crate::WuiMetadataRotation {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataRotationStruct")
            .expect("MetadataRotationStruct class not found");
        env.new_object(
            &class,
            "(JJFF)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Long(self.value.angle as jlong),
                JValue::Float(self.value.anchor.x),
                JValue::Float(self.value.anchor.y),
            ],
        )
        .expect("Failed to create MetadataRotationStruct")
    }
}

/// MetadataOffsetStruct(contentPtr: Long, offsetXPtr: Long, offsetYPtr: Long)
impl ToJavaStruct for crate::WuiMetadataOffset {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataOffsetStruct")
            .expect("MetadataOffsetStruct class not found");
        env.new_object(
            &class,
            "(JJJ)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Long(self.value.x as jlong),
                JValue::Long(self.value.y as jlong),
            ],
        )
        .expect("Failed to create MetadataOffsetStruct")
    }
}

/// MetadataOpacityStruct(contentPtr: Long, valuePtr: Long)
impl ToJavaStruct for crate::WuiMetadataOpacity {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataOpacityStruct")
            .expect("MetadataOpacityStruct class not found");
        env.new_object(
            &class,
            "(JJ)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Long(self.value.value as jlong),
            ],
        )
        .expect("Failed to create MetadataOpacityStruct")
    }
}

/// MetadataFocusedStruct(contentPtr: Long, bindingPtr: Long)
impl ToJavaStruct for crate::WuiMetadataFocused {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataFocusedStruct")
            .expect("MetadataFocusedStruct class not found");
        env.new_object(
            &class,
            "(JJ)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Long(self.value.binding as jlong),
            ],
        )
        .expect("Failed to create MetadataFocusedStruct")
    }
}

/// MetadataIgnoreSafeAreaStruct(contentPtr: Long, top: Boolean, bottom: Boolean, leading: Boolean, trailing: Boolean)
impl ToJavaStruct for crate::WuiMetadataIgnoreSafeArea {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataIgnoreSafeAreaStruct")
            .expect("MetadataIgnoreSafeAreaStruct class not found");
        env.new_object(
            &class,
            "(JZZZZ)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Bool(if self.value.edges.top { 1 } else { 0 }),
                JValue::Bool(if self.value.edges.bottom { 1 } else { 0 }),
                JValue::Bool(if self.value.edges.leading { 1 } else { 0 }),
                JValue::Bool(if self.value.edges.trailing { 1 } else { 0 }),
            ],
        )
        .expect("Failed to create MetadataIgnoreSafeAreaStruct")
    }
}

/// MetadataRetainStruct(contentPtr: Long, retainPtr: Long)
impl ToJavaStruct for crate::WuiMetadataRetain {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataRetainStruct")
            .expect("MetadataRetainStruct class not found");
        let retain_ptr = self.value.opaque_ptr() as jlong;
        env.new_object(
            &class,
            "(JJ)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Long(retain_ptr),
            ],
        )
        .expect("Failed to create MetadataRetainStruct")
    }
}

/// MetadataClipShapeStruct(contentPtr: Long, commands: Array<PathCommandStruct>)
impl ToJavaStruct for crate::WuiMetadataClipShape {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        // Create PathCommandStruct array
        let commands = self.value.commands.as_slice();
        let path_command_class = env
            .find_class("dev/waterui/android/runtime/PathCommandStruct")
            .expect("PathCommandStruct class not found");

        let java_array = env
            .new_object_array(commands.len() as i32, &path_command_class, JObject::null())
            .expect("Failed to create PathCommandStruct array");

        for (i, cmd) in commands.iter().enumerate() {
            let path_cmd = create_path_command_struct(env, cmd);
            env.set_object_array_element(&java_array, i as i32, &path_cmd)
                .expect("Failed to set path command element");
        }

        let class = env
            .find_class("dev/waterui/android/runtime/MetadataClipShapeStruct")
            .expect("MetadataClipShapeStruct class not found");
        let object = env
            .new_object(
                &class,
                "(J[Ldev/waterui/android/runtime/PathCommandStruct;)V",
                &[
                    JValue::Long(self.content as jlong),
                    JValue::Object(&java_array),
                ],
            )
            .expect("Failed to create MetadataClipShapeStruct");
        unsafe { consume_ffi_array(&self.value.commands) };
        object
    }
}

/// MetadataContextMenuStruct(contentPtr: Long, itemsPtr: Long)
impl ToJavaStruct for crate::WuiMetadataContextMenu {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataContextMenuStruct")
            .expect("MetadataContextMenuStruct class not found");
        env.new_object(
            &class,
            "(JJ)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Long(self.value.items as jlong),
            ],
        )
        .expect("Failed to create MetadataContextMenuStruct")
    }
}

/// MetadataHittableStruct(contentPtr: Long, enabledPtr: Long)
impl ToJavaStruct for crate::WuiMetadataHittable {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataHittableStruct")
            .expect("MetadataHittableStruct class not found");
        env.new_object(
            &class,
            "(JJ)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Long(self.value.enabled as jlong),
            ],
        )
        .expect("Failed to create MetadataHittableStruct")
    }
}

/// Both dynamic-range markers carry the same content-only JNI projection. The
/// registered renderer derives SDR versus HDR from the view's distinct type ID.
impl ToJavaStruct for crate::WuiMetadata<crate::WuiDynamicRangeMarker> {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MetadataDynamicRangeStruct")
            .expect("MetadataDynamicRangeStruct class not found");
        env.new_object(&class, "(J)V", &[JValue::Long(self.content as jlong)])
            .expect("Failed to create MetadataDynamicRangeStruct")
    }
}

#[cfg(target_os = "android")]
impl ToJavaStruct for crate::components::media::video::WuiAndroidVideoSurfaceHost {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/AndroidVideoSurfaceHostStruct")
            .expect("AndroidVideoSurfaceHostStruct class not found");
        env.new_object(
            &class,
            "(JJ)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Long(self.bridge as jlong),
            ],
        )
        .expect("Failed to create AndroidVideoSurfaceHostStruct")
    }
}

/// Helper to create a PathCommandStruct Java object from a WuiPathCommand
fn create_path_command_struct<'local>(
    env: &mut JNIEnv<'local>,
    cmd: &crate::WuiPathCommand,
) -> JObject<'local> {
    let class = env
        .find_class("dev/waterui/android/runtime/PathCommandStruct")
        .expect("PathCommandStruct class not found");

    // Extract values based on the command type
    // PathCommandStruct(tag, x, y, cx, cy, c1x, c1y, c2x, c2y, rx, ry, start, sweep)
    let (tag, x, y, cx, cy, c1x, c1y, c2x, c2y, rx, ry, start, sweep) = match cmd {
        crate::WuiPathCommand::MoveTo { x, y } => {
            (0, *x, *y, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        }
        crate::WuiPathCommand::LineTo { x, y } => {
            (1, *x, *y, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        }
        crate::WuiPathCommand::QuadTo { cx, cy, x, y } => {
            (2, *x, *y, *cx, *cy, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        }
        crate::WuiPathCommand::CubicTo {
            c1x,
            c1y,
            c2x,
            c2y,
            x,
            y,
        } => (
            3, *x, *y, 0.0, 0.0, *c1x, *c1y, *c2x, *c2y, 0.0, 0.0, 0.0, 0.0,
        ),
        crate::WuiPathCommand::Arc {
            cx,
            cy,
            rx,
            ry,
            start,
            sweep,
        } => (
            4, 0.0, 0.0, *cx, *cy, 0.0, 0.0, 0.0, 0.0, *rx, *ry, *start, *sweep,
        ),
        crate::WuiPathCommand::Close => (
            5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ),
    };

    env.new_object(
        &class,
        "(IFFFFFFFFFFFF)V",
        &[
            JValue::Int(tag),
            JValue::Float(x),
            JValue::Float(y),
            JValue::Float(cx),
            JValue::Float(cy),
            JValue::Float(c1x),
            JValue::Float(c1y),
            JValue::Float(c2x),
            JValue::Float(c2y),
            JValue::Float(rx),
            JValue::Float(ry),
            JValue::Float(start),
            JValue::Float(sweep),
        ],
    )
    .expect("Failed to create PathCommandStruct")
}

// ============================================================================
// ToJavaStruct implementations for View types
// ============================================================================

/// WuiStr -> PlainStruct(text: String)
impl ToJavaStruct for crate::WuiStr {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let value: waterui::Str = unsafe { crate::IntoRust::into_rust(core::ptr::read(self)) };
        let text = env
            .new_string(value.as_str())
            .expect("Failed to create plain text string");

        let class = env
            .find_class("dev/waterui/android/runtime/PlainStruct")
            .expect("PlainStruct class not found");
        env.new_object(&class, "(Ljava/lang/String;)V", &[JValue::Object(&text)])
            .expect("Failed to create PlainStruct")
    }
}

/// WuiResolvedColor -> ResolvedColorStruct(red, green, blue, opacity, headroom)
impl ToJavaStruct for crate::color::WuiResolvedColor {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/ResolvedColorStruct")
            .expect("ResolvedColorStruct class not found");
        env.new_object(
            &class,
            "(FFFFF)V",
            &[
                JValue::Float(self.red),
                JValue::Float(self.green),
                JValue::Float(self.blue),
                JValue::Float(self.opacity),
                JValue::Float(self.headroom),
            ],
        )
        .expect("Failed to create ResolvedColorStruct")
    }
}

/// WuiResolvedGradientStop -> ResolvedGradientStopStruct(position, color)
impl ToJavaStruct for crate::gradient::WuiResolvedGradientStop {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/ResolvedGradientStopStruct")
            .expect("ResolvedGradientStopStruct class not found");
        let color = crate::color::WuiResolvedColor {
            red: self.color.red,
            green: self.color.green,
            blue: self.color.blue,
            opacity: self.color.opacity,
            headroom: self.color.headroom,
        };
        let java_color = color.to_java_struct(env);
        env.new_object(
            &class,
            "(FLdev/waterui/android/runtime/ResolvedColorStruct;)V",
            &[JValue::Float(self.position), JValue::Object(&java_color)],
        )
        .expect("Failed to create ResolvedGradientStopStruct")
    }
}

/// WuiResolvedGradient -> ResolvedGradientStruct(...)
impl ToJavaStruct for crate::gradient::WuiResolvedGradient {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let stop_class = env
            .find_class("dev/waterui/android/runtime/ResolvedGradientStopStruct")
            .expect("ResolvedGradientStopStruct class not found");
        let stop_array = env
            .new_object_array(self.stops.len() as i32, &stop_class, JObject::null())
            .expect("Failed to create ResolvedGradientStopStruct array");

        for (index, stop) in self.stops.as_slice().iter().enumerate() {
            let java_stop = stop.to_java_struct(env);
            env.set_object_array_element(&stop_array, index as i32, &java_stop)
                .expect("Failed to set ResolvedGradientStopStruct array element");
        }

        let class = env
            .find_class("dev/waterui/android/runtime/ResolvedGradientStruct")
            .expect("ResolvedGradientStruct class not found");
        let object = env
            .new_object(
                &class,
                "(I[Ldev/waterui/android/runtime/ResolvedGradientStopStruct;FFFFFF)V",
                &[
                    JValue::Int(self.gradient_type as i32),
                    JValue::Object(&stop_array),
                    JValue::Float(self.start_x),
                    JValue::Float(self.start_y),
                    JValue::Float(self.end_x),
                    JValue::Float(self.end_y),
                    JValue::Float(self.start_value),
                    JValue::Float(self.end_value),
                ],
            )
            .expect("Failed to create ResolvedGradientStruct");
        unsafe { consume_ffi_array(&self.stops) };
        object
    }
}

/// WuiShapeKind -> ShapeKindStruct(tag, topLeft, topRight, bottomRight, bottomLeft)
impl ToJavaStruct for crate::shape::WuiShapeKind {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/ShapeKindStruct")
            .expect("ShapeKindStruct class not found");
        env.new_object(
            &class,
            "(IFFFF)V",
            &[
                JValue::Int(self.tag),
                JValue::Float(self.top_left),
                JValue::Float(self.top_right),
                JValue::Float(self.bottom_right),
                JValue::Float(self.bottom_left),
            ],
        )
        .expect("Failed to create ShapeKindStruct")
    }
}

/// WuiResolvedShape -> ResolvedShapeStruct(kind, commands, fill)
impl ToJavaStruct for crate::shape::WuiResolvedShape {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let command_class = env
            .find_class("dev/waterui/android/runtime/PathCommandStruct")
            .expect("PathCommandStruct class not found");
        let command_array = env
            .new_object_array(self.commands.len() as i32, &command_class, JObject::null())
            .expect("Failed to create PathCommandStruct array");

        for (index, cmd) in self.commands.as_slice().iter().enumerate() {
            let java_cmd = create_path_command_struct(env, cmd);
            env.set_object_array_element(&command_array, index as i32, &java_cmd)
                .expect("Failed to set PathCommandStruct array element");
        }

        let java_kind = self.kind.to_java_struct(env);
        let class = env
            .find_class("dev/waterui/android/runtime/ResolvedShapeStruct")
            .expect("ResolvedShapeStruct class not found");
        let object = env.new_object(
            &class,
            "(Ldev/waterui/android/runtime/ShapeKindStruct;[Ldev/waterui/android/runtime/PathCommandStruct;J)V",
            &[
                JValue::Object(&java_kind),
                JValue::Object(&command_array),
                JValue::Long(self.fill as jlong),
            ],
        )
        .expect("Failed to create ResolvedShapeStruct");
        unsafe { consume_ffi_array(&self.commands) };
        object
    }
}

/// WuiText -> TextStruct(contentPtr: Long, paragraphAlignmentPtr: Long)
impl ToJavaStruct for crate::components::text::WuiText {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/TextStruct")
            .expect("TextStruct class not found");
        env.new_object(
            &class,
            "(JJ)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Long(self.paragraph_alignment as jlong),
            ],
        )
        .expect("Failed to create TextStruct")
    }
}

/// WuiButton -> ButtonStruct(labelPtr: Long, actionPtr: Long, style: Int, accessibilityLabelPtr: Long, disabledPtr: Long)
///
/// `label` is now a `WuiLabel` struct; we project its `view` pointer as the
/// labelPtr and its `accessibility_label` pointer separately. Display mode is
/// not surfaced to Android yet — visual hide already happens in the Rust
/// `Label::body` rendering path, which produces an empty view for Hidden mode.
impl ToJavaStruct for crate::components::button::WuiButton {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/ButtonStruct")
            .expect("ButtonStruct class not found");
        env.new_object(
            &class,
            "(JJIJJ)V",
            &[
                JValue::Long(self.label.view as jlong),
                JValue::Long(self.action as jlong),
                JValue::Int(self.style as i32),
                JValue::Long(self.label.accessibility_label as jlong),
                JValue::Long(self.disabled as jlong),
            ],
        )
        .expect("Failed to create ButtonStruct")
    }
}

/// WuiTextField -> TextFieldStruct(labelPtr, accessibilityLabelPtr, valuePtr,
/// promptPtr, promptAlignmentPtr, keyboardType, selectionMenuItemsPtr)
impl ToJavaStruct for crate::components::form::WuiTextField {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/TextFieldStruct")
            .expect("TextFieldStruct class not found");
        env.new_object(
            &class,
            "(JJJJJIJ)V",
            &[
                JValue::Long(self.label.view as jlong),
                JValue::Long(self.label.accessibility_label as jlong),
                JValue::Long(self.value as jlong),
                JValue::Long(self.prompt.content as jlong),
                JValue::Long(self.prompt.paragraph_alignment as jlong),
                JValue::Int(self.keyboard as i32),
                JValue::Long(self.selection_menu as jlong),
            ],
        )
        .expect("Failed to create TextFieldStruct")
    }
}

/// WuiSecureField -> SecureFieldStruct(labelPtr, accessibilityLabelPtr, valuePtr)
impl ToJavaStruct for crate::components::form::WuiSecureField {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/SecureFieldStruct")
            .expect("SecureFieldStruct class not found");
        env.new_object(
            &class,
            "(JJJ)V",
            &[
                JValue::Long(self.label.view as jlong),
                JValue::Long(self.label.accessibility_label as jlong),
                JValue::Long(self.value as jlong),
            ],
        )
        .expect("Failed to create SecureFieldStruct")
    }
}

/// WuiToggle -> ToggleStruct(labelPtr, accessibilityLabelPtr, bindingPtr, style, disabledPtr)
impl ToJavaStruct for crate::components::form::WuiToggle {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/ToggleStruct")
            .expect("ToggleStruct class not found");
        env.new_object(
            &class,
            "(JJJIJ)V",
            &[
                JValue::Long(self.label.view as jlong),
                JValue::Long(self.label.accessibility_label as jlong),
                JValue::Long(self.toggle as jlong),
                JValue::Int(self.style as i32),
                JValue::Long(self.disabled as jlong),
            ],
        )
        .expect("Failed to create ToggleStruct")
    }
}

/// WuiSlider -> SliderStruct
impl ToJavaStruct for crate::components::form::WuiSlider {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/SliderStruct")
            .expect("SliderStruct class not found");
        env.new_object(
            &class,
            "(JJJJDDJJ)V",
            &[
                JValue::Long(self.label.view as jlong),
                JValue::Long(self.label.accessibility_label as jlong),
                JValue::Long(self.min_value_label as jlong),
                JValue::Long(self.max_value_label as jlong),
                JValue::Double(self.range.start),
                JValue::Double(self.range.end),
                JValue::Long(self.value as jlong),
                JValue::Long(self.disabled as jlong),
            ],
        )
        .expect("Failed to create SliderStruct")
    }
}

/// WuiStepper -> StepperStruct
impl ToJavaStruct for crate::components::form::WuiStepper {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/StepperStruct")
            .expect("StepperStruct class not found");
        env.new_object(
            &class,
            "(JJJJJII)V",
            &[
                JValue::Long(self.value as jlong),
                JValue::Long(self.step as jlong),
                JValue::Long(self.label.view as jlong),
                JValue::Long(self.label.accessibility_label as jlong),
                JValue::Long(self.value_formatter as jlong),
                JValue::Int(self.range.start),
                JValue::Int(self.range.end),
            ],
        )
        .expect("Failed to create StepperStruct")
    }
}

/// WuiColorPicker -> ColorPickerStruct
impl ToJavaStruct for crate::components::form::WuiColorPicker {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/ColorPickerStruct")
            .expect("ColorPickerStruct class not found");
        env.new_object(
            &class,
            "(JJJZZ)V",
            &[
                JValue::Long(self.label.view as jlong),
                JValue::Long(self.label.accessibility_label as jlong),
                JValue::Long(self.value as jlong),
                JValue::Bool(if self.support_alpha { 1 } else { 0 }),
                JValue::Bool(if self.support_hdr { 1 } else { 0 }),
            ],
        )
        .expect("Failed to create ColorPickerStruct")
    }
}

/// WuiPicker -> PickerStruct(itemsPtr, selectionPtr, style)
impl ToJavaStruct for crate::components::form::WuiPicker {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/PickerStruct")
            .expect("PickerStruct class not found");
        env.new_object(
            &class,
            "(JJI)V",
            &[
                JValue::Long(self.items as jlong),
                JValue::Long(self.selection as jlong),
                JValue::Int(self.style as i32),
            ],
        )
        .expect("Failed to create PickerStruct")
    }
}

impl ToJavaStruct for crate::components::form::WuiPickerItem {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/PickerItemStruct")
            .expect("PickerItemStruct class not found");
        env.new_object(
            &class,
            "(IJ)V",
            &[
                JValue::Int(self.tag.inner),
                JValue::Long(self.label as jlong),
            ],
        )
        .expect("Failed to create PickerItemStruct")
    }
}

/// WuiDatePicker -> DatePickerStruct
impl ToJavaStruct for crate::components::form::WuiDatePicker {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/DatePickerStruct")
            .expect("DatePickerStruct class not found");
        let date_time_class = env
            .find_class("dev/waterui/android/runtime/DateTimeStruct")
            .expect("DateTimeStruct class not found");
        let start = env
            .new_object(
                &date_time_class,
                "(IIIIII)V",
                &[
                    JValue::Int(self.range.start.year),
                    JValue::Int(self.range.start.month as i32),
                    JValue::Int(self.range.start.day as i32),
                    JValue::Int(self.range.start.hour as i32),
                    JValue::Int(self.range.start.minute as i32),
                    JValue::Int(self.range.start.second as i32),
                ],
            )
            .expect("Failed to create DateTimeStruct for range start");
        let end = env
            .new_object(
                &date_time_class,
                "(IIIIII)V",
                &[
                    JValue::Int(self.range.end.year),
                    JValue::Int(self.range.end.month as i32),
                    JValue::Int(self.range.end.day as i32),
                    JValue::Int(self.range.end.hour as i32),
                    JValue::Int(self.range.end.minute as i32),
                    JValue::Int(self.range.end.second as i32),
                ],
            )
            .expect("Failed to create DateTimeStruct for range end");
        env.new_object(
            &class,
            "(JJJLdev/waterui/android/runtime/DateTimeStruct;Ldev/waterui/android/runtime/DateTimeStruct;I)V",
            &[
                JValue::Long(self.label.view as jlong),
                JValue::Long(self.label.accessibility_label as jlong),
                JValue::Long(self.value as jlong),
                JValue::Object(&start),
                JValue::Object(&end),
                JValue::Int(self.ty as i32),
            ],
        )
        .expect("Failed to create DatePickerStruct")
    }
}

impl ToJavaStruct for crate::components::form::WuiMultiDatePicker {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MultiDatePickerStruct")
            .expect("MultiDatePickerStruct class not found");
        let date_class = env
            .find_class("dev/waterui/android/runtime/DateStruct")
            .expect("DateStruct class not found");
        let start = env
            .new_object(
                &date_class,
                "(III)V",
                &[
                    JValue::Int(self.range.start.year),
                    JValue::Int(self.range.start.month as i32),
                    JValue::Int(self.range.start.day as i32),
                ],
            )
            .expect("Failed to create DateStruct for range start");
        let end = env
            .new_object(
                &date_class,
                "(III)V",
                &[
                    JValue::Int(self.range.end.year),
                    JValue::Int(self.range.end.month as i32),
                    JValue::Int(self.range.end.day as i32),
                ],
            )
            .expect("Failed to create DateStruct for range end");
        env.new_object(
            &class,
            "(JJJLdev/waterui/android/runtime/DateStruct;Ldev/waterui/android/runtime/DateStruct;J)V",
            &[
                JValue::Long(self.label.view as jlong),
                JValue::Long(self.label.accessibility_label as jlong),
                JValue::Long(self.value as jlong),
                JValue::Object(&start),
                JValue::Object(&end),
                JValue::Long(self.decorated as jlong),
            ],
        )
        .expect("Failed to create MultiDatePickerStruct")
    }
}

/// WuiScrollView -> ScrollStruct(axis, contentPtr)
impl ToJavaStruct for crate::components::layout::WuiScrollView {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/ScrollStruct")
            .expect("ScrollStruct class not found");
        env.new_object(
            &class,
            "(IJ)V",
            &[
                JValue::Int(self.axis as i32),
                JValue::Long(self.content as jlong),
            ],
        )
        .expect("Failed to create ScrollStruct")
    }
}

/// WuiFixedContainer -> FixedContainerStruct(layoutPtr, childPointers: LongArray)
impl ToJavaStruct for crate::components::layout::WuiFixedContainer {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        // Create the long array for child pointers
        let children = self.contents.as_slice();
        let java_array = env
            .new_long_array(children.len() as i32)
            .expect("Failed to create long array");
        let child_ptrs: Vec<i64> = children.iter().map(|p| *p as jlong).collect();
        env.set_long_array_region(&java_array, 0, &child_ptrs)
            .expect("Failed to set long array region");

        let class = env
            .find_class("dev/waterui/android/runtime/FixedContainerStruct")
            .expect("FixedContainerStruct class not found");
        let object = env
            .new_object(
                &class,
                "(J[J)V",
                &[
                    JValue::Long(self.layout as jlong),
                    JValue::Object(&java_array),
                ],
            )
            .expect("Failed to create FixedContainerStruct");
        unsafe { consume_ffi_array(&self.contents) };
        object
    }
}

/// WuiContainer -> ContainerStruct(layoutPtr, contentsPtr)
impl ToJavaStruct for crate::components::layout::WuiContainer {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/ContainerStruct")
            .expect("ContainerStruct class not found");
        env.new_object(
            &class,
            "(JJ)V",
            &[
                JValue::Long(self.layout as jlong),
                JValue::Long(self.contents as jlong),
            ],
        )
        .expect("Failed to create ContainerStruct")
    }
}

/// WuiNavigationView -> NavigationViewStruct
impl ToJavaStruct for crate::components::navigation::WuiNavigationView {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let search = if self.bar.search.has_value {
            self.bar.search.value.to_java_struct(env)
        } else {
            JObject::null()
        };
        let toolbar_items = self.bar.toolbar.as_slice();
        let toolbar_item_class = env
            .find_class("dev/waterui/android/runtime/NavigationToolbarItemStruct")
            .expect("NavigationToolbarItemStruct class not found");
        let toolbar = env
            .new_object_array(
                toolbar_items.len() as i32,
                &toolbar_item_class,
                JObject::null(),
            )
            .expect("Failed to create NavigationToolbarItemStruct array");
        for (index, item) in toolbar_items.iter().enumerate() {
            let item = env
                .new_object(
                    &toolbar_item_class,
                    "(IJ)V",
                    &[
                        JValue::Int(item.placement as i32),
                        JValue::Long(item.content as jlong),
                    ],
                )
                .expect("Failed to create NavigationToolbarItemStruct");
            env.set_object_array_element(&toolbar, index as i32, item)
                .expect("Failed to set navigation toolbar item");
        }
        let bar_class = env
            .find_class("dev/waterui/android/runtime/BarStruct")
            .expect("BarStruct class not found");
        let bar = env
            .new_object(
                &bar_class,
                "(JJ[Ldev/waterui/android/runtime/NavigationToolbarItemStruct;Ldev/waterui/android/runtime/NavigationSearchStruct;JJI)V",
                &[
                    JValue::Long(self.bar.title as jlong),
                    JValue::Long(self.bar.subtitle as jlong),
                    JValue::Object(&toolbar),
                    JValue::Object(&search),
                    JValue::Long(self.bar.color as jlong),
                    JValue::Long(self.bar.hidden as jlong),
                    JValue::Int(self.bar.display_mode as i32),
                ],
            )
            .expect("Failed to create BarStruct");
        unsafe { consume_ffi_array(&self.bar.toolbar) };

        let class = env
            .find_class("dev/waterui/android/runtime/NavigationViewStruct")
            .expect("NavigationViewStruct class not found");
        env.new_object(
            &class,
            "(Ldev/waterui/android/runtime/BarStruct;JJJJJJ)V",
            &[
                JValue::Object(&bar),
                JValue::Long(self.content as jlong),
                JValue::Long(self.state.pop_enabled as jlong),
                JValue::Long(self.state.pop_attempted as jlong),
                JValue::Long(self.state.appear as jlong),
                JValue::Long(self.state.disappear as jlong),
                JValue::Long(self.state.pop as jlong),
            ],
        )
        .expect("Failed to create NavigationViewStruct")
    }
}

/// WuiNavigationSearch -> NavigationSearchStruct
impl ToJavaStruct for crate::components::navigation::WuiNavigationSearch {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/NavigationSearchStruct")
            .expect("NavigationSearchStruct class not found");
        env.new_object(
            &class,
            "(JJ)V",
            &[
                JValue::Long(self.text as jlong),
                JValue::Long(self.prompt as jlong),
            ],
        )
        .expect("Failed to create NavigationSearchStruct")
    }
}

/// WuiNavigationStack -> NavigationStackStruct(rootPtr, transition, transitionSourceId)
impl ToJavaStruct for crate::components::navigation::WuiNavigationStack {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/NavigationStackStruct")
            .expect("NavigationStackStruct class not found");
        env.new_object(
            &class,
            "(JII)V",
            &[
                JValue::Long(self.root as jlong),
                JValue::Int(self.transition.kind as i32),
                JValue::Int(self.transition.source_id),
            ],
        )
        .expect("Failed to create NavigationStackStruct")
    }
}

/// WuiNavigationSplitLayout -> SplitNavigationContainerStruct
impl ToJavaStruct for crate::components::navigation::WuiNavigationSplitLayout {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/SplitNavigationContainerStruct")
            .expect("SplitNavigationContainerStruct class not found");
        env.new_object(
            &class,
            "(JJJJJJJFFFI)V",
            &[
                JValue::Long(self.sidebar as jlong),
                JValue::Long(self.placeholder as jlong),
                JValue::Long(self.primary_selection as jlong),
                JValue::Long(self.content as jlong),
                JValue::Long(self.secondary_selection as jlong),
                JValue::Long(self.detail as jlong),
                JValue::Long(self.column_visibility as jlong),
                JValue::Float(self.sidebar_width.min),
                JValue::Float(self.sidebar_width.ideal),
                JValue::Float(self.sidebar_width.max),
                JValue::Int(self.style as i32),
            ],
        )
        .expect("Failed to create SplitNavigationContainerStruct")
    }
}

/// WuiTabs -> TabsStruct
impl ToJavaStruct for crate::components::navigation::WuiTabs {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        // Create array of TabStruct
        let tabs = self.tabs.as_slice();
        let tab_class = env
            .find_class("dev/waterui/android/runtime/TabStruct")
            .expect("TabStruct class not found");

        let java_array = env
            .new_object_array(tabs.len() as i32, &tab_class, JObject::null())
            .expect("Failed to create TabStruct array");

        for (i, tab) in tabs.iter().enumerate() {
            let tab_obj = env
                .new_object(
                    &tab_class,
                    "(JJJJJ)V",
                    &[
                        JValue::Long(tab.id as jlong),
                        JValue::Long(tab.label as jlong),
                        JValue::Long(tab.content as jlong),
                        JValue::Long(tab.badge as jlong),
                        JValue::Long(tab.enabled as jlong),
                    ],
                )
                .expect("Failed to create TabStruct");
            env.set_object_array_element(&java_array, i as i32, &tab_obj)
                .expect("Failed to set tab element");
        }

        let class = env
            .find_class("dev/waterui/android/runtime/TabsStruct")
            .expect("TabsStruct class not found");
        let object = env
            .new_object(
                &class,
                "(J[Ldev/waterui/android/runtime/TabStruct;I)V",
                &[
                    JValue::Long(self.selection as jlong),
                    JValue::Object(&java_array),
                    JValue::Int(self.style as i32),
                ],
            )
            .expect("Failed to create TabsStruct");
        unsafe { consume_ffi_array(&self.tabs) };
        object
    }
}

/// WuiList -> ListStruct
impl ToJavaStruct for crate::components::list::WuiList {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/ListStruct")
            .expect("ListStruct class not found");
        env.new_object(
            &class,
            "(JJJJ)V",
            &[
                JValue::Long(self.contents as jlong),
                JValue::Long(self.editing as jlong),
                JValue::Long(self.on_delete as jlong),
                JValue::Long(self.on_move as jlong),
            ],
        )
        .expect("Failed to create ListStruct")
    }
}

/// WuiListItem -> ListItemStruct
impl ToJavaStruct for crate::components::list::WuiListItem {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let section_label: waterui::Str =
            unsafe { crate::IntoRust::into_rust(core::ptr::read(&self.section_label)) };
        let section_footer: waterui::Str =
            unsafe { crate::IntoRust::into_rust(core::ptr::read(&self.section_footer)) };
        let section_label = if section_label.is_empty() {
            JObject::null()
        } else {
            env.new_string(section_label.as_str())
                .expect("Failed to create list section label")
                .into()
        };
        let section_footer = if section_footer.is_empty() {
            JObject::null()
        } else {
            env.new_string(section_footer.as_str())
                .expect("Failed to create list section footer")
                .into()
        };
        let class = env
            .find_class("dev/waterui/android/runtime/ListItemStruct")
            .expect("ListItemStruct class not found");
        env.new_object(
            &class,
            "(JJLjava/lang/String;Ljava/lang/String;)V",
            &[
                JValue::Long(self.content as jlong),
                JValue::Long(self.deletable as jlong),
                JValue::Object(&section_label),
                JValue::Object(&section_footer),
            ],
        )
        .expect("Failed to create ListItemStruct")
    }
}

/// WuiProgress -> ProgressStruct(labelPtr, valueLabelPtr, valuePtr, style, fourColor)
impl ToJavaStruct for crate::components::progress::WuiProgress {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/ProgressStruct")
            .expect("ProgressStruct class not found");
        env.new_object(
            &class,
            "(JJJIZ)V",
            &[
                JValue::Long(self.label as jlong),
                JValue::Long(self.value_label as jlong),
                JValue::Long(self.value as jlong),
                JValue::Int(self.style as i32),
                JValue::Bool(self.four_color.into()),
            ],
        )
        .expect("Failed to create ProgressStruct")
    }
}

/// WuiGpuSurface -> GpuSurfaceStruct(rendererPtr, HDR preference, PiP host)
impl ToJavaStruct for crate::components::gpu_surface::WuiGpuSurface {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let preference =
            unsafe { crate::components::gpu_surface::waterui_gpu_surface_hdr_preference(self) };
        let class = env
            .find_class("dev/waterui/android/runtime/GpuSurfaceStruct")
            .expect("GpuSurfaceStruct class not found");
        env.new_object(
            &class,
            "(JZZZJ)V",
            &[
                JValue::Long(self.surface as jlong),
                JValue::Bool(if preference.has_preference { 1 } else { 0 }),
                JValue::Bool(if preference.prefers_hdr { 1 } else { 0 }),
                JValue::Bool(if self.has_picture_in_picture_host_id {
                    1
                } else {
                    0
                }),
                JValue::Long(self.picture_in_picture_host_id as jlong),
            ],
        )
        .expect("Failed to create GpuSurfaceStruct")
    }
}

/// *mut WuiWebView -> WebViewStruct(webviewPtr)
impl ToJavaStruct for *mut crate::components::webview::WuiWebView {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/WebViewStruct")
            .expect("WebViewStruct class not found");
        env.new_object(&class, "(J)V", &[JValue::Long(*self as jlong)])
            .expect("Failed to create WebViewStruct")
    }
}

/// *mut WuiDynamic -> DynamicStruct(dynamicPtr)
impl ToJavaStruct for *mut crate::components::dynamic::WuiDynamic {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/DynamicStruct")
            .expect("DynamicStruct class not found");
        env.new_object(&class, "(J)V", &[JValue::Long(*self as jlong)])
            .expect("Failed to create DynamicStruct")
    }
}

/// WuiMenu -> MenuStruct(labelPtr, itemsPtr, accessibilityLabelPtr)
impl ToJavaStruct for crate::WuiMenu {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let class = env
            .find_class("dev/waterui/android/runtime/MenuStruct")
            .expect("MenuStruct class not found");
        env.new_object(
            &class,
            "(JJJ)V",
            &[
                JValue::Long(self.label as jlong),
                JValue::Long(self.items as jlong),
                JValue::Long(self.accessibility_label as jlong),
            ],
        )
        .expect("Failed to create MenuStruct")
    }
}

impl ToJavaStruct for crate::WuiMenuItem {
    fn to_java_struct<'local>(&self, env: &mut JNIEnv<'local>) -> JObject<'local> {
        let label_ptr = if self.label.is_null() {
            0
        } else {
            let label = unsafe { Box::from_raw(self.label) };
            if !label.paragraph_alignment.is_null() {
                unsafe { drop(Box::from_raw(label.paragraph_alignment)) };
            }
            label.content as jlong
        };

        assert!(
            self.icon.is_null(),
            "SystemIcon menu items are unsupported on Android; use a portable icon-set view outside native menus"
        );

        let (key_equivalent, command, shift, option, control) = if self.shortcut.is_null() {
            (JObject::null(), false, false, false, false)
        } else {
            let shortcut = unsafe { *Box::from_raw(self.shortcut) };
            let modifiers = shortcut.modifiers;
            let key: waterui::Str = unsafe { crate::IntoRust::into_rust(shortcut.key) };
            let key = env
                .new_string(key.as_str())
                .expect("Failed to create shortcut key string");
            (
                JObject::from(key),
                modifiers.command,
                modifiers.shift,
                modifiers.option,
                modifiers.control,
            )
        };

        let class = env
            .find_class("dev/waterui/android/runtime/MenuItemStruct")
            .expect("MenuItemStruct class not found");
        env.new_object(
            &class,
            "(IJJJJLjava/lang/String;ZZZZJ)V",
            &[
                JValue::Int(self.tag as jint),
                JValue::Long(label_ptr),
                JValue::Long(self.action as jlong),
                JValue::Long(self.disabled as jlong),
                JValue::Long(self.selected as jlong),
                JValue::Object(&key_equivalent),
                JValue::Bool(if command { 1 } else { 0 }),
                JValue::Bool(if shift { 1 } else { 0 }),
                JValue::Bool(if option { 1 } else { 0 }),
                JValue::Bool(if control { 1 } else { 0 }),
                JValue::Long(self.items as jlong),
            ],
        )
        .expect("Failed to create MenuItemStruct")
    }
}
