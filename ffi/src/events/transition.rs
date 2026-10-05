//! Structural transition metadata, native properties, and ghost clocks.

use core::time::Duration;

use waterui_core::layout::{LayoutDirection, Point, Rect, Size};
use waterui_core::transition::{
    TransitionFrame, TransitionGeometry, TransitionPhase, TransitionProperties, TransitionSpec,
    TransitionState,
};
use waterui_core::{Binding, SignalExt};

use crate::{IntoFFI, IntoRust, WuiAnyView, WuiMetadata, animation::WuiAnimation};

/// Owning immutable declaration, reusable for independently retiring ghosts.
pub struct WuiTransitionSpec(TransitionSpec);

impl core::fmt::Debug for WuiTransitionSpec {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

/// Owning frame-driven clock and its host-updated overlay geometry.
///
/// Destroy after the insertion or removal completes. The backend pushes the
/// retained slot's current window frame through `waterui_transition_set_frame`
/// after every placement, so a pixel effect's window overlay follows scroll
/// and ancestor movement on the same signal the core body consumed.
#[derive(Debug)]
pub struct WuiTransitionState {
    state: TransitionState,
    frame: Binding<TransitionFrame>,
}

/// Transition metadata: wrapped content and its owning declaration handle.
pub type WuiMetadataTransition = WuiMetadata<*mut WuiTransitionSpec>;

impl IntoFFI for TransitionSpec {
    type FFI = *mut WuiTransitionSpec;
    fn into_ffi(self) -> Self::FFI {
        Box::into_raw(Box::new(WuiTransitionSpec(self)))
    }
}

ffi_metadata!(TransitionSpec, WuiMetadataTransition, transition);

/// Window-relative geometry for the transition overlay.
///
/// `source_*` is the retained slot in window logical coordinates, `window_*`
/// the full overlay size in logical pixels, `scale_factor` device pixels per
/// logical pixel, and `right_to_left` selects the direction used to resolve
/// leading and trailing edges.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct WuiTransitionFrame {
    /// Retained slot origin x in window logical coordinates.
    pub source_x: f32,
    /// Retained slot origin y in window logical coordinates.
    pub source_y: f32,
    /// Retained slot width in logical pixels.
    pub source_width: f32,
    /// Retained slot height in logical pixels.
    pub source_height: f32,
    /// Window overlay width in logical pixels.
    pub window_width: f32,
    /// Window overlay height in logical pixels.
    pub window_height: f32,
    /// Device pixels per logical pixel.
    pub scale_factor: f32,
    /// Whether the layout direction is right-to-left.
    pub right_to_left: bool,
}

impl IntoRust for WuiTransitionFrame {
    type Rust = TransitionFrame;
    unsafe fn into_rust(self) -> Self::Rust {
        TransitionFrame {
            source: Rect::new(
                Point::new(self.source_x, self.source_y),
                Size::new(self.source_width, self.source_height),
            ),
            window: Size::new(self.window_width, self.window_height),
            scale_factor: self.scale_factor,
            direction: if self.right_to_left {
                LayoutDirection::RightToLeft
            } else {
                LayoutDirection::LeftToRight
            },
        }
    }
}

/// Native visual adjustments. Sizes and offsets use points/dp, rotation radians.
///
/// Compose these with the view's ordinary properties. Scale and rotation apply
/// about the slot centre before translation; none changes layout dimensions.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct WuiTransitionProperties {
    /// Multiplicative alpha.
    pub opacity: f32,
    /// Horizontal scale.
    pub scale_x: f32,
    /// Vertical scale.
    pub scale_y: f32,
    /// Horizontal translation in logical pixels.
    pub translation_x: f32,
    /// Vertical translation in logical pixels.
    pub translation_y: f32,
    /// Clockwise radians.
    pub rotation: f32,
    /// Gaussian radius in logical pixels.
    pub blur: f32,
}

impl IntoFFI for TransitionProperties {
    type FFI = WuiTransitionProperties;
    fn into_ffi(self) -> Self::FFI {
        WuiTransitionProperties {
            opacity: self.opacity,
            scale_x: self.scale[0],
            scale_y: self.scale[1],
            translation_x: self.translation[0],
            translation_y: self.translation[1],
            rotation: self.rotation,
            blur: self.blur,
        }
    }
}

/// Starts an independent transition after structural insertion or removal.
///
/// Before removal, detach all semantic membership, disable input and retain the
/// last layout dimensions as a fixed-size leaf. A pixel removal captures once;
/// its window overlay follows that leaf's current global frame, pushed through
/// `waterui_transition_set_frame` after every placement. Release the visual and
/// slot when `waterui_transition_advance` returns false.
///
/// # Safety
/// `spec` is a live declaration handle, borrowed on its owning UI thread.
/// `frame` is the slot's initial window-relative geometry, copied by value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_transition_start(
    spec: *const WuiTransitionSpec,
    removal: bool,
    reduce_motion: bool,
    frame: WuiTransitionFrame,
) -> *mut WuiTransitionState {
    // SAFETY: the caller holds this declaration alive on the owning UI thread.
    let spec = unsafe { &*spec };
    // SAFETY: `frame` is a plain data descriptor owned by this call.
    let frame = unsafe { frame.into_rust() };
    let phase = if removal {
        TransitionPhase::Removal
    } else {
        TransitionPhase::Insertion
    };
    Box::into_raw(Box::new(WuiTransitionState {
        state: spec.0.start(phase, reduce_motion),
        frame: Binding::container(frame),
    }))
}

/// Pushes the slot's current window-relative geometry into the transition.
///
/// Call after every placement — including scrolling and ancestor movement —
/// so the pixel overlay reads the live frame through the signal the effect
/// subscribed to at `waterui_transition_body`.
///
/// # Safety
/// `state` is a live borrowed state on its owning UI thread; `frame` is a plain
/// data descriptor copied by value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_transition_set_frame(
    state: *const WuiTransitionState,
    frame: WuiTransitionFrame,
) {
    // SAFETY: the state is live for the duration of the caller's borrow and
    // `frame` is a plain data descriptor owned by this call.
    unsafe { (*state).frame.set(frame.into_rust()) }
}

/// Advances from the host frame clock; false means the lifetime has ended.
///
/// # Safety
/// `state` is a live exclusively borrowed state on its owning UI thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_transition_advance(
    state: *mut WuiTransitionState,
    delta_ns: u64,
) -> bool {
    // SAFETY: exclusive access is guaranteed by the UI-thread caller contract.
    unsafe { (*state).state.advance(Duration::from_nanos(delta_ns)) }
}

/// Samples visual properties for the current untransformed slot geometry.
///
/// # Safety
/// `state` is a live borrowed state on its owning UI thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_transition_properties(
    state: *const WuiTransitionState,
    width: f32,
    height: f32,
    right_to_left: bool,
    absent_endpoint: bool,
) -> WuiTransitionProperties {
    // SAFETY: the state is live for the duration of the caller's borrow.
    let state = unsafe { &(*state).state };
    let geometry = TransitionGeometry {
        size: Size::new(width, height),
        direction: if right_to_left {
            LayoutDirection::RightToLeft
        } else {
            LayoutDirection::LeftToRight
        },
    };
    if absent_endpoint {
        state.absent_properties(geometry)
    } else {
        state.properties(geometry)
    }
    .into_ffi()
}

/// Returns the native animation definition for the resolved effect.
///
/// # Safety
/// `state` is a live borrowed state on its owning UI thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_transition_animation(
    state: *const WuiTransitionState,
) -> WuiAnimation {
    // SAFETY: the caller keeps the state alive for this read.
    unsafe { (*state).state.animation().clone().into_ffi() }
}

/// Whether this resolved phase uses the existing GPU capture/effect path.
///
/// # Safety
/// `state` is a live borrowed state on its owning UI thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_transition_captures_content(
    state: *const WuiTransitionState,
) -> bool {
    // SAFETY: the caller keeps the state alive for this read.
    unsafe { (*state).state.captures_content() }
}

/// Constructs the pixel effect once and transfers its view to the caller.
///
/// A backend with an already retained realization may pass an owning empty
/// view and feed its capture into the resulting `FilteredView`. The output shares
/// the state's progress signal and its frame signal, both updated by the host:
/// progress by the frame clock, geometry by `waterui_transition_set_frame`.
///
/// # Safety
/// `state` is a live borrowed state; `content` is an owning view handle consumed
/// exactly once. Both belong to the current UI thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_transition_body(
    state: *const WuiTransitionState,
    content: *mut WuiAnyView,
) -> *mut WuiAnyView {
    // SAFETY: content is transferred and state remains live throughout this call.
    unsafe {
        (*state)
            .state
            .body(content.into_rust(), (*state).frame.computed())
            .into_ffi()
    }
}

/// Releases one declaration; existing clocks remain independently owned.
///
/// # Safety
/// `spec` is an owning handle that is consumed exactly once on its UI thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_transition_drop_spec(spec: *mut WuiTransitionSpec) {
    // SAFETY: this call consumes the allocation returned by IntoFFI.
    drop(unsafe { Box::from_raw(spec) });
}

/// Releases an insertion clock or a completed ghost clock.
///
/// # Safety
/// `state` is an owning handle consumed exactly once on its UI thread. The
/// backend has stopped callbacks into it before this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_transition_drop_state(state: *mut WuiTransitionState) {
    // SAFETY: this call consumes the allocation created by transition_start.
    drop(unsafe { Box::from_raw(state) });
}
