//! The key a shortcut arms, as each kit spells it: the `NSMenuItem` key
//! equivalent on macOS and the `UIKeyCommand` input on iOS.

use waterui_controls::menu::{NamedKey, ShortcutKey};

/// The `NSMenuItem` key equivalent a shortcut key arms.
///
/// It is the character itself, or for a named key the character `AppKit` reports that key as — the
/// function-key code points (`NSDeleteFunctionKey` for forward delete ⌦,
/// `NSF5FunctionKey`, …) and the characters of Tab, Return and Escape. The
/// ⌫ key, W3C `Backspace`, sends `NSDeleteCharacter` (U+007F), not
/// `NSBackspaceCharacter`.
///
/// # Panics
///
/// On a named key `AppKit` has no key equivalent for.
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
        NamedKey::Backspace => appkit::NSDeleteCharacter,
        NamedKey::Tab => appkit::NSTabCharacter,
        NamedKey::Enter => appkit::NSCarriageReturnCharacter,
        // AppKit names no constant for Escape; its key equivalent is ESC.
        NamedKey::Escape => 0x1B,
        NamedKey::Delete => appkit::NSDeleteFunctionKey,
        NamedKey::Insert => appkit::NSInsertFunctionKey,
        NamedKey::Home => appkit::NSHomeFunctionKey,
        NamedKey::End => appkit::NSEndFunctionKey,
        NamedKey::PageUp => appkit::NSPageUpFunctionKey,
        NamedKey::PageDown => appkit::NSPageDownFunctionKey,
        NamedKey::ArrowUp => appkit::NSUpArrowFunctionKey,
        NamedKey::ArrowDown => appkit::NSDownArrowFunctionKey,
        NamedKey::ArrowLeft => appkit::NSLeftArrowFunctionKey,
        NamedKey::ArrowRight => appkit::NSRightArrowFunctionKey,
        NamedKey::PrintScreen => appkit::NSPrintScreenFunctionKey,
        NamedKey::Pause => appkit::NSPauseFunctionKey,
        NamedKey::ContextMenu => appkit::NSMenuFunctionKey,
        NamedKey::Help => appkit::NSHelpFunctionKey,
        NamedKey::Clear => appkit::NSClearLineFunctionKey,
        NamedKey::Find => appkit::NSFindFunctionKey,
        NamedKey::Undo => appkit::NSUndoFunctionKey,
        NamedKey::Redo => appkit::NSRedoFunctionKey,
        NamedKey::Select => appkit::NSSelectFunctionKey,
        NamedKey::Execute => appkit::NSExecuteFunctionKey,
        NamedKey::Print => appkit::NSPrintFunctionKey,
        NamedKey::F1 => appkit::NSF1FunctionKey,
        NamedKey::F2 => appkit::NSF2FunctionKey,
        NamedKey::F3 => appkit::NSF3FunctionKey,
        NamedKey::F4 => appkit::NSF4FunctionKey,
        NamedKey::F5 => appkit::NSF5FunctionKey,
        NamedKey::F6 => appkit::NSF6FunctionKey,
        NamedKey::F7 => appkit::NSF7FunctionKey,
        NamedKey::F8 => appkit::NSF8FunctionKey,
        NamedKey::F9 => appkit::NSF9FunctionKey,
        NamedKey::F10 => appkit::NSF10FunctionKey,
        NamedKey::F11 => appkit::NSF11FunctionKey,
        NamedKey::F12 => appkit::NSF12FunctionKey,
        NamedKey::F13 => appkit::NSF13FunctionKey,
        NamedKey::F14 => appkit::NSF14FunctionKey,
        NamedKey::F15 => appkit::NSF15FunctionKey,
        NamedKey::F16 => appkit::NSF16FunctionKey,
        NamedKey::F17 => appkit::NSF17FunctionKey,
        NamedKey::F18 => appkit::NSF18FunctionKey,
        NamedKey::F19 => appkit::NSF19FunctionKey,
        NamedKey::F20 => appkit::NSF20FunctionKey,
        NamedKey::F21 => appkit::NSF21FunctionKey,
        NamedKey::F22 => appkit::NSF22FunctionKey,
        NamedKey::F23 => appkit::NSF23FunctionKey,
        NamedKey::F24 => appkit::NSF24FunctionKey,
        NamedKey::F25 => appkit::NSF25FunctionKey,
        NamedKey::F26 => appkit::NSF26FunctionKey,
        NamedKey::F27 => appkit::NSF27FunctionKey,
        NamedKey::F28 => appkit::NSF28FunctionKey,
        NamedKey::F29 => appkit::NSF29FunctionKey,
        NamedKey::F30 => appkit::NSF30FunctionKey,
        NamedKey::F31 => appkit::NSF31FunctionKey,
        NamedKey::F32 => appkit::NSF32FunctionKey,
        NamedKey::F33 => appkit::NSF33FunctionKey,
        NamedKey::F34 => appkit::NSF34FunctionKey,
        NamedKey::F35 => appkit::NSF35FunctionKey,
        _ => panic!(
            "the shortcut key `{named}` has no AppKit key equivalent, so a macOS menu item cannot \
             arm it; choose a key AppKit menus support"
        ),
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
/// # Panics
///
/// On a named key `UIKit` has no key-command input for.
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
            NamedKey::Backspace => return String::from('\u{8}'),
            NamedKey::Tab => return String::from('\t'),
            NamedKey::Enter => return String::from('\r'),
            NamedKey::Escape => uikit::UIKeyInputEscape,
            NamedKey::Delete => uikit::UIKeyInputDelete,
            NamedKey::Home => uikit::UIKeyInputHome,
            NamedKey::End => uikit::UIKeyInputEnd,
            NamedKey::PageUp => uikit::UIKeyInputPageUp,
            NamedKey::PageDown => uikit::UIKeyInputPageDown,
            NamedKey::ArrowUp => uikit::UIKeyInputUpArrow,
            NamedKey::ArrowDown => uikit::UIKeyInputDownArrow,
            NamedKey::ArrowLeft => uikit::UIKeyInputLeftArrow,
            NamedKey::ArrowRight => uikit::UIKeyInputRightArrow,
            NamedKey::F1 => UIKeyInputF1,
            NamedKey::F2 => uikit::UIKeyInputF2,
            NamedKey::F3 => uikit::UIKeyInputF3,
            NamedKey::F4 => uikit::UIKeyInputF4,
            NamedKey::F5 => uikit::UIKeyInputF5,
            NamedKey::F6 => uikit::UIKeyInputF6,
            NamedKey::F7 => uikit::UIKeyInputF7,
            NamedKey::F8 => uikit::UIKeyInputF8,
            NamedKey::F9 => uikit::UIKeyInputF9,
            NamedKey::F10 => uikit::UIKeyInputF10,
            NamedKey::F11 => uikit::UIKeyInputF11,
            NamedKey::F12 => uikit::UIKeyInputF12,
            _ => panic!(
                "the shortcut key `{named}` has no UIKeyCommand input, so a UIKit menu command \
                 cannot arm it; choose a key UIKit key commands support"
            ),
        }
    };
    input.to_string()
}
