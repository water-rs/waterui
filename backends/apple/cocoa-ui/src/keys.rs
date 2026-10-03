//! W3C `KeyboardEvent.key` / `.code` / modifier mapping for native key events.
//!
//! No platform keycode crosses the kit boundary: interactive views and
//! keyboard-metadata alike translate native events into the same W3C
//! vocabulary. Codes absent from these tables are keys the platform reports
//! but the W3C model has no name for; they travel as `Unidentified`.
//!
//! # Safety
//!
//! The `unsafe` here reads event properties (`charactersIgnoringModifiers`,
//! `charactersByApplyingModifiers:`, `keyCode`, modifier flags) on live event
//! objects delivered by the frameworks; all are ordinary main-thread reads.

use keyboard_types::{Code, Key, Modifiers, NamedKey};

/// The `KeyboardEvent.code` reported for a physical key with no W3C name.
#[must_use]
pub const fn unidentified_code() -> Code {
    // `Code::Unidentified` exists in the W3C vocabulary.
    Code::Unidentified
}

/// The `KeyboardEvent.key` reported for a key with no W3C name.
#[must_use]
pub const fn unidentified_key() -> Key {
    Key::Named(NamedKey::Unidentified)
}

/// Parses a W3C code name, e.g. `"KeyA"` or `"Enter"`.
fn code_named(name: &str) -> Code {
    name.parse().unwrap_or(Code::Unidentified)
}

/// Parses a W3C key name, e.g. `"ArrowUp"` or a single character.
fn key_named(name: &str) -> Key {
    name.parse().unwrap_or(Key::Named(NamedKey::Unidentified))
}

/// The W3C `key` a modifier key reports, from its `code` name.
#[cfg(target_os = "macos")]
fn modifier_key_name(code: &str) -> Option<&'static str> {
    match code {
        "ShiftLeft" | "ShiftRight" => Some("Shift"),
        "ControlLeft" | "ControlRight" => Some("Control"),
        "AltLeft" | "AltRight" => Some("Alt"),
        "OSLeft" | "OSRight" => Some("Meta"),
        "CapsLock" => Some("CapsLock"),
        "NumLock" => Some("NumLock"),
        "Fn" => Some("Fn"),
        _ => None,
    }
}

/// The character a non-modifier key produces, as the W3C `key`.
fn character_key(characters: &str) -> Key {
    if characters.to_string().chars().count() == 1 {
        Key::Character(characters.into())
    } else {
        key_named(characters)
    }
}

/// One key press, in platform-independent terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    /// What the key means under the current layout and modifiers.
    pub key: Key,
    /// Where the key sits on the keyboard, independent of layout.
    pub code: Code,
    /// The modifiers held with the key.
    pub modifiers: Modifiers,
    /// Whether this press is an auto-repeat of a held key.
    pub repeat: bool,
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2_app_kit::{NSEvent, NSEventModifierFlags};

    use super::{Code, Key, Modifiers, character_key, code_named, key_named, modifier_key_name};

    /// macOS virtual keycodes (`kVK_*`) to W3C `KeyboardEvent.code` names.
    const MAC_VIRTUAL_KEYCODES: &[(u16, &str)] = &[
        (0x00, "KeyA"),
        (0x01, "KeyS"),
        (0x02, "KeyD"),
        (0x03, "KeyF"),
        (0x04, "KeyH"),
        (0x05, "KeyG"),
        (0x06, "KeyZ"),
        (0x07, "KeyX"),
        (0x08, "KeyC"),
        (0x09, "KeyV"),
        (0x0A, "IntlBackslash"),
        (0x0B, "KeyB"),
        (0x0C, "KeyQ"),
        (0x0D, "KeyW"),
        (0x0E, "KeyE"),
        (0x0F, "KeyR"),
        (0x10, "KeyY"),
        (0x11, "KeyT"),
        (0x12, "Digit1"),
        (0x13, "Digit2"),
        (0x14, "Digit3"),
        (0x15, "Digit4"),
        (0x16, "Digit6"),
        (0x17, "Digit5"),
        (0x18, "Equal"),
        (0x19, "Digit9"),
        (0x1A, "Digit7"),
        (0x1B, "Minus"),
        (0x1C, "Digit8"),
        (0x1D, "Digit0"),
        (0x1E, "BracketRight"),
        (0x1F, "KeyO"),
        (0x20, "KeyU"),
        (0x21, "BracketLeft"),
        (0x22, "KeyI"),
        (0x23, "KeyP"),
        (0x24, "Enter"),
        (0x25, "KeyL"),
        (0x26, "KeyJ"),
        (0x27, "Quote"),
        (0x28, "KeyK"),
        (0x29, "Semicolon"),
        (0x2A, "Backslash"),
        (0x2B, "Comma"),
        (0x2C, "Slash"),
        (0x2D, "KeyN"),
        (0x2E, "KeyM"),
        (0x2F, "Period"),
        (0x30, "Tab"),
        (0x31, "Space"),
        (0x32, "Backquote"),
        (0x33, "Backspace"),
        (0x35, "Escape"),
        (0x36, "OSRight"),
        (0x37, "OSLeft"),
        (0x38, "ShiftLeft"),
        (0x39, "CapsLock"),
        (0x3A, "AltLeft"),
        (0x3B, "ControlLeft"),
        (0x3C, "ShiftRight"),
        (0x3D, "AltRight"),
        (0x3E, "ControlRight"),
        (0x3F, "Fn"),
        (0x40, "F17"),
        (0x41, "NumpadDecimal"),
        (0x43, "NumpadMultiply"),
        (0x45, "NumpadAdd"),
        (0x47, "NumLock"),
        (0x48, "VolumeUp"),
        (0x49, "VolumeDown"),
        (0x4A, "VolumeMute"),
        (0x4B, "NumpadDivide"),
        (0x4C, "NumpadEnter"),
        (0x4E, "NumpadSubtract"),
        (0x4F, "F18"),
        (0x50, "F19"),
        (0x51, "NumpadEqual"),
        (0x52, "Numpad0"),
        (0x53, "Numpad1"),
        (0x54, "Numpad2"),
        (0x55, "Numpad3"),
        (0x56, "Numpad4"),
        (0x57, "Numpad5"),
        (0x58, "Numpad6"),
        (0x59, "Numpad7"),
        (0x5A, "F20"),
        (0x5B, "Numpad8"),
        (0x5C, "Numpad9"),
        (0x5D, "IntlYen"),
        (0x5E, "IntlRo"),
        (0x5F, "NumpadComma"),
        (0x60, "F5"),
        (0x61, "F6"),
        (0x62, "F7"),
        (0x63, "F3"),
        (0x64, "F8"),
        (0x65, "F9"),
        (0x66, "Lang2"),
        (0x67, "F11"),
        (0x68, "Lang1"),
        (0x69, "F13"),
        (0x6A, "F16"),
        (0x6B, "F14"),
        (0x6D, "F10"),
        (0x6E, "ContextMenu"),
        (0x6F, "F12"),
        (0x71, "F15"),
        (0x72, "Help"),
        (0x73, "Home"),
        (0x74, "PageUp"),
        (0x75, "Delete"),
        (0x76, "F4"),
        (0x77, "End"),
        (0x78, "F2"),
        (0x79, "PageDown"),
        (0x7A, "F1"),
        (0x7B, "ArrowLeft"),
        (0x7C, "ArrowRight"),
        (0x7D, "ArrowDown"),
        (0x7E, "ArrowUp"),
    ];

    fn mac_virtual_key_code(key_code: u16) -> Option<&'static str> {
        MAC_VIRTUAL_KEYCODES
            .iter()
            .find_map(|(value, name)| (*value == key_code).then_some(*name))
    }

    /// `AppKit`'s private-use function-key code points to W3C `key` names.
    ///
    /// `charactersIgnoringModifiers` reports these keys as characters in
    /// Unicode's private-use area (U+F700–U+F8FF), which is exactly the
    /// platform detail the neutral vocabulary exists to hide.
    const fn mac_function_key_name(scalar: u32) -> Option<&'static str> {
        Some(match scalar {
            0xF700 => "ArrowUp",
            0xF701 => "ArrowDown",
            0xF702 => "ArrowLeft",
            0xF703 => "ArrowRight",
            0xF704 => "F1",
            0xF705 => "F2",
            0xF706 => "F3",
            0xF707 => "F4",
            0xF708 => "F5",
            0xF709 => "F6",
            0xF70A => "F7",
            0xF70B => "F8",
            0xF70C => "F9",
            0xF70D => "F10",
            0xF70E => "F11",
            0xF70F => "F12",
            0xF710 => "F13",
            0xF711 => "F14",
            0xF712 => "F15",
            0xF713 => "F16",
            0xF714 => "F17",
            0xF715 => "F18",
            0xF716 => "F19",
            0xF717 => "F20",
            0xF727 => "Insert",
            0xF728 => "Delete",
            0xF729 => "Home",
            0xF72B => "End",
            0xF72C => "PageUp",
            0xF72D => "PageDown",
            0xF72E => "PrintScreen",
            0xF72F => "ScrollLock",
            0xF730 => "Pause",
            0xF735 => "ContextMenu",
            0xF746 => "Help",
            0xF739 => "Clear",
            _ => return None,
        })
    }

    /// Control characters `AppKit` delivers for keys the W3C model names.
    const fn mac_control_key_name(scalar: u32) -> Option<&'static str> {
        Some(match scalar {
            0x0D | 0x03 => "Enter",
            0x09 | 0x19 => "Tab",
            0x1B => "Escape",
            0x7F => "Backspace",
            _ => return None,
        })
    }

    /// The unmodified character of a key whose event carries a control code.
    fn printable_for_control(event: &NSEvent) -> Option<String> {
        // SAFETY: see the module safety note. `charactersByApplyingModifiers:`
        // takes a copy of the modifier-flags value.
        let characters = event.charactersByApplyingModifiers(NSEventModifierFlags(0))?;
        let scalar = characters.to_string().chars().next()?;
        (u32::from(scalar) >= 0x20).then(|| characters.to_string())
    }

    /// The W3C `KeyboardEvent.code` of the physical key this event came from.
    #[must_use]
    pub fn surface_code(event: &NSEvent) -> Code {
        mac_virtual_key_code(event.keyCode()).map_or(super::unidentified_code(), code_named)
    }

    /// The W3C `KeyboardEvent.code` name of a modifier keycode, for events —
    /// `flagsChanged` — that carry no `code` context beyond the keycode.
    #[must_use]
    pub fn modifier_code_name(key_code: u16) -> Option<&'static str> {
        mac_virtual_key_code(key_code)
    }

    /// The W3C `KeyboardEvent.key` — the value the layout and modifiers
    /// produce.
    ///
    /// Modifier keys report themselves by name, function and control keys map
    /// out of `AppKit`'s private-use encoding, and everything else is the
    /// character the key types.
    #[must_use]
    pub fn surface_key(event: &NSEvent, code: Code) -> Key {
        if let Some(name) = code_name(code).and_then(modifier_key_name) {
            return key_named(name);
        }
        // SAFETY: see the module safety note.
        let Some(characters) = event.charactersIgnoringModifiers() else {
            return super::unidentified_key();
        };
        let Some(scalar) = characters.to_string().chars().next() else {
            return super::unidentified_key();
        };
        let value = u32::from(scalar);
        if let Some(name) = mac_function_key_name(value) {
            return key_named(name);
        }
        if let Some(name) = mac_control_key_name(value) {
            return key_named(name);
        }
        // A chord such as ⌃A yields the control character, not the letter;
        // the W3C `key` for it is still the letter the physical key types.
        if value < 0x20
            && let Some(printable) = printable_for_control(event)
        {
            return character_key(&printable);
        }
        character_key(&characters.to_string())
    }

    /// The `code` name of a [`Code`], for the modifier-key lookup.
    fn code_name(code: Code) -> Option<&'static str> {
        // `keyboard_types::Code` has no `name()`; the W3C table above maps
        // names to codes, so reverse it once.
        const NAMES: &[&str] = &[
            "ShiftLeft",
            "ShiftRight",
            "ControlLeft",
            "ControlRight",
            "AltLeft",
            "AltRight",
            "OSLeft",
            "OSRight",
            "CapsLock",
            "NumLock",
            "Fn",
        ];
        NAMES.iter().copied().find(|name| code_named(name) == code)
    }

    /// The W3C `key` name of a modifier keycode, e.g. `0x38` → `"Shift"`.
    ///
    /// `flagsChanged` events name the modifier by keycode rather than
    /// character.
    #[must_use]
    pub fn modifier_key_for_keycode(key_code: u16) -> Option<Key> {
        modifier_code_name(key_code)
            .and_then(modifier_key_name)
            .map(key_named)
    }

    /// The modifier chord, as [`Modifiers`] bits.
    #[must_use]
    pub fn surface_modifiers(flags: NSEventModifierFlags) -> Modifiers {
        let mut modifiers = Modifiers::empty();
        if flags.contains(NSEventModifierFlags::Shift) {
            modifiers |= Modifiers::SHIFT;
        }
        if flags.contains(NSEventModifierFlags::Control) {
            modifiers |= Modifiers::CONTROL;
        }
        if flags.contains(NSEventModifierFlags::Option) {
            modifiers |= Modifiers::ALT;
        }
        if flags.contains(NSEventModifierFlags::Command) {
            modifiers |= Modifiers::META;
        }
        if flags.contains(NSEventModifierFlags::CapsLock) {
            modifiers |= Modifiers::CAPS_LOCK;
        }
        if flags.contains(NSEventModifierFlags::NumericPad) {
            modifiers |= Modifiers::NUM_LOCK;
        }
        modifiers
    }

    /// Whether `key_code` is a modifier key (`flagsChanged` only fires for
    /// these).
    #[must_use]
    pub fn is_modifier_keycode(key_code: u16) -> bool {
        mac_virtual_key_code(key_code)
            .and_then(modifier_key_name)
            .is_some()
    }

    /// Whether a `flagsChanged` event reports the modifier currently pressed.
    ///
    /// `AppKit` signals modifier key state through the modifier flags rather
    /// than through a dedicated pressed bit: the flag for the key's modifier
    /// is set while it is held.
    #[must_use]
    pub fn modifier_is_pressed(event: &NSEvent, key_code: u16) -> bool {
        let flags = event.modifierFlags();
        match mac_virtual_key_code(key_code) {
            Some("ShiftLeft" | "ShiftRight") => flags.contains(NSEventModifierFlags::Shift),
            Some("ControlLeft" | "ControlRight") => flags.contains(NSEventModifierFlags::Control),
            Some("AltLeft" | "AltRight") => flags.contains(NSEventModifierFlags::Option),
            Some("OSLeft" | "OSRight") => flags.contains(NSEventModifierFlags::Command),
            Some("CapsLock") => flags.contains(NSEventModifierFlags::CapsLock),
            // Fn, NumLock and anything else have no flag to read.
            _ => false,
        }
    }
    /// The press this `keyDown` event describes.
    #[must_use]
    pub fn key_event(event: &NSEvent) -> super::KeyEvent {
        let code = surface_code(event);
        super::KeyEvent {
            key: surface_key(event, code),
            code,
            modifiers: surface_modifiers(event.modifierFlags()),
            repeat: event.isARepeat(),
        }
    }
}
#[cfg(target_os = "ios")]
mod imp {
    use objc2_ui_kit::{UIKey, UIKeyModifierFlags, UIPress};

    use super::{Code, Key, Modifiers, character_key, code_named, key_named};

    /// `UIKit` `UIKeyboardHIDUsage` values to W3C `KeyboardEvent.code` names.
    ///
    /// `UIKey.keyCode` is a USB HID usage, so this is the HID keyboard page
    /// mapped onto the same vocabulary `AppKit`'s virtual keycodes map onto.
    #[allow(clippy::too_many_lines)]
    const fn hid_usage_code(usage: u32) -> Option<&'static str> {
        Some(match usage {
            0x04 => "KeyA",
            0x05 => "KeyB",
            0x06 => "KeyC",
            0x07 => "KeyD",
            0x08 => "KeyE",
            0x09 => "KeyF",
            0x0A => "KeyG",
            0x0B => "KeyH",
            0x0C => "KeyI",
            0x0D => "KeyJ",
            0x0E => "KeyK",
            0x0F => "KeyL",
            0x10 => "KeyM",
            0x11 => "KeyN",
            0x12 => "KeyO",
            0x13 => "KeyP",
            0x14 => "KeyQ",
            0x15 => "KeyR",
            0x16 => "KeyS",
            0x17 => "KeyT",
            0x18 => "KeyU",
            0x19 => "KeyV",
            0x1A => "KeyW",
            0x1B => "KeyX",
            0x1C => "KeyY",
            0x1D => "KeyZ",
            0x1E => "Digit1",
            0x1F => "Digit2",
            0x20 => "Digit3",
            0x21 => "Digit4",
            0x22 => "Digit5",
            0x23 => "Digit6",
            0x24 => "Digit7",
            0x25 => "Digit8",
            0x26 => "Digit9",
            0x27 => "Digit0",
            0x28 => "Enter",
            0x29 => "Escape",
            0x2A => "Backspace",
            0x2B => "Tab",
            0x2C => "Space",
            0x2D => "Minus",
            0x2E => "Equal",
            0x2F => "BracketLeft",
            0x30 => "BracketRight",
            0x31 => "Backslash",
            0x33 => "Semicolon",
            0x34 => "Quote",
            0x35 => "Backquote",
            0x36 => "Comma",
            0x37 => "Period",
            0x38 => "Slash",
            0x39 => "CapsLock",
            0x3A => "F1",
            0x3B => "F2",
            0x3C => "F3",
            0x3D => "F4",
            0x3E => "F5",
            0x3F => "F6",
            0x40 => "F7",
            0x41 => "F8",
            0x42 => "F9",
            0x43 => "F10",
            0x44 => "F11",
            0x45 => "F12",
            0x46 => "PrintScreen",
            0x47 => "ScrollLock",
            0x48 => "Pause",
            0x49 => "Insert",
            0x4A => "Home",
            0x4B => "PageUp",
            0x4C => "Delete",
            0x4D => "End",
            0x4E => "PageDown",
            0x4F => "ArrowRight",
            0x50 => "ArrowLeft",
            0x51 => "ArrowDown",
            0x52 => "ArrowUp",
            0x53 => "NumLock",
            0x54 => "NumpadDivide",
            0x55 => "NumpadMultiply",
            0x56 => "NumpadSubtract",
            0x57 => "NumpadAdd",
            0x58 => "NumpadEnter",
            0x59 => "Numpad1",
            0x5A => "Numpad2",
            0x5B => "Numpad3",
            0x5C => "Numpad4",
            0x5D => "Numpad5",
            0x5E => "Numpad6",
            0x5F => "Numpad7",
            0x60 => "Numpad8",
            0x61 => "Numpad9",
            0x62 => "Numpad0",
            0x63 => "NumpadDecimal",
            0x64 => "IntlBackslash",
            0x65 => "ContextMenu",
            0x67 => "NumpadEqual",
            0x68 => "F13",
            0x69 => "F14",
            0x6A => "F15",
            0x6B => "F16",
            0x6C => "F17",
            0x6D => "F18",
            0x6E => "F19",
            0x6F => "F20",
            0x75 => "Help",
            0x85 => "NumpadComma",
            0x87 => "IntlRo",
            0x88 => "Lang1",
            0x89 => "IntlYen",
            0x8A => "Lang2",
            0xE0 => "ControlLeft",
            0xE1 => "ShiftLeft",
            0xE2 => "AltLeft",
            0xE3 => "OSLeft",
            0xE4 => "ControlRight",
            0xE5 => "ShiftRight",
            0xE6 => "AltRight",
            0xE7 => "OSRight",
            _ => return None,
        })
    }

    /// The W3C `key` a HID usage names on its own, before the layout speaks.
    const fn hid_usage_key(usage: u32) -> Option<&'static str> {
        Some(match usage {
            0x28 | 0x58 => "Enter",
            0x29 => "Escape",
            0x2A => "Backspace",
            0x2B => "Tab",
            0x39 => "CapsLock",
            0x3A => "F1",
            0x3B => "F2",
            0x3C => "F3",
            0x3D => "F4",
            0x3E => "F5",
            0x3F => "F6",
            0x40 => "F7",
            0x41 => "F8",
            0x42 => "F9",
            0x43 => "F10",
            0x44 => "F11",
            0x45 => "F12",
            0x46 => "PrintScreen",
            0x47 => "ScrollLock",
            0x48 => "Pause",
            0x49 => "Insert",
            0x4A => "Home",
            0x4B => "PageUp",
            0x4C => "Delete",
            0x4D => "End",
            0x4E => "PageDown",
            0x4F => "ArrowRight",
            0x50 => "ArrowLeft",
            0x51 => "ArrowDown",
            0x52 => "ArrowUp",
            0x53 => "NumLock",
            0x65 => "ContextMenu",
            0x68 => "F13",
            0x69 => "F14",
            0x6A => "F15",
            0x6B => "F16",
            0x6C => "F17",
            0x6D => "F18",
            0x6E => "F19",
            0x6F => "F20",
            0x75 => "Help",
            0xE0 | 0xE4 => "Control",
            0xE1 | 0xE5 => "Shift",
            0xE2 | 0xE6 => "Alt",
            0xE3 | 0xE7 => "Meta",
            _ => return None,
        })
    }

    /// The W3C `KeyboardEvent.code` of the physical key this press came from.
    #[must_use]
    pub fn surface_code(key: &UIKey) -> Code {
        hid_usage_code(u32::try_from(key.keyCode().0).unwrap_or(u32::MAX))
            .map_or(super::unidentified_code(), code_named)
    }

    /// The W3C `KeyboardEvent.key` this press produces.
    #[must_use]
    pub fn surface_key(key: &UIKey) -> Key {
        if let Some(named) = hid_usage_key(u32::try_from(key.keyCode().0).unwrap_or(u32::MAX)) {
            return key_named(named);
        }
        let characters = key.charactersIgnoringModifiers();
        if characters.is_empty() {
            return super::unidentified_key();
        }
        character_key(&characters.to_string())
    }

    /// The modifier chord, as [`Modifiers`] bits.
    #[must_use]
    pub fn surface_modifiers(flags: UIKeyModifierFlags) -> Modifiers {
        let mut modifiers = Modifiers::empty();
        if flags.contains(UIKeyModifierFlags::Shift) {
            modifiers |= Modifiers::SHIFT;
        }
        if flags.contains(UIKeyModifierFlags::Control) {
            modifiers |= Modifiers::CONTROL;
        }
        if flags.contains(UIKeyModifierFlags::Alternate) {
            modifiers |= Modifiers::ALT;
        }
        if flags.contains(UIKeyModifierFlags::Command) {
            modifiers |= Modifiers::META;
        }
        if flags.contains(UIKeyModifierFlags::AlphaShift) {
            modifiers |= Modifiers::CAPS_LOCK;
        }
        if flags.contains(UIKeyModifierFlags::NumericPad) {
            modifiers |= Modifiers::NUM_LOCK;
        }
        modifiers
    }

    /// Whether `press` is a modifier key press.
    ///
    /// Modifier presses emit `Modifiers`/`Key` pairs via the press handler;
    /// `press.key` identifies them by HID usage.
    #[must_use]
    pub fn is_modifier_press(mtm: objc2::MainThreadMarker, press: &UIPress) -> bool {
        press.key(mtm).is_some_and(|key| {
            hid_usage_key(u32::try_from(key.keyCode().0).unwrap_or(u32::MAX)).is_some_and(|_| {
                // A modifier keycode is exactly one the *code* table lists
                // in the 0xE0..=0xE7 modifier range.
                matches!(key.keyCode().0, 0xE0..=0xE7)
            })
        })
    }
    /// The press this `UIPress`'s key describes.
    #[must_use]
    pub fn key_event(key: &UIKey) -> super::KeyEvent {
        let code = surface_code(key);
        super::KeyEvent {
            key: surface_key(key),
            code,
            modifiers: surface_modifiers(key.modifierFlags()),
            repeat: false,
        }
    }
}
pub use imp::*;
