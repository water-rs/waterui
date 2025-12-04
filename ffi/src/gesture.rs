//! FFI bindings for gesture types.

use alloc::boxed::Box;
use crate::action::WuiAction;
use crate::IntoFFI;
use waterui::gesture::{Gesture, GestureObserver};

/// FFI-safe representation of a gesture type.
#[repr(C)]
pub enum WuiGesture {
    /// A tap gesture requiring a specific number of taps.
    Tap { count: u32 },
    /// A long-press gesture requiring a minimum duration.
    LongPress { duration: u32 },
    /// A drag gesture with minimum distance threshold.
    Drag { min_distance: f32 },
    /// A magnification (pinch) gesture with initial scale.
    Magnification { initial_scale: f32 },
    /// A rotation gesture with initial angle.
    Rotation { initial_angle: f32 },
    /// A sequential composition of two gestures.
    Then {
        /// The first gesture that must complete.
        first: *mut WuiGesture,
        /// The gesture that runs after the first completes.
        then: *mut WuiGesture,
    },
}

impl IntoFFI for Gesture {
    type FFI = WuiGesture;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Gesture::Tap(tap) => WuiGesture::Tap { count: tap.count },
            Gesture::LongPress(lp) => WuiGesture::LongPress {
                duration: lp.duration,
            },
            Gesture::Drag(drag) => WuiGesture::Drag {
                min_distance: drag.min_distance,
            },
            Gesture::Magnification(mag) => WuiGesture::Magnification {
                initial_scale: mag.initial_scale,
            },
            Gesture::Rotation(rot) => WuiGesture::Rotation {
                initial_angle: rot.initial_angle,
            },
            Gesture::Then(then) => {
                let first = Box::into_raw(Box::new(then.first().clone().into_ffi()));
                let then_gesture = Box::into_raw(Box::new(then.then().clone().into_ffi()));
                WuiGesture::Then {
                    first,
                    then: then_gesture,
                }
            }
            // Handle any future gesture variants
            _ => WuiGesture::Tap { count: 1 },
        }
    }
}

/// Drops a WuiGesture, recursively freeing any Then variants.
///
/// # Safety
///
/// The gesture pointer must be valid and properly initialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_gesture(gesture: *mut WuiGesture) {
    if gesture.is_null() {
        return;
    }
    unsafe {
        let gesture = Box::from_raw(gesture);
        if let WuiGesture::Then { first, then } = *gesture {
            waterui_drop_gesture(first);
            waterui_drop_gesture(then);
        }
    }
}

/// FFI-safe representation of a gesture observer.
#[repr(C)]
pub struct WuiGestureObserver {
    /// The gesture type to observe.
    pub gesture: WuiGesture,
    /// Pointer to the action handler.
    pub action: *mut WuiAction,
}

impl IntoFFI for GestureObserver {
    type FFI = WuiGestureObserver;
    fn into_ffi(self) -> Self::FFI {
        WuiGestureObserver {
            gesture: self.gesture.into_ffi(),
            action: self.action.into_ffi(),
        }
    }
}
