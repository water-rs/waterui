//! FFI bindings for gesture types.

use crate::IntoFFI;
use crate::action::WuiAction;
use alloc::boxed::Box;
use waterui::gesture::{Gesture, GestureObserver};

/// `WuiGesture.buttons` bit for the primary button.
pub const WATERUI_POINTER_BUTTON_PRIMARY: u8 = 1 << 0;
/// `WuiGesture.buttons` bit for the secondary button.
pub const WATERUI_POINTER_BUTTON_SECONDARY: u8 = 1 << 1;
/// `WuiGesture.buttons` bit for the middle button.
pub const WATERUI_POINTER_BUTTON_MIDDLE: u8 = 1 << 2;
/// `WuiGesture.buttons` bit for the "back" side button.
pub const WATERUI_POINTER_BUTTON_BACK: u8 = 1 << 3;
/// `WuiGesture.buttons` bit for the "forward" side button.
pub const WATERUI_POINTER_BUTTON_FORWARD: u8 = 1 << 4;

/// FFI-safe representation of a gesture type.
#[repr(C)]
#[derive(Debug)]
pub enum WuiGesture {
    /// A tap gesture requiring a specific number of taps.
    Tap {
        /// Number of taps required to recognize the gesture.
        count: u32,
        /// Mask of buttons that can tap (`WATERUI_POINTER_BUTTON_*`);
        /// `WATERUI_POINTER_BUTTON_PRIMARY` for a default gesture.
        buttons: u8,
    },
    /// A long-press gesture requiring a minimum duration.
    LongPress {
        /// Minimum press duration in milliseconds before the gesture fires.
        duration: u32,
        /// Mask of buttons that can press (`WATERUI_POINTER_BUTTON_*`).
        buttons: u8,
    },
    /// A drag gesture with minimum distance threshold.
    Drag {
        /// Minimum drag distance (in points) before the gesture fires.
        min_distance: f32,
        /// Mask of buttons that can drag (`WATERUI_POINTER_BUTTON_*`).
        buttons: u8,
    },
    /// A magnification (pinch) gesture with initial scale.
    Magnification {
        /// Scale factor the gesture starts recognizing from.
        initial_scale: f32,
    },
    /// A rotation gesture with initial angle.
    Rotation {
        /// Angle (in radians) the gesture starts recognizing from.
        initial_angle: f32,
    },
    /// A sequential composition of two gestures.
    Then {
        /// The first gesture that must complete.
        first: *mut Self,
        /// The gesture that runs after the first completes.
        then: *mut Self,
    },
    /// A parallel composition of two gestures.
    Simultaneous {
        /// The first gesture in the composition.
        first: *mut Self,
        /// The second gesture in the composition.
        second: *mut Self,
    },
    /// An exclusive composition where first has priority over second.
    Exclusive {
        /// The primary gesture.
        first: *mut Self,
        /// The fallback gesture.
        second: *mut Self,
    },
}

impl IntoFFI for Gesture {
    type FFI = WuiGesture;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Self::Tap(tap) => WuiGesture::Tap {
                count: tap.count,
                buttons: tap.buttons.bits(),
            },
            Self::LongPress(lp) => WuiGesture::LongPress {
                duration: lp.duration,
                buttons: lp.buttons.bits(),
            },
            Self::Drag(drag) => WuiGesture::Drag {
                min_distance: drag.min_distance,
                buttons: drag.buttons.bits(),
            },
            Self::Magnification(mag) => WuiGesture::Magnification {
                initial_scale: mag.initial_scale,
            },
            Self::Rotation(rot) => WuiGesture::Rotation {
                initial_angle: rot.initial_angle,
            },
            Self::Then(then) => {
                let first = Box::into_raw(Box::new(then.first().clone().into_ffi()));
                let then_gesture = Box::into_raw(Box::new(then.then().clone().into_ffi()));
                WuiGesture::Then {
                    first,
                    then: then_gesture,
                }
            }
            Self::Simultaneous(pair) => {
                let first = Box::into_raw(Box::new(pair.first().clone().into_ffi()));
                let second = Box::into_raw(Box::new(pair.second().clone().into_ffi()));
                WuiGesture::Simultaneous { first, second }
            }
            Self::Exclusive(pair) => {
                let first = Box::into_raw(Box::new(pair.first().clone().into_ffi()));
                let second = Box::into_raw(Box::new(pair.second().clone().into_ffi()));
                WuiGesture::Exclusive { first, second }
            }
            _ => panic!("Unsupported Gesture variant for FFI conversion"),
        }
    }
}

/// Drops a `WuiGesture`, recursively freeing any composite variants.
///
/// # Safety
///
/// The gesture pointer must be valid and properly initialized.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_gesture(gesture: *mut WuiGesture) {
    // SAFETY: the caller contract makes `gesture` an owning pointer from the matching
    // constructor, reclaimed once here.
    unsafe {
        let gesture = Box::from_raw(gesture);
        match *gesture {
            WuiGesture::Then { first, then } => {
                waterui_drop_gesture(first);
                waterui_drop_gesture(then);
            }
            WuiGesture::Simultaneous { first, second }
            | WuiGesture::Exclusive { first, second } => {
                waterui_drop_gesture(first);
                waterui_drop_gesture(second);
            }
            WuiGesture::Tap { .. }
            | WuiGesture::LongPress { .. }
            | WuiGesture::Drag { .. }
            | WuiGesture::Magnification { .. }
            | WuiGesture::Rotation { .. } => {}
        }
    }
}

/// FFI-safe representation of a gesture observer.
#[repr(C)]
#[derive(Debug)]
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
