//! JNI glue for Android runtime helper classes (not WatcherJni).
//!
//! These functions are called from Kotlin to complete callbacks that originate
//! from Rust via environment services (media picker / media loader).

extern crate std;

use jni::JNIEnv;
use jni::objects::{JClass, JObject, JString};
use jni::sys::{jbyte, jint, jlong};

use crate::components::media::MediaLoadResult;

type PresentCallbackFn = unsafe extern "C" fn(*mut (), u32);
type LoadCallbackFn = unsafe extern "C" fn(*mut (), MediaLoadResult);

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_runtime_MediaPickerManager_nativeCompletePresentCallback<
    'local,
>(
    _env: JNIEnv<'local>,
    _this: JObject<'local>,
    callback_data: jlong,
    callback_fn: jlong,
    selected_id: jint,
) {
    if callback_fn == 0 {
        return;
    }

    let call: PresentCallbackFn =
        unsafe { core::mem::transmute::<usize, PresentCallbackFn>(callback_fn as usize) };

    unsafe {
        call(callback_data as *mut (), selected_id as u32);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_runtime_MediaLoader_nativeCompleteMediaLoad<
    'local,
>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    callback_data: jlong,
    callback_fn: jlong,
    image_url: JString<'local>,
    video_url: JObject<'local>,
    media_type: jbyte,
) {
    if callback_fn == 0 {
        return;
    }

    let call: LoadCallbackFn =
        unsafe { core::mem::transmute::<usize, LoadCallbackFn>(callback_fn as usize) };

    let image_url: std::string::String = env
        .get_string(&image_url)
        .expect("MediaLoader.nativeCompleteMediaLoad: imageUrl")
        .into();
    let image_bytes = image_url.into_bytes();

    let (video_ptr, video_len, _video_bytes) = if video_url.is_null() {
        (core::ptr::null(), 0usize, None)
    } else {
        let video_jstring = JString::from(video_url);
        let video_url: std::string::String = env
            .get_string(&video_jstring)
            .expect("MediaLoader.nativeCompleteMediaLoad: videoUrl")
            .into();
        let bytes = video_url.into_bytes();
        (bytes.as_ptr(), bytes.len(), Some(bytes))
    };

    // Keep the backing buffers alive for the duration of the callback.
    let result = MediaLoadResult {
        url_ptr: image_bytes.as_ptr(),
        url_len: image_bytes.len(),
        video_url_ptr: video_ptr,
        video_url_len: video_len,
        media_type: media_type as u8,
    };

    unsafe {
        call(callback_data as *mut (), result);
    }
}
