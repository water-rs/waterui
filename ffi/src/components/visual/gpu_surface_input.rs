//! Backend-neutral input carrier for [`GpuSurface`](waterui_graphics::GpuSurface).
//!
//! [`waterui_gpu_surface_set_input`](super::gpu_surface::waterui_gpu_surface_set_input)
//! carries a pointer *snapshot*, which is all a chart or a shader needs. A GPU
//! view that draws interactive content — a browser engine, a terminal, a text
//! editor — needs the events themselves: keys with their W3C identity, the
//! modifier chord, committed text, an input-method composition session, scroll
//! with its unit, and focus. This module is that carrier, and it speaks the
//! same vocabulary as
//! [`SurfaceInputEvent`](waterui_graphics::SurfaceInputEvent) so no platform
//! keycode ever crosses the ABI.
//!
//! # Shape
//!
//! [`WuiSurfaceInputEvent`] is a *flat tagged struct*, not a C union: every
//! payload field is present in every event and only the ones its
//! [`WuiSurfaceInputEventKind`] names carry meaning. cbindgen, Swift and Kotlin
//! all bind that shape without a discriminated-union dance, and the struct is
//! small enough that the redundancy costs nothing.
//!
//! # Ownership
//!
//! The three [`WuiStr`] fields follow the usual convention: the caller hands
//! over ownership and this crate frees them. All three are consumed on every
//! call, whatever the kind, so a host builds an event with empty strings for
//! the fields its kind does not use — never with a zeroed struct, which has no
//! valid array vtable.
//!
//! # Thread affinity
//!
//! Like every other `WuiGpuSurfaceState` entry point, these run on the thread
//! that created the state. Delivery is synchronous: the event reaches
//! [`GpuView::input`](waterui_graphics::GpuView::input) before the call
//! returns. A view that needs the screen back asks for it through the
//! [`RedrawHandle`](waterui_graphics::RedrawHandle) it cloned from its
//! `GpuContext` during setup, which fires the host's installed redraw callback.

use waterui_core::layout::{Point as LayoutPoint, Rect as LayoutRect, Size as LayoutSize};
use waterui_graphics::input::{
    Code, Key, Modifiers, ScrollUnit, SurfaceInputEvent, SurfacePointerButton,
};

use super::gpu_surface::{WuiGpuSurfaceState, with_semantic_input};
use crate::components::layouting::layout::WuiRect;
use crate::{IntoFFI, IntoRust, WuiStr};

/// `Modifiers::SHIFT` — a shift key is held.
pub const WUI_SURFACE_MODIFIER_SHIFT: u32 = 0x200;
/// `Modifiers::CONTROL` — a control key is held.
pub const WUI_SURFACE_MODIFIER_CONTROL: u32 = 0x8;
/// `Modifiers::ALT` — an alt/option key is held.
pub const WUI_SURFACE_MODIFIER_ALT: u32 = 0x1;
/// `Modifiers::META` — a meta/command/Windows key is held.
pub const WUI_SURFACE_MODIFIER_META: u32 = 0x40;
/// `Modifiers::CAPS_LOCK` — caps lock is latched on.
pub const WUI_SURFACE_MODIFIER_CAPS_LOCK: u32 = 0x4;
/// `Modifiers::NUM_LOCK` — num lock is latched on.
pub const WUI_SURFACE_MODIFIER_NUM_LOCK: u32 = 0x80;

/// Every modifier bit this ABI carries.
///
/// The W3C model has more (`AltGraph`, `Fn`, `Symbol`, the scroll and symbol
/// locks); no host forwards them today and no GPU view reads them, so they are
/// rejected rather than silently dropped.
const SUPPORTED_MODIFIERS: u32 = WUI_SURFACE_MODIFIER_SHIFT
    | WUI_SURFACE_MODIFIER_CONTROL
    | WUI_SURFACE_MODIFIER_ALT
    | WUI_SURFACE_MODIFIER_META
    | WUI_SURFACE_MODIFIER_CAPS_LOCK
    | WUI_SURFACE_MODIFIER_NUM_LOCK;

/// Which event a [`WuiSurfaceInputEvent`] carries.
///
/// The tag decides which of the struct's payload fields are read; the rest are
/// ignored (their strings are still freed).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiSurfaceInputEventKind {
    /// Keyboard focus entered or left the surface. Reads `focused`.
    Focus,
    /// The modifier chord changed on its own. Reads `modifiers`.
    Modifiers,
    /// The pointer moved. Reads `x`, `y`.
    PointerMove,
    /// A pointer button changed state. Reads `pressed`, `button`, `x`, `y`.
    PointerButton,
    /// A scroll gesture. Reads `x`, `y`, `delta_x`, `delta_y`, `scroll_unit`,
    /// `finished`.
    Scroll,
    /// A key changed state. Reads `pressed`, `key`, `code`, `modifiers`,
    /// `repeat`.
    Key,
    /// Committed text to insert at the caret. Reads `text`.
    TextInput,
    /// An input-method composition session began. Reads nothing.
    CompositionStart,
    /// The pre-edit text changed. Reads `text`, `caret`.
    CompositionUpdate,
    /// The composition session ended and its text is inserted. Reads `text`.
    CompositionCommit,
    /// The composition session was abandoned. Reads nothing.
    CompositionCancel,
}

/// What one unit of a scroll delta means.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiScrollUnit {
    /// Deltas count lines of text — a discrete mouse wheel.
    Line,
    /// Deltas are logical pixels — a trackpad or a precise wheel.
    Pixel,
}

impl WuiScrollUnit {
    const fn into_rust(self) -> ScrollUnit {
        match self {
            Self::Line => ScrollUnit::Line,
            Self::Pixel => ScrollUnit::Pixel,
        }
    }
}

/// A pointer button, in the W3C UI Events vocabulary.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiSurfacePointerButton {
    /// The primary button — the left button on a right-handed mouse, a tap.
    Primary,
    /// The secondary button — the right button on a right-handed mouse.
    Secondary,
    /// The middle button, usually the wheel pressed down.
    Middle,
    /// The "back" side button.
    Back,
    /// The "forward" side button.
    Forward,
}

impl WuiSurfacePointerButton {
    const fn into_rust(self) -> SurfacePointerButton {
        match self {
            Self::Primary => SurfacePointerButton::Primary,
            Self::Secondary => SurfacePointerButton::Secondary,
            Self::Middle => SurfacePointerButton::Middle,
            Self::Back => SurfacePointerButton::Back,
            Self::Forward => SurfacePointerButton::Forward,
        }
    }
}

/// One input event on its way to a GPU view.
///
/// See the [module docs](self) for the flat-tagged shape and the string
/// ownership rules.
#[repr(C)]
#[derive(Debug)]
pub struct WuiSurfaceInputEvent {
    /// Which event this is, and therefore which fields below are read.
    pub kind: WuiSurfaceInputEventKind,
    /// `Focus`: whether the surface gained (`true`) or lost focus.
    pub focused: bool,
    /// `Modifiers`, `Key`: the modifier chord, as `WUI_SURFACE_MODIFIER_*` bits.
    pub modifiers: u32,
    /// `PointerMove`, `PointerButton`, `Scroll`: logical surface-local x.
    pub x: f64,
    /// `PointerMove`, `PointerButton`, `Scroll`: logical surface-local y.
    pub y: f64,
    /// `PointerButton`, `Key`: `true` for a press, `false` for a release.
    pub pressed: bool,
    /// `PointerButton`: which button changed state.
    pub button: WuiSurfacePointerButton,
    /// `Scroll`: horizontal delta, positive when the content should move left.
    pub delta_x: f64,
    /// `Scroll`: vertical delta, positive when the content should move up.
    pub delta_y: f64,
    /// `Scroll`: what one unit of the deltas means.
    pub scroll_unit: WuiScrollUnit,
    /// `Scroll`: `true` on the event that ends a continuous gesture.
    pub finished: bool,
    /// `Key`: the W3C `KeyboardEvent.key` name — `"a"`, `"Enter"`, `"ArrowUp"`.
    pub key: WuiStr,
    /// `Key`: the W3C `KeyboardEvent.code` name — `"KeyA"`, `"Enter"`.
    pub code: WuiStr,
    /// `TextInput`, `CompositionUpdate`, `CompositionCommit`: the text.
    pub text: WuiStr,
    /// `Key`: `true` when the platform generated this press by auto-repeat.
    pub repeat: bool,
    /// `CompositionUpdate`: caret byte offset within `text`, or `-1` for none.
    pub caret: i64,
}

/// Reads the modifier chord, rejecting bits this ABI does not carry.
fn modifiers_from_ffi(bits: u32) -> Modifiers {
    assert_eq!(
        bits & !SUPPORTED_MODIFIERS,
        0,
        "waterui_gpu_surface_send_input_event: unsupported modifier bits {:#x}",
        bits & !SUPPORTED_MODIFIERS
    );
    Modifiers::from_bits(bits).expect("supported modifier bits are a subset of `Modifiers`")
}

/// Reads a `CompositionUpdate` caret, where `-1` is "the platform reported none".
fn caret_from_ffi(caret: i64, text: &str) -> Option<usize> {
    if caret < 0 {
        assert_eq!(
            caret, -1,
            "waterui_gpu_surface_send_input_event: composition caret must be a byte offset or -1, got {caret}"
        );
        return None;
    }
    let caret = usize::try_from(caret)
        .expect("a non-negative i64 caret fits usize on every platform WaterUI targets");
    assert!(
        text.is_char_boundary(caret),
        "waterui_gpu_surface_send_input_event: composition caret {caret} is not a UTF-8 boundary of {text:?}"
    );
    Some(caret)
}

impl IntoRust for WuiSurfaceInputEvent {
    type Rust = SurfaceInputEvent;

    /// # Panics
    ///
    /// Panics when `key` or `code` is not a W3C UI Events name, when
    /// `modifiers` carries a bit outside `WUI_SURFACE_MODIFIER_*`, or when
    /// `caret` is neither `-1` nor a UTF-8 boundary of `text`. A host that
    /// invents its own names has a translation bug, and silently dropping the
    /// event would hide it.
    unsafe fn into_rust(self) -> Self::Rust {
        // Every string is owned by this call whatever the kind, so all three are
        // consumed up front: leaving one behind on a `PointerMove` would leak the
        // host's allocation once per event.
        let Self {
            kind,
            focused,
            modifiers,
            x,
            y,
            pressed,
            button,
            delta_x,
            delta_y,
            scroll_unit,
            finished,
            key,
            code,
            text,
            repeat,
            caret,
        } = self;
        // SAFETY: the caller contract hands over ownership of all three strings,
        // each built by the matching FFI constructor and consumed exactly once here.
        let (key, code, text) = unsafe { (key.into_rust(), code.into_rust(), text.into_rust()) };
        let position = kurbo::Point::new(x, y);

        match kind {
            WuiSurfaceInputEventKind::Focus => SurfaceInputEvent::Focus(focused),
            WuiSurfaceInputEventKind::Modifiers => {
                SurfaceInputEvent::Modifiers(modifiers_from_ffi(modifiers))
            }
            WuiSurfaceInputEventKind::PointerMove => SurfaceInputEvent::PointerMove { position },
            WuiSurfaceInputEventKind::PointerButton => SurfaceInputEvent::PointerButton {
                pressed,
                button: button.into_rust(),
                position,
            },
            WuiSurfaceInputEventKind::Scroll => SurfaceInputEvent::Scroll {
                position,
                delta_x,
                delta_y,
                unit: scroll_unit.into_rust(),
                finished,
            },
            WuiSurfaceInputEventKind::Key => SurfaceInputEvent::Key {
                pressed,
                key: key.parse::<Key>().unwrap_or_else(|_| {
                    panic!(
                        "waterui_gpu_surface_send_input_event: {key:?} is not a W3C KeyboardEvent.key name"
                    )
                }),
                code: code.parse::<Code>().unwrap_or_else(|_| {
                    panic!(
                        "waterui_gpu_surface_send_input_event: {code:?} is not a W3C KeyboardEvent.code name"
                    )
                }),
                modifiers: modifiers_from_ffi(modifiers),
                repeat,
            },
            WuiSurfaceInputEventKind::TextInput => SurfaceInputEvent::TextInput(text),
            WuiSurfaceInputEventKind::CompositionStart => SurfaceInputEvent::CompositionStart,
            WuiSurfaceInputEventKind::CompositionUpdate => SurfaceInputEvent::CompositionUpdate {
                caret: caret_from_ffi(caret, &text),
                text,
            },
            WuiSurfaceInputEventKind::CompositionCommit => {
                SurfaceInputEvent::CompositionCommit(text)
            }
            WuiSurfaceInputEventKind::CompositionCancel => SurfaceInputEvent::CompositionCancel,
        }
    }
}

/// Whether this GPU view handles its own keyboard, IME, pointer and scroll input.
///
/// Hosts ask once per registration and install a key/IME responder only for the
/// surfaces that answer `true`; a view that merely draws keeps every event with
/// the surrounding `WaterUI` widgets.
///
/// # Safety
///
/// `state` must be a valid pointer returned by
/// [`waterui_gpu_surface_create`](super::gpu_surface::waterui_gpu_surface_create).
#[unsafe(no_mangle)]
pub const unsafe extern "C" fn waterui_gpu_surface_wants_input_events(
    state: *const WuiGpuSurfaceState,
) -> bool {
    // SAFETY: the caller contract requires `state` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let state = unsafe { crate::borrow_ffi(state) };
    state.wants_input_events()
}

/// Delivers one input event to the GPU view.
///
/// The event is translated to [`SurfaceInputEvent`] and handed to
/// [`GpuView::input`](waterui_graphics::GpuView::input) before this returns.
/// All three of the event's strings are consumed, whatever its kind.
///
/// # Returns
///
/// Whether the event reached the view. `false` means this surface does not take
/// input, or its asynchronous renderer setup has not finished yet — in both
/// cases the host should fall through to its own handling of the event.
///
/// # Panics
///
/// Panics when the event's `key`/`code` are not W3C UI Events names, when
/// `modifiers` carries a bit outside `WUI_SURFACE_MODIFIER_*`, or when `caret`
/// is neither `-1` nor a UTF-8 boundary of `text` — see
/// [`WuiSurfaceInputEvent`]'s conversion.
///
/// # Safety
///
/// `state` must be a valid pointer returned by
/// [`waterui_gpu_surface_create`](super::gpu_surface::waterui_gpu_surface_create),
/// and every [`WuiStr`] in `event` must be an owning handle from the matching
/// FFI constructor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_send_input_event(
    state: *mut WuiGpuSurfaceState,
    event: WuiSurfaceInputEvent,
) -> bool {
    // SAFETY: the caller contract requires `state` to be a valid handle, alive and
    // not otherwise borrowed for this call; the exclusive borrow ends here.
    let state = unsafe { crate::borrow_ffi_mut(state) };
    // SAFETY: the caller contract hands over the event's strings; `into_rust`
    // consumes each exactly once.
    let event = unsafe { event.into_rust() };
    if !state.wants_input_events() {
        return false;
    }
    with_semantic_input(state, |gpu_surface| gpu_surface.input(&event)).is_some()
}

/// The GPU view's text caret, in logical surface-local coordinates.
///
/// Native text-input clients place the input-method candidate window with it:
/// `NSTextInputClient.firstRect(forCharacterRange:)` on macOS,
/// `InputConnection`/`updateCursorAnchorInfo` on Android.
///
/// # Returns
///
/// Whether a caret was written to `out`. `false` leaves `out` untouched and
/// means the view has no caret to place the panel against.
///
/// # Panics
///
/// Panics when `out` is null, or when the view reports a caret whose
/// coordinates the `f32` layout ABI cannot represent.
///
/// # Safety
///
/// `state` must be a valid pointer returned by
/// [`waterui_gpu_surface_create`](super::gpu_surface::waterui_gpu_surface_create),
/// and `out` must point to writable storage for one [`WuiRect`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_gpu_surface_ime_caret(
    state: *const WuiGpuSurfaceState,
    out: *mut WuiRect,
) -> bool {
    assert!(
        !out.is_null(),
        "waterui_gpu_surface_ime_caret: `out` must point to writable WuiRect storage"
    );
    // SAFETY: the caller contract requires `state` to be a valid handle that stays
    // alive for this call; it is only borrowed.
    let state = unsafe { crate::borrow_ffi(state) };
    let Some(caret) = state.ime_caret() else {
        return false;
    };
    assert!(
        caret.x0.is_finite()
            && caret.y0.is_finite()
            && caret.x1.is_finite()
            && caret.y1.is_finite()
            && caret
                .x0
                .abs()
                .max(caret.y0.abs())
                .max(caret.width())
                .max(caret.height())
                <= f64::from(f32::MAX),
        "waterui_gpu_surface_ime_caret: the view reported a caret the f32 layout ABI cannot carry: {caret:?}"
    );
    // WaterUI's layout ABI is `f32` throughout (`WuiPoint`, `WuiSize`); the GPU
    // view vocabulary is kurbo's `f64`. The assertion above rules out the
    // truncation that matters — an out-of-range magnitude — leaving only the
    // sub-pixel precision a caret rect measured in logical points never needs.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "narrowing to the f32 layout ABI, range-checked directly above"
    )]
    let rect = LayoutRect::new(
        LayoutPoint::new(caret.x0 as f32, caret.y0 as f32),
        LayoutSize::new(caret.width() as f32, caret.height() as f32),
    );
    // SAFETY: the caller contract requires `out` to point at writable storage for
    // one `WuiRect`, and the null case was rejected above.
    unsafe { out.write(rect.into_ffi()) };
    true
}

#[cfg(test)]
mod tests {
    use super::{
        WUI_SURFACE_MODIFIER_ALT, WUI_SURFACE_MODIFIER_CAPS_LOCK, WUI_SURFACE_MODIFIER_CONTROL,
        WUI_SURFACE_MODIFIER_META, WUI_SURFACE_MODIFIER_NUM_LOCK, WUI_SURFACE_MODIFIER_SHIFT,
        WuiScrollUnit, WuiSurfaceInputEvent, WuiSurfaceInputEventKind, WuiSurfacePointerButton,
    };
    use crate::{IntoFFI, IntoRust};
    use waterui_core::Str;
    use waterui_graphics::input::{
        Code, Key, Modifiers, NamedKey, ScrollUnit, SurfaceInputEvent, SurfacePointerButton,
    };

    /// Builds the event a host would send, with the fields its kind ignores at
    /// the neutral values a host fills in (and valid, ownable empty strings).
    fn event(kind: WuiSurfaceInputEventKind) -> WuiSurfaceInputEvent {
        WuiSurfaceInputEvent {
            kind,
            focused: false,
            modifiers: 0,
            x: 0.0,
            y: 0.0,
            pressed: false,
            button: WuiSurfacePointerButton::Primary,
            delta_x: 0.0,
            delta_y: 0.0,
            scroll_unit: WuiScrollUnit::Pixel,
            finished: false,
            key: Str::default().into_ffi(),
            code: Str::default().into_ffi(),
            text: Str::default().into_ffi(),
            repeat: false,
            caret: -1,
        }
    }

    fn into_rust(event: WuiSurfaceInputEvent) -> SurfaceInputEvent {
        // SAFETY: every string in an event from `event()` is a fresh owning handle.
        unsafe { event.into_rust() }
    }

    #[test]
    fn focus_round_trips() {
        let mut ffi = event(WuiSurfaceInputEventKind::Focus);
        ffi.focused = true;
        assert_eq!(into_rust(ffi), SurfaceInputEvent::Focus(true));
        let ffi = event(WuiSurfaceInputEventKind::Focus);
        assert_eq!(into_rust(ffi), SurfaceInputEvent::Focus(false));
    }

    #[test]
    fn every_modifier_bit_maps_to_its_w3c_flag() {
        for (bits, expected) in [
            (WUI_SURFACE_MODIFIER_SHIFT, Modifiers::SHIFT),
            (WUI_SURFACE_MODIFIER_CONTROL, Modifiers::CONTROL),
            (WUI_SURFACE_MODIFIER_ALT, Modifiers::ALT),
            (WUI_SURFACE_MODIFIER_META, Modifiers::META),
            (WUI_SURFACE_MODIFIER_CAPS_LOCK, Modifiers::CAPS_LOCK),
            (WUI_SURFACE_MODIFIER_NUM_LOCK, Modifiers::NUM_LOCK),
        ] {
            let mut ffi = event(WuiSurfaceInputEventKind::Modifiers);
            ffi.modifiers = bits;
            assert_eq!(into_rust(ffi), SurfaceInputEvent::Modifiers(expected));
        }
    }

    #[test]
    fn a_modifier_chord_round_trips_as_one_value() {
        let mut ffi = event(WuiSurfaceInputEventKind::Modifiers);
        ffi.modifiers = WUI_SURFACE_MODIFIER_SHIFT | WUI_SURFACE_MODIFIER_META;
        assert_eq!(
            into_rust(ffi),
            SurfaceInputEvent::Modifiers(Modifiers::SHIFT | Modifiers::META)
        );
    }

    #[test]
    #[should_panic(expected = "unsupported modifier bits")]
    fn a_modifier_bit_this_abi_does_not_carry_fails_fast() {
        let mut ffi = event(WuiSurfaceInputEventKind::Modifiers);
        ffi.modifiers = Modifiers::SCROLL_LOCK.bits();
        let _ = into_rust(ffi);
    }

    #[test]
    fn pointer_move_round_trips() {
        let mut ffi = event(WuiSurfaceInputEventKind::PointerMove);
        ffi.x = 12.5;
        ffi.y = -3.25;
        assert_eq!(
            into_rust(ffi),
            SurfaceInputEvent::PointerMove {
                position: kurbo::Point::new(12.5, -3.25),
            }
        );
    }

    #[test]
    fn every_pointer_button_round_trips() {
        for (ffi_button, expected) in [
            (
                WuiSurfacePointerButton::Primary,
                SurfacePointerButton::Primary,
            ),
            (
                WuiSurfacePointerButton::Secondary,
                SurfacePointerButton::Secondary,
            ),
            (
                WuiSurfacePointerButton::Middle,
                SurfacePointerButton::Middle,
            ),
            (WuiSurfacePointerButton::Back, SurfacePointerButton::Back),
            (
                WuiSurfacePointerButton::Forward,
                SurfacePointerButton::Forward,
            ),
        ] {
            let mut ffi = event(WuiSurfaceInputEventKind::PointerButton);
            ffi.pressed = true;
            ffi.button = ffi_button;
            ffi.x = 4.0;
            ffi.y = 8.0;
            assert_eq!(
                into_rust(ffi),
                SurfaceInputEvent::PointerButton {
                    pressed: true,
                    button: expected,
                    position: kurbo::Point::new(4.0, 8.0),
                }
            );
        }
    }

    #[test]
    fn scroll_round_trips_with_its_unit_and_finished_flag() {
        for (ffi_unit, expected) in [
            (WuiScrollUnit::Line, ScrollUnit::Line),
            (WuiScrollUnit::Pixel, ScrollUnit::Pixel),
        ] {
            let mut ffi = event(WuiSurfaceInputEventKind::Scroll);
            ffi.x = 1.0;
            ffi.y = 2.0;
            ffi.delta_x = -10.0;
            ffi.delta_y = 40.0;
            ffi.scroll_unit = ffi_unit;
            ffi.finished = true;
            assert_eq!(
                into_rust(ffi),
                SurfaceInputEvent::Scroll {
                    position: kurbo::Point::new(1.0, 2.0),
                    delta_x: -10.0,
                    delta_y: 40.0,
                    unit: expected,
                    finished: true,
                }
            );
        }
    }

    #[test]
    fn a_character_key_round_trips_with_its_chord_and_repeat() {
        let mut ffi = event(WuiSurfaceInputEventKind::Key);
        ffi.pressed = true;
        ffi.repeat = true;
        ffi.key = Str::from_static("a").into_ffi();
        ffi.code = Str::from_static("KeyA").into_ffi();
        ffi.modifiers = WUI_SURFACE_MODIFIER_CONTROL;
        assert_eq!(
            into_rust(ffi),
            SurfaceInputEvent::Key {
                pressed: true,
                key: Key::Character("a".to_owned()),
                code: Code::KeyA,
                modifiers: Modifiers::CONTROL,
                repeat: true,
            }
        );
    }

    #[test]
    fn a_named_key_round_trips() {
        let mut ffi = event(WuiSurfaceInputEventKind::Key);
        ffi.key = Str::from_static("ArrowLeft").into_ffi();
        ffi.code = Str::from_static("ArrowLeft").into_ffi();
        assert_eq!(
            into_rust(ffi),
            SurfaceInputEvent::Key {
                pressed: false,
                key: Key::Named(NamedKey::ArrowLeft),
                code: Code::ArrowLeft,
                modifiers: Modifiers::empty(),
                repeat: false,
            }
        );
    }

    #[test]
    #[should_panic(expected = "is not a W3C KeyboardEvent.key name")]
    fn a_platform_key_name_fails_fast() {
        let mut ffi = event(WuiSurfaceInputEventKind::Key);
        // AppKit's own spelling, not the W3C one.
        ffi.key = Str::from_static("LeftArrow").into_ffi();
        ffi.code = Str::from_static("ArrowLeft").into_ffi();
        let _ = into_rust(ffi);
    }

    #[test]
    #[should_panic(expected = "is not a W3C KeyboardEvent.code name")]
    fn a_platform_code_name_fails_fast() {
        let mut ffi = event(WuiSurfaceInputEventKind::Key);
        ffi.key = Str::from_static("a").into_ffi();
        // A raw platform scancode is exactly what must not cross this ABI.
        ffi.code = Str::from_static("0").into_ffi();
        let _ = into_rust(ffi);
    }

    #[test]
    fn text_input_round_trips() {
        let mut ffi = event(WuiSurfaceInputEventKind::TextInput);
        ffi.text = Str::from_static("日本語").into_ffi();
        assert_eq!(
            into_rust(ffi),
            SurfaceInputEvent::TextInput(Str::from_static("日本語"))
        );
    }

    #[test]
    fn composition_start_and_cancel_round_trip() {
        assert_eq!(
            into_rust(event(WuiSurfaceInputEventKind::CompositionStart)),
            SurfaceInputEvent::CompositionStart
        );
        assert_eq!(
            into_rust(event(WuiSurfaceInputEventKind::CompositionCancel)),
            SurfaceInputEvent::CompositionCancel
        );
    }

    #[test]
    fn a_composition_update_carries_its_caret_or_none() {
        let mut ffi = event(WuiSurfaceInputEventKind::CompositionUpdate);
        ffi.text = Str::from_static("にほん").into_ffi();
        ffi.caret = 6;
        assert_eq!(
            into_rust(ffi),
            SurfaceInputEvent::CompositionUpdate {
                text: Str::from_static("にほん"),
                caret: Some(6),
            }
        );

        let mut ffi = event(WuiSurfaceInputEventKind::CompositionUpdate);
        ffi.text = Str::from_static("にほん").into_ffi();
        assert_eq!(
            into_rust(ffi),
            SurfaceInputEvent::CompositionUpdate {
                text: Str::from_static("にほん"),
                caret: None,
            }
        );
    }

    #[test]
    #[should_panic(expected = "is not a UTF-8 boundary")]
    fn a_caret_inside_a_character_fails_fast() {
        let mut ffi = event(WuiSurfaceInputEventKind::CompositionUpdate);
        ffi.text = Str::from_static("にほん").into_ffi();
        ffi.caret = 1;
        let _ = into_rust(ffi);
    }

    #[test]
    #[should_panic(expected = "must be a byte offset or -1")]
    fn a_negative_caret_that_is_not_the_absent_sentinel_fails_fast() {
        let mut ffi = event(WuiSurfaceInputEventKind::CompositionUpdate);
        ffi.caret = -2;
        let _ = into_rust(ffi);
    }

    #[test]
    fn composition_commit_round_trips() {
        let mut ffi = event(WuiSurfaceInputEventKind::CompositionCommit);
        ffi.text = Str::from_static("日本").into_ffi();
        assert_eq!(
            into_rust(ffi),
            SurfaceInputEvent::CompositionCommit(Str::from_static("日本"))
        );
    }
}
