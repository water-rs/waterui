//! Keyboard shortcut definitions shared by buttons, commands, and menus.

use alloc::{
    format,
    string::{String, ToString},
};
use core::fmt;

use nami::impl_constant;
use waterui_core::contract_violation;
use waterui_interaction::key::{Key, KeyPress, Modifiers, NamedKey};

/// The four accelerator modifiers a [`Shortcut`] matches against.
/// Lock and layout modifiers (`CapsLock`, `NumLock`, `Fn`, etc.) are ignored.
const CHORD_MODIFIERS: Modifiers = Modifiers::CONTROL
    .union(Modifiers::ALT)
    .union(Modifiers::SHIFT)
    .union(Modifiers::META);

/// Keyboard shortcut modifiers used by buttons, commands, and system menus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShortcutModifiers {
    flags: u8,
}

impl ShortcutModifiers {
    const COMMAND: u8 = 1 << 0;
    const SHIFT: u8 = 1 << 1;
    const OPTION: u8 = 1 << 2;
    const CONTROL: u8 = 1 << 3;

    /// Returns whether the command modifier is active.
    #[must_use]
    pub const fn command(self) -> bool {
        self.flags & Self::COMMAND != 0
    }

    /// Returns whether the shift modifier is active.
    #[must_use]
    pub const fn shift(self) -> bool {
        self.flags & Self::SHIFT != 0
    }

    /// Returns whether the option/alt modifier is active.
    #[must_use]
    pub const fn option(self) -> bool {
        self.flags & Self::OPTION != 0
    }

    /// Returns whether the control modifier is active.
    #[must_use]
    pub const fn control(self) -> bool {
        self.flags & Self::CONTROL != 0
    }

    /// Resolves these shortcut modifiers to the platform's W3C [`Modifiers`]:
    /// [`command`](Self::command) maps to [`Modifiers::META`] on Apple targets
    /// and [`Modifiers::CONTROL`] elsewhere.
    #[must_use]
    pub const fn to_modifiers(self) -> Modifiers {
        let mut flags = Modifiers::empty();
        #[cfg(target_vendor = "apple")]
        {
            if self.control() {
                flags = flags.union(Modifiers::CONTROL);
            }
            if self.command() {
                flags = flags.union(Modifiers::META);
            }
        }
        #[cfg(not(target_vendor = "apple"))]
        {
            if self.control() || self.command() {
                flags = flags.union(Modifiers::CONTROL);
            }
        }
        if self.option() {
            flags = flags.union(Modifiers::ALT);
        }
        if self.shift() {
            flags = flags.union(Modifiers::SHIFT);
        }
        flags
    }

    const fn inserting(mut self, flag: u8) -> Self {
        self.flags |= flag;
        self
    }
}

/// A named key that desktop platform menus and buttons can arm on a [`Shortcut`].
///
/// Restricts `W3C` [`NamedKey`] values to the portable subset backed by menu
/// accelerators across platforms (`Backspace`, `Tab`, `Enter`, `Escape`,
/// `Delete`, `Insert`, navigation keys, `F1`..=`F24`, and editing/function keys).
/// Convert from a broader [`NamedKey`] fallibly via `TryFrom<NamedKey>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each armable named shortcut key directly; adding a key must fail backend builds instead of hitting a wildcard"
)]
pub enum ShortcutNamedKey {
    /// Arrow Up (`↑`).
    ArrowUp,
    /// Arrow Down (`↓`).
    ArrowDown,
    /// Arrow Left (`←`).
    ArrowLeft,
    /// Arrow Right (`→`).
    ArrowRight,
    /// Backspace (`⌫`).
    Backspace,
    /// Clear key (`⌧`).
    Clear,
    /// Context Menu / Application key.
    ContextMenu,
    /// Forward Delete (`⌦`).
    Delete,
    /// End (`↘`).
    End,
    /// Enter / Return (`↩`).
    Enter,
    /// Escape (`⎋`).
    Escape,
    /// Execute key.
    Execute,
    /// F1 function key.
    F1,
    /// F2 function key.
    F2,
    /// F3 function key.
    F3,
    /// F4 function key.
    F4,
    /// F5 function key.
    F5,
    /// F6 function key.
    F6,
    /// F7 function key.
    F7,
    /// F8 function key.
    F8,
    /// F9 function key.
    F9,
    /// F10 function key.
    F10,
    /// F11 function key.
    F11,
    /// F12 function key.
    F12,
    /// F13 function key.
    F13,
    /// F14 function key.
    F14,
    /// F15 function key.
    F15,
    /// F16 function key.
    F16,
    /// F17 function key.
    F17,
    /// F18 function key.
    F18,
    /// F19 function key.
    F19,
    /// F20 function key.
    F20,
    /// F21 function key.
    F21,
    /// F22 function key.
    F22,
    /// F23 function key.
    F23,
    /// F24 function key.
    F24,
    /// Help key.
    Help,
    /// Home (`↖`).
    Home,
    /// Insert key.
    Insert,
    /// Page Down (`⇟`).
    PageDown,
    /// Page Up (`⇞`).
    PageUp,
    /// Pause key.
    Pause,
    /// Print Screen key.
    PrintScreen,
    /// Select key.
    Select,
    /// Tab (`⇥`).
    Tab,
}

impl ShortcutNamedKey {
    /// Returns the corresponding W3C [`NamedKey`].
    #[must_use]
    pub const fn to_named_key(self) -> NamedKey {
        match self {
            Self::ArrowUp => NamedKey::ArrowUp,
            Self::ArrowDown => NamedKey::ArrowDown,
            Self::ArrowLeft => NamedKey::ArrowLeft,
            Self::ArrowRight => NamedKey::ArrowRight,
            Self::Backspace => NamedKey::Backspace,
            Self::Clear => NamedKey::Clear,
            Self::ContextMenu => NamedKey::ContextMenu,
            Self::Delete => NamedKey::Delete,
            Self::End => NamedKey::End,
            Self::Enter => NamedKey::Enter,
            Self::Escape => NamedKey::Escape,
            Self::Execute => NamedKey::Execute,
            Self::F1 => NamedKey::F1,
            Self::F2 => NamedKey::F2,
            Self::F3 => NamedKey::F3,
            Self::F4 => NamedKey::F4,
            Self::F5 => NamedKey::F5,
            Self::F6 => NamedKey::F6,
            Self::F7 => NamedKey::F7,
            Self::F8 => NamedKey::F8,
            Self::F9 => NamedKey::F9,
            Self::F10 => NamedKey::F10,
            Self::F11 => NamedKey::F11,
            Self::F12 => NamedKey::F12,
            Self::F13 => NamedKey::F13,
            Self::F14 => NamedKey::F14,
            Self::F15 => NamedKey::F15,
            Self::F16 => NamedKey::F16,
            Self::F17 => NamedKey::F17,
            Self::F18 => NamedKey::F18,
            Self::F19 => NamedKey::F19,
            Self::F20 => NamedKey::F20,
            Self::F21 => NamedKey::F21,
            Self::F22 => NamedKey::F22,
            Self::F23 => NamedKey::F23,
            Self::F24 => NamedKey::F24,
            Self::Help => NamedKey::Help,
            Self::Home => NamedKey::Home,
            Self::Insert => NamedKey::Insert,
            Self::PageDown => NamedKey::PageDown,
            Self::PageUp => NamedKey::PageUp,
            Self::Pause => NamedKey::Pause,
            Self::PrintScreen => NamedKey::PrintScreen,
            Self::Select => NamedKey::Select,
            Self::Tab => NamedKey::Tab,
        }
    }

    /// Converts a W3C [`NamedKey`] into a [`ShortcutNamedKey`] when platform menus can arm it.
    #[must_use]
    pub const fn from_named_key(key: NamedKey) -> Option<Self> {
        Some(match key {
            NamedKey::ArrowUp => Self::ArrowUp,
            NamedKey::ArrowDown => Self::ArrowDown,
            NamedKey::ArrowLeft => Self::ArrowLeft,
            NamedKey::ArrowRight => Self::ArrowRight,
            NamedKey::Backspace => Self::Backspace,
            NamedKey::Clear => Self::Clear,
            NamedKey::ContextMenu => Self::ContextMenu,
            NamedKey::Delete => Self::Delete,
            NamedKey::End => Self::End,
            NamedKey::Enter => Self::Enter,
            NamedKey::Escape => Self::Escape,
            NamedKey::Execute => Self::Execute,
            NamedKey::F1 => Self::F1,
            NamedKey::F2 => Self::F2,
            NamedKey::F3 => Self::F3,
            NamedKey::F4 => Self::F4,
            NamedKey::F5 => Self::F5,
            NamedKey::F6 => Self::F6,
            NamedKey::F7 => Self::F7,
            NamedKey::F8 => Self::F8,
            NamedKey::F9 => Self::F9,
            NamedKey::F10 => Self::F10,
            NamedKey::F11 => Self::F11,
            NamedKey::F12 => Self::F12,
            NamedKey::F13 => Self::F13,
            NamedKey::F14 => Self::F14,
            NamedKey::F15 => Self::F15,
            NamedKey::F16 => Self::F16,
            NamedKey::F17 => Self::F17,
            NamedKey::F18 => Self::F18,
            NamedKey::F19 => Self::F19,
            NamedKey::F20 => Self::F20,
            NamedKey::F21 => Self::F21,
            NamedKey::F22 => Self::F22,
            NamedKey::F23 => Self::F23,
            NamedKey::F24 => Self::F24,
            NamedKey::Help => Self::Help,
            NamedKey::Home => Self::Home,
            NamedKey::Insert => Self::Insert,
            NamedKey::PageDown => Self::PageDown,
            NamedKey::PageUp => Self::PageUp,
            NamedKey::Pause => Self::Pause,
            NamedKey::PrintScreen => Self::PrintScreen,
            NamedKey::Select => Self::Select,
            NamedKey::Tab => Self::Tab,
            _ => return None,
        })
    }
}

impl fmt::Display for ShortcutNamedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.to_named_key(), f)
    }
}

impl From<ShortcutNamedKey> for NamedKey {
    fn from(key: ShortcutNamedKey) -> Self {
        key.to_named_key()
    }
}

impl TryFrom<NamedKey> for ShortcutNamedKey {
    type Error = InvalidShortcutKey;

    fn try_from(key: NamedKey) -> Result<Self, Self::Error> {
        Self::from_named_key(key).ok_or_else(|| InvalidShortcutKey::new(key.to_string()))
    }
}

/// The key a [`Shortcut`] fires on: one character, or an armable named key.
///
/// The variants are public so backends can match on them; constructing
/// `Character` directly bypasses control-character validation, so build
/// character keys through `From<char>` (`Shortcut::new`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each shortcut key form variant by variant; a new shortcut key form must fail their build instead of falling into a wildcard arm"
)]
pub enum ShortcutKey {
    /// A key that produces one character. Letters match regardless of case;
    /// Shift is a separate modifier.
    Character(char),
    /// A named key such as `Delete`, `Enter`, `F5` or `ArrowLeft`.
    Named(ShortcutNamedKey),
}

impl ShortcutKey {
    /// Whether a key press reporting `key` fires this shortcut key. A
    /// character matches case-insensitively, so `Shift` stays a separate
    /// modifier rather than changing which key a chord names.
    #[must_use]
    pub fn matches(&self, key: &Key) -> bool {
        match (self, key) {
            (Self::Character(expected), Key::Character(pressed)) => {
                let mut chars = pressed.chars();
                chars.next().is_some_and(|pressed| {
                    chars.next().is_none() && expected.to_lowercase().eq(pressed.to_lowercase())
                })
            }
            (Self::Named(expected), Key::Named(pressed)) => expected.to_named_key() == *pressed,
            _ => false,
        }
    }

    /// The W3C key value of a press that fires this shortcut key.
    #[must_use]
    pub fn to_key(&self) -> Key {
        match self {
            Self::Character(character) => Key::Character(character.to_string()),
            Self::Named(named) => Key::Named(named.to_named_key()),
        }
    }
}

/// The label a menu hint draws for a shortcut key: a letter in upper case,
/// the space bar as `Space`, and a named key in the platform's own notation —
/// the `⌦`/`↩`/`←` glyphs of Apple menus, the `Del`/`Enter`/`Left` text of
/// Windows and Linux menus. A named key without a platform abbreviation
/// shows its W3C name, which is already the label those menus print (`F5`,
/// `Home`).
impl fmt::Display for ShortcutKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Character(' ') => f.write_str("Space"),
            Self::Character(character) => {
                for upper in character.to_uppercase() {
                    write!(f, "{upper}")?;
                }
                Ok(())
            }
            Self::Named(named) => match named_key_label(*named) {
                Some(label) => f.write_str(label),
                None => fmt::Display::fmt(named, f),
            },
        }
    }
}

#[cfg(target_vendor = "apple")]
const fn named_key_label(key: ShortcutNamedKey) -> Option<&'static str> {
    Some(match key {
        ShortcutNamedKey::Delete => "⌦",
        ShortcutNamedKey::Backspace => "⌫",
        ShortcutNamedKey::Enter => "↩",
        ShortcutNamedKey::Escape => "⎋",
        ShortcutNamedKey::Tab => "⇥",
        ShortcutNamedKey::ArrowUp => "↑",
        ShortcutNamedKey::ArrowDown => "↓",
        ShortcutNamedKey::ArrowLeft => "←",
        ShortcutNamedKey::ArrowRight => "→",
        ShortcutNamedKey::PageUp => "⇞",
        ShortcutNamedKey::PageDown => "⇟",
        ShortcutNamedKey::Home => "↖",
        ShortcutNamedKey::End => "↘",
        ShortcutNamedKey::Clear => "⌧",
        _ => return None,
    })
}

#[cfg(not(target_vendor = "apple"))]
const fn named_key_label(key: ShortcutNamedKey) -> Option<&'static str> {
    Some(match key {
        ShortcutNamedKey::Delete => "Del",
        ShortcutNamedKey::Insert => "Ins",
        ShortcutNamedKey::Escape => "Esc",
        ShortcutNamedKey::PageUp => "PgUp",
        ShortcutNamedKey::PageDown => "PgDn",
        ShortcutNamedKey::ArrowUp => "Up",
        ShortcutNamedKey::ArrowDown => "Down",
        ShortcutNamedKey::ArrowLeft => "Left",
        ShortcutNamedKey::ArrowRight => "Right",
        _ => return None,
    })
}

/// The named key a control character stands for, when it has one.
const fn named_key_for_control(character: char) -> Option<ShortcutNamedKey> {
    match character {
        '\u{8}' => Some(ShortcutNamedKey::Backspace),
        '\t' => Some(ShortcutNamedKey::Tab),
        '\n' | '\r' => Some(ShortcutNamedKey::Enter),
        '\u{1b}' => Some(ShortcutNamedKey::Escape),
        '\u{7f}' => Some(ShortcutNamedKey::Delete),
        _ => None,
    }
}

impl From<char> for ShortcutKey {
    /// # Contract violation
    ///
    /// A control character is reported: a press reports the key it stands
    /// for as a named key (`'\t'` is `ShortcutNamedKey::Tab`), never as that
    /// character. A debug build panics; a release build logs it and the
    /// shortcut is created but never fires.
    fn from(character: char) -> Self {
        if character.is_control() {
            contract_violation!(
                "{character:?} is a control character, not a shortcut key: {}; the shortcut is created but never fires",
                named_key_for_control(character).map_or_else(
                    || String::from("no key press reports it, so the shortcut could never fire"),
                    |named| format!(
                        "the key press reports it as `ShortcutNamedKey::{named:?}`, so use that"
                    ),
                )
            );
        }
        Self::Character(character)
    }
}

impl From<ShortcutNamedKey> for ShortcutKey {
    fn from(key: ShortcutNamedKey) -> Self {
        Self::Named(key)
    }
}

impl TryFrom<NamedKey> for ShortcutKey {
    type Error = InvalidShortcutKey;

    fn try_from(key: NamedKey) -> Result<Self, Self::Error> {
        ShortcutNamedKey::try_from(key).map(Self::Named)
    }
}

/// Parses a single character or an armable W3C named key; anything else is an error.
impl core::str::FromStr for ShortcutKey {
    type Err = InvalidShortcutKey;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut chars = value.chars();
        if let (Some(character), None) = (chars.next(), chars.next()) {
            if character.is_control() {
                return Err(InvalidShortcutKey::new(value.to_string()));
            }
            return Ok(Self::Character(character));
        }
        value
            .parse::<NamedKey>()
            .ok()
            .and_then(ShortcutNamedKey::from_named_key)
            .map(Self::Named)
            .ok_or_else(|| InvalidShortcutKey::new(value.to_string()))
    }
}

/// A string that is neither a single character nor an armable named shortcut key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is neither a single character nor an armable shortcut key")]
pub struct InvalidShortcutKey(String);

impl InvalidShortcutKey {
    /// Rejects `key`.
    #[must_use]
    pub const fn new(key: String) -> Self {
        Self(key)
    }

    /// The rejected string.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.0
    }
}

/// Keyboard shortcut metadata attached to a button or command.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Shortcut {
    /// The key the shortcut fires on.
    key: ShortcutKey,
    /// Modifier flags required alongside the key.
    modifiers: ShortcutModifiers,
}

impl Shortcut {
    /// The key the shortcut fires on.
    #[must_use]
    pub const fn key(&self) -> &ShortcutKey {
        &self.key
    }

    /// Modifier flags required alongside the key.
    #[must_use]
    pub const fn modifiers(&self) -> ShortcutModifiers {
        self.modifiers
    }

    /// Creates a shortcut on `key` — a `char` or a [`ShortcutNamedKey`].
    ///
    /// Named keys are restricted at compile time to [`ShortcutNamedKey`] so
    /// unarmable W3C keys such as media, browser, modifier, or lock keys are
    /// rejected unless explicitly converted with `TryFrom<NamedKey>`. Any
    /// per-platform gap (for example desktop-only keys such as `F13`..=`F24`
    /// on iOS) logs a contract violation and omits the key equivalent in
    /// release builds rather than aborting.
    #[must_use]
    pub fn new(key: impl Into<ShortcutKey>) -> Self {
        Self {
            key: key.into(),
            modifiers: ShortcutModifiers::default(),
        }
    }

    /// Adds the command modifier (`⌘` / `META` on Apple targets, `Ctrl` /
    /// `CONTROL` elsewhere).
    #[must_use]
    pub const fn command(mut self) -> Self {
        self.modifiers = self.modifiers.inserting(ShortcutModifiers::COMMAND);
        self
    }

    /// Adds the shift modifier.
    #[must_use]
    pub const fn shift(mut self) -> Self {
        self.modifiers = self.modifiers.inserting(ShortcutModifiers::SHIFT);
        self
    }

    /// Adds the option/alt modifier.
    #[must_use]
    pub const fn option(mut self) -> Self {
        self.modifiers = self.modifiers.inserting(ShortcutModifiers::OPTION);
        self
    }

    /// Adds the control modifier.
    #[must_use]
    pub const fn control(mut self) -> Self {
        self.modifiers = self.modifiers.inserting(ShortcutModifiers::CONTROL);
        self
    }

    /// Whether `press` matches this shortcut on the current platform.
    ///
    /// Matches the logical key (case-insensitively for characters) and requires
    /// the exact chord modifier set (`Control`, `Alt`, `Shift`, `Meta`),
    /// ignoring lock flags such as `CapsLock` or `NumLock`. The `command`
    /// modifier maps to [`Modifiers::META`] on Apple targets and
    /// [`Modifiers::CONTROL`] elsewhere.
    #[must_use]
    pub fn matches(&self, press: &KeyPress) -> bool {
        self.matches_key_and_modifiers(press.key(), press.modifiers())
    }

    /// Whether a logical `key` pressed with `modifiers` matches this shortcut
    /// on the current platform.
    #[must_use]
    pub fn matches_key_and_modifiers(&self, key: &Key, modifiers: Modifiers) -> bool {
        (modifiers & CHORD_MODIFIERS) == self.modifiers.to_modifiers() && self.key.matches(key)
    }
}

/// The trailing hint a menu row draws for the shortcut — `⌃⌥⇧⌘` glyphs on
/// Apple targets, `Ctrl+Alt+Shift+` text elsewhere, where the command
/// modifier is control, the platform's menu accelerator. The key renders
/// through [`ShortcutKey`]'s own `Display`, so a menu row and an
/// application's own listing of its chords cannot disagree.
impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let modifiers = self.modifiers;
        #[cfg(target_vendor = "apple")]
        {
            if modifiers.control() {
                f.write_str("⌃")?;
            }
            if modifiers.option() {
                f.write_str("⌥")?;
            }
            if modifiers.shift() {
                f.write_str("⇧")?;
            }
            if modifiers.command() {
                f.write_str("⌘")?;
            }
        }
        #[cfg(not(target_vendor = "apple"))]
        {
            if modifiers.control() || modifiers.command() {
                f.write_str("Ctrl+")?;
            }
            if modifiers.option() {
                f.write_str("Alt+")?;
            }
            if modifiers.shift() {
                f.write_str("Shift+")?;
            }
        }
        fmt::Display::fmt(&self.key, f)
    }
}

impl_constant!(Shortcut, ShortcutModifiers);

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_interaction::key::Code;

    fn press(key: Key, modifiers: Modifiers) -> KeyPress {
        KeyPress::new(key, Code::Unidentified, modifiers, false)
    }

    #[test]
    fn shortcut_matches_command_modifier_per_platform() {
        let send = Shortcut::new(ShortcutNamedKey::Enter).command();
        let apple_press = press(Key::Named(NamedKey::Enter), Modifiers::META);
        let non_apple_press = press(Key::Named(NamedKey::Enter), Modifiers::CONTROL);

        #[cfg(target_vendor = "apple")]
        {
            assert!(send.matches(&apple_press));
            assert!(!send.matches(&non_apple_press));
        }
        #[cfg(not(target_vendor = "apple"))]
        {
            assert!(send.matches(&non_apple_press));
            assert!(!send.matches(&apple_press));
        }

        // Extra modifiers must not match.
        let extra = press(
            Key::Named(NamedKey::Enter),
            send.modifiers().to_modifiers() | Modifiers::SHIFT,
        );
        assert!(!send.matches(&extra));

        // Lock flags such as CapsLock or NumLock are ignored.
        let with_locks = press(
            Key::Named(NamedKey::Enter),
            send.modifiers().to_modifiers() | Modifiers::CAPS_LOCK | Modifiers::NUM_LOCK,
        );
        assert!(send.matches(&with_locks));
    }

    #[test]
    fn shortcut_matches_character_case_insensitively_and_checks_modifiers() {
        let save_as = Shortcut::new('s').command().shift();
        let expected = save_as.modifiers().to_modifiers();

        assert!(save_as.matches(&press(Key::Character("s".into()), expected)));
        assert!(save_as.matches(&press(Key::Character("S".into()), expected)));
        assert!(!save_as.matches(&press(
            Key::Character("s".into()),
            expected - Modifiers::SHIFT
        )));
        assert!(!save_as.matches(&press(Key::Character("x".into()), expected)));
    }
}
