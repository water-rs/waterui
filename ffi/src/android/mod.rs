use core::{mem, ptr};

use jni::{
    JNIEnv,
    objects::{JClass, JObject, JValue},
    sys::{jbyteArray, jint, jlong, jlongArray, jobject, jobjectArray},
};

use crate::{
    IntoFFI, IntoRust, WuiAnyView, WuiEnv, WuiTypeId, components,
    components::layout::{WuiChildMetadata, WuiProposalSize, WuiRect, WuiSize},
    reactive::WatcherStruct,
};

type Result<T> = core::result::Result<T, jni::errors::Error>;

#[inline]
fn ptr_to_jlong<T>(ptr: *mut T) -> jlong {
    ptr as isize as jlong
}

#[inline]
fn jlong_to_ptr<T>(value: jlong) -> *mut T {
    value as isize as *mut T
}

fn throw(env: &JNIEnv, msg: impl core::fmt::Display) {
    let _ = env.throw_new("java/lang/RuntimeException", msg.to_string());
}

fn type_id_to_java(env: &JNIEnv, type_id: WuiTypeId) -> jlongArray {
    let bits: [u64; 2] = unsafe { mem::transmute(type_id) };
    let arr = match env.new_long_array(2) {
        Ok(array) => array,
        Err(err) => {
            throw(env, err);
            return ptr::null_mut();
        }
    };

    let longs = [bits[0] as i64, bits[1] as i64];
    if let Err(err) = env.set_long_array_region(&arr, 0, &longs) {
        throw(env, err);
        return ptr::null_mut();
    }

    arr.into_raw()
}

fn new_object(env: &JNIEnv, class: &str, sig: &str, args: &[JValue<'_>]) -> jobject {
    let class = match env.find_class(class) {
        Ok(clazz) => clazz,
        Err(err) => {
            throw(env, format!("Class {} not found: {err}", class));
            return ptr::null_mut();
        }
    };

    let obj = env.new_object_unchecked(class, sig, args);
    match obj {
        Ok(object) => object.into_raw(),
        Err(err) => {
            throw(env, err);
            ptr::null_mut()
        }
    }
}

fn byte_slice_to_array(env: &JNIEnv, data: &[u8]) -> jbyteArray {
    match env.byte_array_from_slice(data) {
        Ok(array) => array.into_raw(),
        Err(err) => {
            throw(env, err);
            ptr::null_mut()
        }
    }
}

fn proposal_from_java(env: &JNIEnv, obj: JObject) -> Result<waterui_layout::ProposalSize> {
    let width = env
        .call_method(&obj, "getWidth", "()F", &[])?
        .f()
        .map_err(|err| jni::errors::Error::from(err))?;
    let height = env
        .call_method(&obj, "getHeight", "()F", &[])?
        .f()
        .map_err(|err| jni::errors::Error::from(err))?;
    Ok(waterui_layout::ProposalSize::new(
        if width.is_nan() { None } else { Some(width) },
        if height.is_nan() { None } else { Some(height) },
    ))
}

fn child_metadata_from_java(env: &JNIEnv, obj: JObject) -> Result<WuiChildMetadata> {
    let proposal_obj = env
        .call_method(
            &obj,
            "getProposal",
            "()Ldev/waterui/android/runtime/ProposalStruct;",
            &[],
        )?
        .l()?;
    let proposal = proposal_from_java(env, proposal_obj)?;
    let priority = env.call_method(&obj, "getPriority", "()I", &[])?.i()?;
    let stretch = env.call_method(&obj, "getStretch", "()Z", &[])?.z()?;
    Ok(waterui_layout::ChildMetadata::new(proposal, priority as u8, stretch).into_ffi())
}

fn proposal_to_java(env: &JNIEnv, proposal: WuiProposalSize) -> jobject {
    let proposal = unsafe { IntoRust::into_rust(proposal) };
    new_object(
        env,
        "dev/waterui/android/runtime/ProposalStruct",
        "(FF)V",
        &[
            JValue::Float(proposal.width).into(),
            JValue::Float(proposal.height).into(),
        ],
    )
}

fn size_to_java(env: &JNIEnv, size: WuiSize) -> jobject {
    let size = unsafe { IntoRust::into_rust(size) };
    new_object(
        env,
        "dev/waterui/android/runtime/SizeStruct",
        "(FF)V",
        &[
            JValue::Float(size.width).into(),
            JValue::Float(size.height).into(),
        ],
    )
}

fn rect_to_java(env: &JNIEnv, rect: WuiRect) -> jobject {
    let rect = unsafe { IntoRust::into_rust(rect) };
    let size = rect.size();
    new_object(
        env,
        "dev/waterui/android/runtime/RectStruct",
        "(FFFF)V",
        &[
            JValue::Float(rect.x()).into(),
            JValue::Float(rect.y()).into(),
            JValue::Float(size.width).into(),
            JValue::Float(size.height).into(),
        ],
    )
}

fn make_object_array(
    env: &JNIEnv,
    class: &str,
    values: impl IntoIterator<Item = jobject>,
) -> jobjectArray {
    let cls = match env.find_class(class) {
        Ok(cls) => cls,
        Err(err) => {
            throw(env, format!("Class {} not found: {err}", class));
            return ptr::null_mut();
        }
    };

    let values: Vec<jobject> = values.into_iter().collect();
    let array = match env.new_object_array(values.len() as jint, cls, JObject::null()) {
        Ok(arr) => arr,
        Err(err) => {
            throw(env, err);
            return ptr::null_mut();
        }
    };

    for (idx, value) in values.into_iter().enumerate() {
        if let Err(err) =
            env.set_object_array_element(&array, idx as jint, unsafe { JObject::from_raw(value) })
        {
            throw(env, err);
            return ptr::null_mut();
        }
    }

    array.into_raw()
}

extern "C" {
    fn waterui_init() -> *mut WuiEnv;
    fn waterui_main() -> *mut crate::WuiAnyView;
    fn waterui_main_reloadble() -> *mut crate::WuiAnyView;
}

#[no_mangle]
pub extern "system" fn Java_dev_waterui_android_runtime_NativeBindings_waterui_1init(
    _env: JNIEnv,
    _class: JClass,
) -> jlong {
    unsafe { ptr_to_jlong(waterui_init()) }
}

#[no_mangle]
pub extern "system" fn Java_dev_waterui_android_runtime_NativeBindings_waterui_1main(
    _env: JNIEnv,
    _class: JClass,
    _env_ptr: jlong,
) -> jlong {
    unsafe { ptr_to_jlong(waterui_main()) }
}

#[no_mangle]
pub extern "system" fn Java_dev_waterui_android_runtime_NativeBindings_waterui_1view_1id(
    env: JNIEnv,
    _class: JClass,
    any_view_ptr: jlong,
) -> jlongArray {
    let ptr = jlong_to_ptr::<WuiAnyView>(any_view_ptr);
    let type_id = unsafe { crate::waterui_view_id(ptr) };
    type_id_to_java(&env, type_id)
}

#[no_mangle]
pub extern "system" fn Java_dev_waterui_android_runtime_NativeBindings_waterui_1view_1body(
    _env: JNIEnv,
    _class: JClass,
    any_view_ptr: jlong,
    env_ptr: jlong,
) -> jlong {
    unsafe {
        ptr_to_jlong(crate::waterui_view_body(
            jlong_to_ptr::<WuiAnyView>(any_view_ptr),
            jlong_to_ptr::<crate::WuiEnv>(env_ptr),
        ))
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_waterui_android_runtime_NativeBindings_waterui_1env_1clone(
    _env: JNIEnv,
    _class: JClass,
    env_ptr: jlong,
) -> jlong {
    unsafe {
        ptr_to_jlong(crate::waterui_clone_env(
            jlong_to_ptr::<WuiEnv>(env_ptr) as *const WuiEnv
        ))
    }
}

#[no_mangle]
pub extern "system" fn Java_dev_waterui_android_runtime_NativeBindings_waterui_1env_1drop(
    _env: JNIEnv,
    _class: JClass,
    env_ptr: jlong,
) {
    unsafe { crate::waterui_drop_env(jlong_to_ptr::<WuiEnv>(env_ptr)) }
}

#[no_mangle]
pub extern "system" fn Java_dev_waterui_android_runtime_NativeBindings_waterui_1anyview_1drop(
    _env: JNIEnv,
    _class: JClass,
    view_ptr: jlong,
) {
    unsafe { crate::waterui_drop_any_view(jlong_to_ptr::<WuiAnyView>(view_ptr)) }
}

macro_rules! type_id_wrapper {
    ($java_name:ident, $ffi:path) => {
        #[no_mangle]
        pub extern "system" fn $java_name(env: JNIEnv, _class: JClass) -> jlongArray {
            type_id_to_java(&env, $ffi())
        }
    };
}

type_id_wrapper!(
    Java_dev_waterui_android_runtime_NativeBindings_waterui_1empty_1id,
    crate::components::waterui_empty_id
);
type_id_wrapper!(
    Java_dev_waterui_android_runtime_NativeBindings_waterui_1text_1id,
    crate::components::text::waterui_text_id
);
type_id_wrapper!(
    Java_dev_waterui_android_runtime_NativeBindings_waterui_1label_1id,
    crate::components::waterui_label_id
);
type_id_wrapper!(
    Java_dev_waterui_android_runtime_NativeBindings_waterui_1button_1id,
    crate::components::button::waterui_button_id
);
type_id_wrapper!(
    Java_dev_waterui_android_runtime_NativeBindings_waterui_1color_1id,
    crate::color::waterui_color_id
);
type_id_wrapper!(
    Java_dev_waterui_android_runtime_NativeBindings_waterui_1text_1field_1id,
    crate::components::form::waterui_text_field_id
);
type_id_wrapper!(
    Java_dev_waterui_android_runtime_NativeBindings_waterui_1stepper_1id,
    crate::components::form::waterui_stepper_id
);
type_id_wrapper!(
    Java_dev_waterui_android_runtime_NativeBindings_waterui_1progress_1id,
    crate::components::progress::waterui_progress_id
);
type_id_wrapper!(
    Java_dev_waterui_android_runtime_NativeBindings_waterui_1dynamic_1id,
    crate::components::dynamic::waterui_dynamic_id
);
type_id_wrapper!(
    Java_dev_waterui_android_runtime_NativeBindings_waterui_1scroll_1view_1id,
    crate::components::layout::waterui_scroll_view_id
);
type_id_wrapper!(
    Java_dev_waterui_android_runtime_NativeBindings_waterui_1spacer_1id,
    crate::components::layout::waterui_spacer_id
);
type_id_wrapper!(
    Java_dev_waterui_android_runtime_NativeBindings_waterui_1toggle_1id,
    crate::components::form::waterui_toggle_id
);
type_id_wrapper!(
    Java_dev_waterui_android_runtime_NativeBindings_waterui_1slider_1id,
    crate::components::form::waterui_slider_id
);
type_id_wrapper!(
    Java_dev_waterui_android_runtime_NativeBindings_waterui_1renderer_1view_1id,
    crate::components::graphics::waterui_renderer_view_id
);
