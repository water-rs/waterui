use alloc::boxed::Box;
use waterui_core::Metadata;

use crate::{IntoFFI, WuiAnyView, WuiEnv, action::WuiAction, ffi_enum, ffi_struct, ffi_view};

use core::ptr::null_mut;

use waterui::{
    Environment,
    gesture::{
        DragEvent, DragGesture, Gesture, GestureObserver, GesturePhase, GesturePoint,
        LongPressEvent, LongPressGesture, MagnificationEvent, MagnificationGesture,
        RotationGesture, TapEvent, TapGesture,
    },
};

/// Describes the kind of gesture attached to a metadata entry.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiGestureKind {
    Tap,
    LongPress,
    Drag,
    Magnification,
    Rotation,
    Then,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WuiTapGesture {
    pub count: u32,
}

ffi_struct!(TapGesture, WuiTapGesture, count);

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WuiLongPressGesture {
    pub duration: u32,
}

ffi_struct!(LongPressGesture, WuiLongPressGesture, duration);

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WuiDragGesture {
    pub min_distance: f32,
}

ffi_struct!(DragGesture, WuiDragGesture, min_distance);

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WuiMagnificationGesture {
    pub initial_scale: f32,
}

ffi_struct!(MagnificationGesture, WuiMagnificationGesture, initial_scale);

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WuiRotationGesture {
    pub initial_angle: f32,
}

ffi_struct!(RotationGesture, WuiRotationGesture, initial_angle);

#[repr(C)]
#[derive(Debug, PartialEq)]
pub struct WuiGesture {
    pub kind: WuiGestureKind,
    pub tap: WuiTapGesture,
    pub long_press: WuiLongPressGesture,
    pub drag: WuiDragGesture,
    pub magnification: WuiMagnificationGesture,
    pub rotation: WuiRotationGesture,
    pub first: *mut WuiGesture,
    pub then: *mut WuiGesture,
}

impl Default for WuiGesture {
    fn default() -> Self {
        Self {
            kind: WuiGestureKind::Tap,
            tap: WuiTapGesture::default(),
            long_press: WuiLongPressGesture::default(),
            drag: WuiDragGesture::default(),
            magnification: WuiMagnificationGesture::default(),
            rotation: WuiRotationGesture::default(),
            first: null_mut(),
            then: null_mut(),
        }
    }
}

fn gesture_to_ffi_struct(gesture: Gesture) -> WuiGesture {
    match gesture {
        Gesture::Tap(gesture) => WuiGesture {
            kind: WuiGestureKind::Tap,
            tap: gesture.into_ffi(),
            ..WuiGesture::default()
        },
        Gesture::LongPress(gesture) => WuiGesture {
            kind: WuiGestureKind::LongPress,
            long_press: gesture.into_ffi(),
            ..WuiGesture::default()
        },
        Gesture::Drag(gesture) => WuiGesture {
            kind: WuiGestureKind::Drag,
            drag: gesture.into_ffi(),
            ..WuiGesture::default()
        },
        Gesture::Magnification(gesture) => WuiGesture {
            kind: WuiGestureKind::Magnification,
            magnification: gesture.into_ffi(),
            ..WuiGesture::default()
        },
        Gesture::Rotation(gesture) => WuiGesture {
            kind: WuiGestureKind::Rotation,
            rotation: gesture.into_ffi(),
            ..WuiGesture::default()
        },
        Gesture::Then(sequence) => WuiGesture {
            kind: WuiGestureKind::Then,
            first: sequence.first().clone().into_ffi(),
            then: sequence.then().clone().into_ffi(),
            ..WuiGesture::default()
        },
        _ => unreachable!("Unsupported gesture type"),
    }
}

impl IntoFFI for Gesture {
    type FFI = *mut WuiGesture;

    fn into_ffi(self) -> Self::FFI {
        Box::into_raw(Box::new(gesture_to_ffi_struct(self)))
    }
}

/// Releases a gesture descriptor tree allocated by Rust.
///
/// # Safety
///
/// The pointer must either be null or point to a gesture obtained through
/// `waterui_force_as_gesture` or any conversion that returns a `WuiGesture` pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_drop_gesture(value: *mut WuiGesture) {
    if value.is_null() {
        return;
    }

    let mut boxed = unsafe { Box::from_raw(value) };
    let first = boxed.first;
    let then = boxed.then;
    boxed.first = null_mut();
    boxed.then = null_mut();
    drop(boxed);

    unsafe {
        waterui_drop_gesture(first);
        waterui_drop_gesture(then);
    }
}

#[repr(C)]
#[derive(Debug, PartialEq)]
pub struct WuiGestureObserverValue {
    pub gesture: *mut WuiGesture,
    pub action: *mut WuiAction,
}

ffi_struct!(GestureObserver, WuiGestureObserverValue, gesture, action);

#[repr(C)]
#[derive(Debug, PartialEq)]
pub struct WuiGestureMetadata {
    pub view: *mut WuiAnyView,
    pub gesture: *mut WuiGesture,
    pub action: *mut WuiAction,
}

impl IntoFFI for Metadata<GestureObserver> {
    type FFI = WuiGestureMetadata;

    fn into_ffi(self) -> Self::FFI {
        let observer = self.value.into_ffi();

        WuiGestureMetadata {
            view: self.content.into_ffi(),
            gesture: observer.gesture,
            action: observer.action,
        }
    }
}

ffi_view!(
    Metadata<GestureObserver>,
    WuiGestureMetadata,
    waterui_gesture_id,
    waterui_force_as_gesture
);

// FFI-safe representation of a gesture phase.
ffi_enum!(
    GesturePhase,
    WuiGesturePhase,
    Started,
    Updated,
    Ended,
    Cancelled
);

impl From<WuiGesturePhase> for GesturePhase {
    fn from(value: WuiGesturePhase) -> Self {
        match value {
            WuiGesturePhase::Started => GesturePhase::Started,
            WuiGesturePhase::Updated => GesturePhase::Updated,
            WuiGesturePhase::Ended => GesturePhase::Ended,
            WuiGesturePhase::Cancelled => GesturePhase::Cancelled,
        }
    }
}

/// FFI-safe two-dimensional point used in gesture payloads.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WuiGesturePoint {
    pub x: f32,
    pub y: f32,
}

impl From<WuiGesturePoint> for GesturePoint {
    fn from(value: WuiGesturePoint) -> Self {
        GesturePoint::new(value.x, value.y)
    }
}

/// Gesture event kind.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiGestureEventKind {
    Tap,
    LongPress,
    Drag,
    Magnification,
}

/// FFI-safe gesture event payload sent from the backend.
#[repr(C)]
#[derive(Debug)]
pub struct WuiGestureEvent {
    pub kind: WuiGestureEventKind,
    pub phase: WuiGesturePhase,
    pub location: WuiGesturePoint,
    pub translation: WuiGesturePoint,
    pub velocity: WuiGesturePoint,
    pub scale: f32,
    pub velocity_scalar: f32,
    pub count: u32,
    pub duration: f32,
}

impl WuiGestureEvent {
    fn write_to_env(self, env: &mut Environment) {
        match self.kind {
            WuiGestureEventKind::Tap => {
                env.insert(TapEvent {
                    location: self.location.into(),
                    count: self.count,
                });
            }
            WuiGestureEventKind::LongPress => {
                env.insert(LongPressEvent {
                    location: self.location.into(),
                    duration: self.duration,
                });
            }
            WuiGestureEventKind::Drag => {
                env.insert(DragEvent {
                    phase: self.phase.into(),
                    location: self.location.into(),
                    translation: self.translation.into(),
                    velocity: self.velocity.into(),
                });
            }
            WuiGestureEventKind::Magnification => {
                env.insert(MagnificationEvent {
                    phase: self.phase.into(),
                    center: self.location.into(),
                    scale: self.scale,
                    velocity: self.velocity_scalar,
                });
            }
        }
    }
}

/// Calls a gesture action with the provided event payload.
///
/// # Safety
///
/// * `action` must be a valid pointer to an existing `WuiAction`.
/// * `env` must be a valid pointer to an existing `WuiEnv`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_gesture_action(
    action: *mut WuiAction,
    env: *const WuiEnv,
    event: WuiGestureEvent,
) {
    if action.is_null() || env.is_null() {
        return;
    }

    let action = unsafe { &*action };
    let env = unsafe { &**env };
    let mut env = env.clone();
    event.write_to_env(&mut env);
    action.handle(&env);
}
