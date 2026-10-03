//! FFI bindings for key bubbling: `OnKeyPress` metadata and `KeyPress`.
//!
//! A view's `.on_key_press` handler sees keys the focused view left
//! unconsumed. The press crosses the ABI as [`WuiKeyPress`] — the W3C
//! `KeyboardEvent.key` and `KeyboardEvent.code` names plus the modifier chord,
//! the same vocabulary `waterui_gpu_content_send_input_event` already speaks —
//! and the native side answers [`WuiKeyHandling`], deciding whether the key
//! keeps bubbling to the next ancestor.

use crate::bridge::closure::RetainedCallback;
use crate::{IntoFFI, IntoRust, WuiStr};
use waterui_core::key::{Code, Key, KeyHandling, KeyPress, Modifiers, OnKeyPress};

/// Whether a key handler consumed a key, as `KeyHandling` crosses the ABI.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiKeyHandling {
    /// The key was consumed; bubbling stops here.
    Handled,
    /// The key was not used; it keeps bubbling to the next ancestor.
    Ignored,
}

impl IntoFFI for KeyHandling {
    type FFI = WuiKeyHandling;
    fn into_ffi(self) -> Self::FFI {
        match self {
            Self::Handled => WuiKeyHandling::Handled,
            Self::Ignored => WuiKeyHandling::Ignored,
        }
    }
}

/// Modifier bit for the Shift key in `WuiKeyPress::modifiers`.
pub const WUI_KEY_MODIFIER_SHIFT: u32 = 0x200;
/// Modifier bit for the Control key in `WuiKeyPress::modifiers`.
pub const WUI_KEY_MODIFIER_CONTROL: u32 = 0x8;
/// Modifier bit for the Alt/Option key in `WuiKeyPress::modifiers`.
pub const WUI_KEY_MODIFIER_ALT: u32 = 0x1;
/// Modifier bit for the Meta/Command key in `WuiKeyPress::modifiers`.
pub const WUI_KEY_MODIFIER_META: u32 = 0x40;
/// Modifier bit for the Caps Lock state in `WuiKeyPress::modifiers`.
pub const WUI_KEY_MODIFIER_CAPS_LOCK: u32 = 0x4;
/// Modifier bit for the Num Lock state in `WuiKeyPress::modifiers`.
pub const WUI_KEY_MODIFIER_NUM_LOCK: u32 = 0x80;

const SUPPORTED_MODIFIERS: u32 = WUI_KEY_MODIFIER_SHIFT
    | WUI_KEY_MODIFIER_CONTROL
    | WUI_KEY_MODIFIER_ALT
    | WUI_KEY_MODIFIER_META
    | WUI_KEY_MODIFIER_CAPS_LOCK
    | WUI_KEY_MODIFIER_NUM_LOCK;

/// Converts the `WUI_KEY_MODIFIER_*` / `WUI_SURFACE_MODIFIER_*` bit set into
/// `Modifiers`. Both ABIs carry the same bits — `Modifiers`' own bit values —
/// so the `gpu` feature's input path shares this ungated conversion.
///
/// # Panics
///
/// Panics when `bits` carries a bit outside the supported set — a host
/// passing unknown bits has a translation bug.
pub fn modifiers_from_ffi(bits: u32) -> Modifiers {
    assert_eq!(
        bits & !SUPPORTED_MODIFIERS,
        0,
        "unsupported modifier bits {:#x}",
        bits & !SUPPORTED_MODIFIERS
    );
    Modifiers::from_bits(bits).expect("supported modifier bits are a subset of `Modifiers`")
}

/// A key press handed to a native `OnKeyPress` handler.
///
/// `key` and `code` are owned strings: the caller hands over ownership and
/// this crate frees them during [`waterui_call_on_key_press`]. A host builds
/// them through its usual `WuiStr` constructor — never a zeroed struct, which
/// has no valid array vtable.
#[repr(C)]
#[derive(Debug)]
pub struct WuiKeyPress {
    /// The W3C `KeyboardEvent.key` name — `"a"`, `"Enter"`, `"Escape"`.
    pub key: WuiStr,
    /// The W3C `KeyboardEvent.code` name — `"KeyA"`, `"Escape"`.
    pub code: WuiStr,
    /// The modifier chord, as `WUI_KEY_MODIFIER_*` bits.
    pub modifiers: u32,
    /// Whether the platform generated this press by auto-repeat.
    pub repeat: bool,
}

impl IntoRust for WuiKeyPress {
    type Rust = KeyPress;

    /// # Panics
    ///
    /// Panics when `key` or `code` is not a W3C UI Events name, or when
    /// `modifiers` carries a bit outside `WUI_KEY_MODIFIER_*` — a host
    /// inventing its own names has a translation bug, and silently dropping
    /// the press would hide it.
    unsafe fn into_rust(self) -> Self::Rust {
        // SAFETY: the caller contract hands over ownership of both strings,
        // each built by the matching FFI constructor and consumed exactly once.
        let (key, code) = unsafe { (self.key.into_rust(), self.code.into_rust()) };
        KeyPress {
            key: key.parse::<Key>().unwrap_or_else(|_| {
                panic!("waterui_call_on_key_press: {key:?} is not a W3C KeyboardEvent.key name")
            }),
            code: code.parse::<Code>().unwrap_or_else(|_| {
                panic!("waterui_call_on_key_press: {code:?} is not a W3C KeyboardEvent.code name")
            }),
            modifiers: modifiers_from_ffi(self.modifiers),
            repeat: self.repeat,
        }
    }
}

crate::opaque!(WuiOnKeyPress, RetainedCallback<OnKeyPress>, on_key_press);

impl IntoFFI for OnKeyPress {
    type FFI = *mut WuiOnKeyPress;
    fn into_ffi(self) -> Self::FFI {
        RetainedCallback::new(self).into_ffi()
    }
}

/// Calls an `OnKeyPress` handler with the key press.
///
/// The handler runs with `press` in its environment, readable as
/// `Use<KeyPress>`; its [`WuiKeyHandling`] answer says whether the key keeps
/// bubbling to the next ancestor. This handler can be called multiple times.
///
/// # Safety
///
/// * `handler` must be a valid pointer returned by
///   [`waterui_force_as_metadata_on_key_press`](crate::waterui_force_as_metadata_on_key_press).
/// * `env` must be a valid pointer to a `WuiEnv`.
/// * `press` is consumed by this call — its strings must not be used
///   afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_call_on_key_press(
    handler: *const WuiOnKeyPress,
    env: *const crate::WuiEnv,
    press: WuiKeyPress,
) -> WuiKeyHandling {
    // SAFETY: the caller contract requires `handler` to be a valid handle that
    // stays alive for this call; it is only borrowed. Cloning the callback
    // retains it through a synchronous drop issued reentrantly by the handler.
    let handler = unsafe { crate::borrow_ffi(handler) }.0.clone();
    // SAFETY: the caller contract requires `env` to be a valid handle that
    // stays alive for this call; it is only borrowed.
    let env = unsafe { crate::borrow_ffi(env) }.0.clone();
    // SAFETY: the caller contract hands `press` over for consumption.
    let press = unsafe { press.into_rust() };
    let env = env.extending(press);
    handler.call(|handler| handler.handle(&env)).into_ffi()
}
