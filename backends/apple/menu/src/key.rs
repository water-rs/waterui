//! The key a shortcut arms, as each kit spells it: the `NSMenuItem` key
//! equivalent on macOS and the `UIKeyCommand` input on iOS.

use waterui_controls::menu::{ShortcutKey, ShortcutNamedKey};

/// The `NSMenuItem` key equivalent a shortcut key arms.
///
/// It is the character itself, or for a [`ShortcutNamedKey`] the character
/// `AppKit` reports that key as — the function-key code points
/// (`NSDeleteFunctionKey` for forward delete ⌦, `NSF5FunctionKey`, …) and the
/// characters of Tab, Return and Escape. The ⌫ key, W3C `Backspace`, sends
/// `NSDeleteCharacter` (U+007F), not `NSBackspaceCharacter`.
///
/// # Panics
///
/// Panics if a platform key-equivalent code point is not a valid Unicode scalar value.
#[must_use]
#[cfg(target_os = "macos")]
pub fn key_equivalent_for(key: &ShortcutKey) -> String {
    use cocoa_ui::objc2_app_kit as appkit;
    let named = match key {
        // An uppercase key equivalent implies Shift; Shift comes only from
        // the shortcut's modifiers.
        ShortcutKey::Character(character) => return character.to_lowercase().collect(),
        ShortcutKey::Named(named) => *named,
    };
    let code = match named {
        ShortcutNamedKey::Backspace => appkit::NSDeleteCharacter,
        ShortcutNamedKey::Tab => appkit::NSTabCharacter,
        ShortcutNamedKey::Enter => appkit::NSCarriageReturnCharacter,
        // AppKit names no constant for Escape; its key equivalent is ESC.
        ShortcutNamedKey::Escape => 0x1B,
        ShortcutNamedKey::Delete => appkit::NSDeleteFunctionKey,
        ShortcutNamedKey::Insert => appkit::NSInsertFunctionKey,
        ShortcutNamedKey::Home => appkit::NSHomeFunctionKey,
        ShortcutNamedKey::End => appkit::NSEndFunctionKey,
        ShortcutNamedKey::PageUp => appkit::NSPageUpFunctionKey,
        ShortcutNamedKey::PageDown => appkit::NSPageDownFunctionKey,
        ShortcutNamedKey::ArrowUp => appkit::NSUpArrowFunctionKey,
        ShortcutNamedKey::ArrowDown => appkit::NSDownArrowFunctionKey,
        ShortcutNamedKey::ArrowLeft => appkit::NSLeftArrowFunctionKey,
        ShortcutNamedKey::ArrowRight => appkit::NSRightArrowFunctionKey,
        ShortcutNamedKey::PrintScreen => appkit::NSPrintScreenFunctionKey,
        ShortcutNamedKey::Pause => appkit::NSPauseFunctionKey,
        ShortcutNamedKey::ContextMenu => appkit::NSMenuFunctionKey,
        ShortcutNamedKey::Help => appkit::NSHelpFunctionKey,
        ShortcutNamedKey::Clear => appkit::NSClearLineFunctionKey,
        ShortcutNamedKey::Select => appkit::NSSelectFunctionKey,
        ShortcutNamedKey::Execute => appkit::NSExecuteFunctionKey,
        ShortcutNamedKey::F1 => appkit::NSF1FunctionKey,
        ShortcutNamedKey::F2 => appkit::NSF2FunctionKey,
        ShortcutNamedKey::F3 => appkit::NSF3FunctionKey,
        ShortcutNamedKey::F4 => appkit::NSF4FunctionKey,
        ShortcutNamedKey::F5 => appkit::NSF5FunctionKey,
        ShortcutNamedKey::F6 => appkit::NSF6FunctionKey,
        ShortcutNamedKey::F7 => appkit::NSF7FunctionKey,
        ShortcutNamedKey::F8 => appkit::NSF8FunctionKey,
        ShortcutNamedKey::F9 => appkit::NSF9FunctionKey,
        ShortcutNamedKey::F10 => appkit::NSF10FunctionKey,
        ShortcutNamedKey::F11 => appkit::NSF11FunctionKey,
        ShortcutNamedKey::F12 => appkit::NSF12FunctionKey,
        ShortcutNamedKey::F13 => appkit::NSF13FunctionKey,
        ShortcutNamedKey::F14 => appkit::NSF14FunctionKey,
        ShortcutNamedKey::F15 => appkit::NSF15FunctionKey,
        ShortcutNamedKey::F16 => appkit::NSF16FunctionKey,
        ShortcutNamedKey::F17 => appkit::NSF17FunctionKey,
        ShortcutNamedKey::F18 => appkit::NSF18FunctionKey,
        ShortcutNamedKey::F19 => appkit::NSF19FunctionKey,
        ShortcutNamedKey::F20 => appkit::NSF20FunctionKey,
        ShortcutNamedKey::F21 => appkit::NSF21FunctionKey,
        ShortcutNamedKey::F22 => appkit::NSF22FunctionKey,
        ShortcutNamedKey::F23 => appkit::NSF23FunctionKey,
        ShortcutNamedKey::F24 => appkit::NSF24FunctionKey,
    };
    String::from(
        char::from_u32(code).expect("AppKit key-equivalent constants are Unicode scalar values"),
    )
}

/// The `UIKeyCommand` input a shortcut key arms.
///
/// It is the character itself, the
/// `UIKeyInput*` constant for a named key `UIKit` names, and the control
/// characters of Backspace, Tab and Return, which `UIKit` takes as inputs.
///
/// A named key `UIKit` has no key-command input for is a contract
/// violation: a debug build panics, a release build reports it and answers
/// the empty input, which arms no shortcut.
///
/// # Panics
///
/// In a debug build, on a named key `UIKit` has no key-command input for.
#[must_use]
#[cfg(not(target_os = "macos"))]
pub fn key_equivalent_for(key: &ShortcutKey) -> String {
    use cocoa_ui::objc2_foundation::NSString;
    use cocoa_ui::objc2_ui_kit as uikit;

    // objc2-ui-kit 0.3 binds `UIKeyInputF2`…`F12` but not `UIKeyInputF1`,
    // which `UIResponder.h` declares beside them (iOS 13.4).
    unsafe extern "C" {
        static UIKeyInputF1: &'static NSString;
    }

    let named = match key {
        // An uppercase key equivalent implies Shift; Shift comes only from
        // the shortcut's modifiers.
        ShortcutKey::Character(character) => return character.to_lowercase().collect(),
        ShortcutKey::Named(named) => *named,
    };
    // SAFETY: each `UIKeyInput*` is an immutable `NSString` constant UIKit
    // exports for the process's lifetime.
    let input: &NSString = unsafe {
        match named {
            ShortcutNamedKey::Backspace => return String::from('\u{8}'),
            ShortcutNamedKey::Tab => return String::from('\t'),
            ShortcutNamedKey::Enter => return String::from('\r'),
            ShortcutNamedKey::Escape => uikit::UIKeyInputEscape,
            ShortcutNamedKey::Delete => uikit::UIKeyInputDelete,
            ShortcutNamedKey::Home => uikit::UIKeyInputHome,
            ShortcutNamedKey::End => uikit::UIKeyInputEnd,
            ShortcutNamedKey::PageUp => uikit::UIKeyInputPageUp,
            ShortcutNamedKey::PageDown => uikit::UIKeyInputPageDown,
            ShortcutNamedKey::ArrowUp => uikit::UIKeyInputUpArrow,
            ShortcutNamedKey::ArrowDown => uikit::UIKeyInputDownArrow,
            ShortcutNamedKey::ArrowLeft => uikit::UIKeyInputLeftArrow,
            ShortcutNamedKey::ArrowRight => uikit::UIKeyInputRightArrow,
            ShortcutNamedKey::F1 => UIKeyInputF1,
            ShortcutNamedKey::F2 => uikit::UIKeyInputF2,
            ShortcutNamedKey::F3 => uikit::UIKeyInputF3,
            ShortcutNamedKey::F4 => uikit::UIKeyInputF4,
            ShortcutNamedKey::F5 => uikit::UIKeyInputF5,
            ShortcutNamedKey::F6 => uikit::UIKeyInputF6,
            ShortcutNamedKey::F7 => uikit::UIKeyInputF7,
            ShortcutNamedKey::F8 => uikit::UIKeyInputF8,
            ShortcutNamedKey::F9 => uikit::UIKeyInputF9,
            ShortcutNamedKey::F10 => uikit::UIKeyInputF10,
            ShortcutNamedKey::F11 => uikit::UIKeyInputF11,
            ShortcutNamedKey::F12 => uikit::UIKeyInputF12,
            ShortcutNamedKey::Clear
            | ShortcutNamedKey::ContextMenu
            | ShortcutNamedKey::Execute
            | ShortcutNamedKey::Help
            | ShortcutNamedKey::Insert
            | ShortcutNamedKey::Pause
            | ShortcutNamedKey::PrintScreen
            | ShortcutNamedKey::Select
            | ShortcutNamedKey::F13
            | ShortcutNamedKey::F14
            | ShortcutNamedKey::F15
            | ShortcutNamedKey::F16
            | ShortcutNamedKey::F17
            | ShortcutNamedKey::F18
            | ShortcutNamedKey::F19
            | ShortcutNamedKey::F20
            | ShortcutNamedKey::F21
            | ShortcutNamedKey::F22
            | ShortcutNamedKey::F23
            | ShortcutNamedKey::F24 => {
                waterui_core::contract_violation!(
                    "the shortcut key `{named}` has no UIKeyCommand input, so a UIKit menu \
                     command cannot arm it; choose a key UIKit key commands support; the \
                     command has no shortcut"
                );
                return String::new();
            }
        }
    };
    input.to_string()
}
