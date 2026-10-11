//! JNI entrypoints for anchored overlays: the `PopupWindow` placement
//! contract the Android runtime calls instead of re-implementing it.

use jni::EnvUnowned;
use jni::objects::{JClass, JValue};
use jni::sys::{jboolean, jfloat, jint, jlong, jobject};
use jni::{jni_sig, jni_str};
use waterui_core::layout::{LayoutDirection, Point, Rect, Size};

use crate::WuiEnv;
use crate::jni::convert::jlong_to_ptr;
use crate::jni::with_env;
use waterui::metadata::anchored_overlay::{AnchorEdge, AnchorPlacement, Clamp, EdgeAlignment};

/// JNI: computes an anchored overlay's frame in window space — the shared
/// placement contract — for the Android runtime's `PopupWindow`.
///
/// `(envPtr: Long, anchorX: Float, anchorY: Float, anchorW: Float,
/// anchorH: Float, windowW: Float, windowH: Float, overlayW: Float,
/// overlayH: Float, edge: Int, alignment: Int, gap: Float, flip: Boolean,
/// clampTag: Int, clampMargin: Float)` → `AnchoredOverlayPlacementStruct`.
///
/// `envPtr` is the `WuiEnv` pointer the renderer holds; `0` computes with the
/// default left-to-right layout direction.
///
/// # Safety
///
/// `env_ptr` must be `0` or a valid `WuiEnv` pointer; the geometry floats
/// must be finite.
///
/// # Panics
///
/// Panics when the `AnchoredOverlayPlacementStruct` class or constructor is
/// missing from the runtime dex.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_anchoredOverlayPlace<
    'local,
>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
    env_ptr: jlong,
    anchor_x: jfloat,
    anchor_y: jfloat,
    anchor_w: jfloat,
    anchor_h: jfloat,
    window_w: jfloat,
    window_h: jfloat,
    overlay_w: jfloat,
    overlay_h: jfloat,
    edge: jint,
    alignment: jint,
    gap: jfloat,
    flip: jboolean,
    clamp_tag: jint,
    clamp_margin: jfloat,
) -> jobject {
    with_env(&mut env, |env| {
        let placement = AnchorPlacement {
            edge: jint_to_anchor_edge(edge),
            alignment: jint_to_edge_alignment(alignment),
            gap,
            flip,
            clamp: if clamp_tag == 1 {
                Clamp::Window {
                    margin: clamp_margin,
                }
            } else {
                Clamp::Off
            },
        };
        // SAFETY: the caller contract makes `env_ptr` either 0 or a valid
        // `WuiEnv` pointer; `LeftToRight` is only a fallback for a null env
        // (the environment's own direction wins when it carries one).
        let (placed, logical_edge) = unsafe {
            crate::anchored_overlay_place(
                Rect::new(
                    Point::new(anchor_x, anchor_y),
                    Size::new(anchor_w, anchor_h),
                ),
                Rect::from_size(Size::new(window_w, window_h)),
                Size::new(overlay_w, overlay_h),
                placement,
                LayoutDirection::LeftToRight,
                jlong_to_ptr::<WuiEnv>(env_ptr),
            )
        };
        let class = env
            .find_class(jni_str!(
                "dev/waterui/android/runtime/AnchoredOverlayPlacementStruct"
            ))
            .expect("AnchoredOverlayPlacementStruct class not found");
        env.new_object(
            &class,
            jni_sig!("(FFFFII)V"),
            &[
                JValue::Float(placed.frame.x()),
                JValue::Float(placed.frame.y()),
                JValue::Float(placed.frame.width()),
                JValue::Float(placed.frame.height()),
                JValue::Int(placed.edge as jint),
                JValue::Int(crate::IntoFFI::into_ffi(logical_edge) as jint),
            ],
        )
        .expect("Failed to create AnchoredOverlayPlacementStruct")
        .into_raw()
    })
}

const fn jint_to_anchor_edge(edge: jint) -> AnchorEdge {
    match edge {
        1 => AnchorEdge::Bottom,
        2 => AnchorEdge::Leading,
        3 => AnchorEdge::Trailing,
        _ => AnchorEdge::Top,
    }
}

const fn jint_to_edge_alignment(alignment: jint) -> EdgeAlignment {
    match alignment {
        0 => EdgeAlignment::Start,
        2 => EdgeAlignment::End,
        _ => EdgeAlignment::Center,
    }
}
