//! CEF input adaptation, expressed against the backend-neutral surface
//! vocabulary.
//!
//! Every backend that embeds a CEF page used to re-derive the same handful of
//! Chromium facts for itself: that a wheel notch is 120 units and the fraction
//! left over has to be carried into the next event, that a keystroke needs a
//! Windows virtual key *and* a platform hardware code, that the character a key
//! types is what makes CEF emit a `char` event at all, and that ⌘Z/⌘X/⌘C/⌘V/⌘A
//! are frame commands rather than keystrokes on macOS. Three copies of that
//! knowledge existed, and they had already drifted.
//!
//! [`CefSurfaceInput`] is the single copy. A backend translates its own
//! platform events into [`SurfaceInputEvent`] — which it must do anyway, for
//! every other interactive GPU surface — and hands them here.
//!
//! # Event order
//!
//! Text a keystroke produces may arrive either side of the
//! [`SurfaceInputEvent::Key`] that produced it. The adapter holds a
//! [`SurfaceInputEvent::TextInput`] until the next key press and prefers it
//! over the character the logical key implies, because the platform's committed
//! text is the authority on what a dead key or an accented layout actually
//! typed. A backend that emits no text at all (GTK) is equally well served: the
//! logical key carries the character.

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::str::FromStr as _;

use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, StretchAxis, ViewDimensions};
use waterui_graphics::gpu_surface::{GpuContext, GpuFrame, GpuView};
use waterui_graphics::input::{
    Code, Key, Modifiers, NamedKey, ScrollUnit, SurfaceInputEvent, SurfacePointerButton,
};

use crate::page::{CefInputModifiers, CefKeyInput, CefPageHandle, CefPointerButton};

/// Chromium counts one wheel notch as 120 units, and everything downstream of
/// `send_mouse_wheel_event` divides by it.
const CEF_WHEEL_DELTA: f64 = 120.0;

/// Chromium's keycode table marks "this key has no code on that platform" with
/// this sentinel rather than omitting the row.
#[cfg(any(target_os = "macos", target_os = "linux"))]
const KEYCODE_UNMAPPED: u16 = 0xffff;

/// Drives one CEF page from the backend-neutral surface input vocabulary.
///
/// Holds the state CEF's windowless input ABI needs but does not carry in its
/// events: the modifier chord, which pointer buttons are down, the sub-notch
/// wheel remainder, and the text a keystroke produced.
///
/// Positions are logical and surface-local, exactly as
/// [`SurfaceInputEvent`] defines them: the page's own top-left is `(0, 0)`.
#[derive(Debug)]
pub struct CefSurfaceInput {
    page: CefPageHandle,
    /// Both halves of CEF's modifier word — the keyboard chord and the pressed
    /// buttons — because CEF sends them in one field on every event.
    modifiers: CefInputModifiers,
    /// The fraction of a wheel unit that did not survive rounding to CEF's
    /// integer ABI. Dropping it turns a slow trackpad glide into no scroll at
    /// all.
    wheel_remainder: (f64, f64),
    pending_text: Option<String>,
}

impl CefSurfaceInput {
    /// Creates an input adapter for one CEF page.
    #[must_use]
    pub fn new(page: CefPageHandle) -> Self {
        Self {
            page,
            modifiers: CefInputModifiers::default(),
            wheel_remainder: (0.0, 0.0),
            pending_text: None,
        }
    }

    /// The page this adapter drives.
    #[must_use]
    pub const fn page(&self) -> &CefPageHandle {
        &self.page
    }

    /// Applies one input event to the page.
    ///
    /// # Panics
    ///
    /// Panics when two [`SurfaceInputEvent::TextInput`] events arrive with no
    /// key press between them, or when a composition caret is not on a
    /// character boundary of its text.
    pub fn handle(&mut self, event: &SurfaceInputEvent) {
        match event {
            SurfaceInputEvent::Focus(focused) => self.page.set_focus(*focused),
            SurfaceInputEvent::Modifiers(modifiers) => self.set_modifiers(*modifiers),
            SurfaceInputEvent::PointerMove { position } => {
                self.page
                    .pointer_move(position.x, position.y, self.modifiers);
            }
            SurfaceInputEvent::PointerButton {
                pressed,
                button,
                position,
            } => self.pointer_button(*pressed, *button, position.x, position.y),
            SurfaceInputEvent::Scroll {
                position,
                delta_x,
                delta_y,
                unit,
                ..
            } => self.scroll(position.x, position.y, *delta_x, *delta_y, *unit),
            SurfaceInputEvent::Key {
                pressed,
                key,
                code,
                modifiers,
                ..
            } => {
                self.set_modifiers(*modifiers);
                self.key(*pressed, key, *code);
            }
            SurfaceInputEvent::TextInput(text) => {
                let previous = self.pending_text.replace(text.to_string());
                assert!(
                    previous.is_none(),
                    "CEF received consecutive text input without the corresponding key event"
                );
            }
            // CEF has no "a composition began" call: the first pre-edit opens
            // the session on the browser side.
            SurfaceInputEvent::CompositionStart => {}
            SurfaceInputEvent::CompositionUpdate { text, caret } => {
                let selection = composition_selection(text, *caret);
                self.page.set_composition(text, selection, selection, None);
            }
            SurfaceInputEvent::CompositionCommit(text) => self.page.commit_text(text, None),
            SurfaceInputEvent::CompositionCancel => self.page.cancel_composition(),
        }
    }

    /// Replaces the keyboard chord, keeping the pressed-button half.
    const fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.modifiers = CefInputModifiers {
            shift: modifiers.contains(Modifiers::SHIFT),
            control: modifiers.contains(Modifiers::CONTROL),
            alt: modifiers.contains(Modifiers::ALT),
            command: modifiers.contains(Modifiers::META),
            ..self.modifiers
        };
    }

    fn pointer_button(&mut self, pressed: bool, button: SurfacePointerButton, x: f64, y: f64) {
        let Some(button) = cef_pointer_button(button) else {
            // Chromium's OSR input ABI has three buttons; the side buttons are
            // navigation gestures instead, which is what a browser does with
            // them anyway.
            if pressed {
                match button {
                    SurfacePointerButton::Back => self.page.go_back(),
                    SurfacePointerButton::Forward => self.page.go_forward(),
                    _ => unreachable!("only navigation buttons omit a CEF pointer button"),
                }
            }
            return;
        };
        match button {
            CefPointerButton::Primary => self.modifiers.primary_button = pressed,
            CefPointerButton::Middle => self.modifiers.middle_button = pressed,
            CefPointerButton::Secondary => self.modifiers.secondary_button = pressed,
        }
        self.page
            .pointer_button(pressed, button, x, y, self.modifiers);
    }

    fn scroll(&mut self, x: f64, y: f64, delta_x: f64, delta_y: f64, unit: ScrollUnit) {
        // The event that ends a continuous gesture carries no motion, and CEF
        // has no wheel event that means "the gesture is over".
        if delta_x == 0.0 && delta_y == 0.0 {
            return;
        }
        let multiplier = match unit {
            ScrollUnit::Line => CEF_WHEEL_DELTA,
            ScrollUnit::Pixel => 1.0,
        };
        let delta_x = delta_x.mul_add(multiplier, self.wheel_remainder.0);
        let delta_y = delta_y.mul_add(multiplier, self.wheel_remainder.1);
        let integral_x = delta_x.round();
        let integral_y = delta_y.round();
        self.wheel_remainder = (delta_x - integral_x, delta_y - integral_y);
        self.page
            .scroll(x, y, integral_x, integral_y, self.modifiers);
    }

    fn key(&mut self, pressed: bool, key: &Key, code: Code) {
        let text = if pressed {
            self.pending_text.take()
        } else {
            None
        };
        // The platform's committed text wins over the character the logical key
        // implies: a dead key resolving to "é" types "é", not the accent.
        let text_character = text.as_deref().and_then(single_cef_character);
        let input = CefKeyInput {
            native_keycode: native_key_code(code),
            keyval: windows_virtual_key(key),
            character: text_character.or_else(|| key_character(key)),
        };
        self.page.key(pressed, input, self.modifiers);
        #[cfg(target_os = "macos")]
        if pressed && let Some(command) = MacEditShortcut::from_input(key, self.modifiers) {
            command.execute(&self.page);
        }
        // Text CEF's single-UTF-16-unit character field cannot carry — an
        // emoji, a ligature, a pasted run — is inserted as an edit instead.
        if text_character.is_none()
            && let Some(text) = text
        {
            self.page.commit_text(&text, None);
        }
    }
}

/// A CEF presenter that also consumes the input landing on its surface.
///
/// The presenter and the input adapter are separate concerns — one owns the
/// shared texture, the other owns Chromium's input ABI — but a backend that
/// routes input to GPU views by
/// [`wants_input_events`](GpuView::wants_input_events) needs them as one
/// object. See [`gpu_view_with_input`](crate::gpu_view_with_input).
pub struct CefInputGpuView<V> {
    view: V,
    input: CefSurfaceInput,
}

impl<V> CefInputGpuView<V> {
    pub const fn new(view: V, input: CefSurfaceInput) -> Self {
        Self { view, input }
    }
}

impl<V: GpuView> GpuView for CefInputGpuView<V> {
    #[expect(
        clippy::future_not_send,
        reason = "CEF and WaterUI view state are confined to the UI thread"
    )]
    async fn setup(&mut self, ctx: &GpuContext<'_>, env: &mut Environment) {
        self.view.setup(ctx, env).await;
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        self.view.render(frame);
    }

    fn preferred_surface_hdr(&self) -> Option<bool> {
        self.view.preferred_surface_hdr()
    }

    fn wants_input_events(&self) -> bool {
        true
    }

    fn input(&mut self, event: &SurfaceInputEvent) {
        self.input.handle(event);
    }

    fn ime_caret(&self) -> Option<kurbo::Rect> {
        // Chromium places its own candidate window relative to the composition
        // it is rendering, so the host has no caret to report on its behalf.
        self.view.ime_caret()
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.view.measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.view.stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.view.priority()
    }
}

/// Chromium's OSR input ABI models three buttons; the W3C vocabulary has five.
const fn cef_pointer_button(button: SurfacePointerButton) -> Option<CefPointerButton> {
    match button {
        SurfacePointerButton::Primary => Some(CefPointerButton::Primary),
        SurfacePointerButton::Middle => Some(CefPointerButton::Middle),
        SurfacePointerButton::Secondary => Some(CefPointerButton::Secondary),
        SurfacePointerButton::Back | SurfacePointerButton::Forward => None,
    }
}

/// The UTF-16 offset CEF's composition selection is expressed in.
///
/// [`SurfaceInputEvent::CompositionUpdate`] reports the caret in bytes, the way
/// every Rust producer of it has it; CEF counts UTF-16 code units, the way
/// Chromium's editor does. A caret the platform did not report sits at the end.
///
/// # Panics
///
/// Panics when `caret` is not on a character boundary of `text`, or the
/// composition is longer than `u32` UTF-16 code units.
fn composition_selection(text: &str, caret: Option<usize>) -> u32 {
    let caret = caret.unwrap_or(text.len());
    assert!(
        text.is_char_boundary(caret),
        "CEF composition caret {caret} is not a character boundary of {text:?}"
    );
    u32::try_from(text[..caret].encode_utf16().count())
        .expect("CEF composition caret exceeds u32 UTF-16 code units")
}

/// The one character CEF's key event can carry, when there is exactly one.
///
/// `CefKeyEvent::character` is a single UTF-16 code unit, so anything outside
/// the basic multilingual plane — every emoji — has to travel as an edit
/// instead of a keystroke.
fn single_cef_character(text: &str) -> Option<char> {
    let mut characters = text.chars();
    let character = characters.next()?;
    (characters.next().is_none() && character.len_utf16() == 1).then_some(character)
}

/// The character a key types, as Chromium's editor expects to receive it.
///
/// The editing keys carry their control character, which is what makes CEF
/// deliver a `char` event for them at all.
fn key_character(key: &Key) -> Option<char> {
    match key {
        Key::Character(value) => single_cef_character(value),
        Key::Named(NamedKey::Backspace) => Some('\u{7f}'),
        Key::Named(NamedKey::Tab) => Some('\t'),
        Key::Named(NamedKey::Enter) => Some('\r'),
        Key::Named(NamedKey::Escape) => Some('\u{1b}'),
        Key::Named(_) => None,
    }
}

/// The Windows virtual key Chromium identifies a logical key by.
///
/// Chromium's key handling is written against Windows virtual keys on every
/// platform — `ui::KeyboardCode` *is* the VK table — so this is what CEF's
/// `windows_key_code` wants everywhere it is read. Space needs no entry: the
/// W3C vocabulary has no named space key, and the character it types is
/// already `VK_SPACE`.
fn windows_virtual_key(key: &Key) -> u32 {
    match key {
        Key::Character(value) => value
            .chars()
            .next()
            .map_or(0, |character| character.to_ascii_uppercase().into()),
        Key::Named(named) => named_virtual_key(*named),
    }
}

const fn named_virtual_key(key: NamedKey) -> u32 {
    match key {
        NamedKey::Backspace => 0x08,
        NamedKey::Tab => 0x09,
        NamedKey::Enter => 0x0d,
        NamedKey::Shift => 0x10,
        NamedKey::Control => 0x11,
        NamedKey::Alt => 0x12,
        NamedKey::Escape => 0x1b,
        NamedKey::PageUp => 0x21,
        NamedKey::PageDown => 0x22,
        NamedKey::End => 0x23,
        NamedKey::Home => 0x24,
        NamedKey::ArrowLeft => 0x25,
        NamedKey::ArrowUp => 0x26,
        NamedKey::ArrowRight => 0x27,
        NamedKey::ArrowDown => 0x28,
        NamedKey::Insert => 0x2d,
        NamedKey::Delete => 0x2e,
        NamedKey::F1 => 0x70,
        NamedKey::F2 => 0x71,
        NamedKey::F3 => 0x72,
        NamedKey::F4 => 0x73,
        NamedKey::F5 => 0x74,
        NamedKey::F6 => 0x75,
        NamedKey::F7 => 0x76,
        NamedKey::F8 => 0x77,
        NamedKey::F9 => 0x78,
        NamedKey::F10 => 0x79,
        NamedKey::F11 => 0x7a,
        NamedKey::F12 => 0x7b,
        _ => 0,
    }
}

/// The platform hardware code Chromium expects for a physical key.
///
/// macOS is where this matters most: CEF rebuilds an `NSEvent` from the key
/// event, so `native_key_code` — not `windows_key_code`, which the macOS path
/// discards — is what identifies the key. The table is Chromium's own
/// `keycode_converter_data.inc`, by way of the `keycode` crate, so the value is
/// the one the browser process would have computed for itself.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn native_key_code(code: Code) -> u32 {
    let Ok(mapping) = keycode::KeyMappingCode::from_str(&code.to_string()) else {
        return 0;
    };
    let map = keycode::KeyMap::from(mapping);
    #[cfg(target_os = "macos")]
    let native = map.mac;
    #[cfg(target_os = "linux")]
    let native = map.xkb;
    if native == KEYCODE_UNMAPPED {
        0
    } else {
        u32::from(native)
    }
}

/// Windows identifies the key by `windows_key_code`; the native code there is a
/// `WM_KEYDOWN` `lParam`, which a windowless surface has none of.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
const fn native_key_code(_code: Code) -> u32 {
    0
}

/// The macOS editing shortcuts Chromium expects the embedder to perform.
///
/// A windowless browser has no menu bar, so ⌘Z/⌘X/⌘C/⌘V/⌘A reach the page as
/// ordinary keystrokes and nothing happens. `AppKit` applications answer them by
/// invoking the corresponding editing command, which is what these do.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacEditShortcut {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

#[cfg(target_os = "macos")]
impl MacEditShortcut {
    fn from_input(key: &Key, modifiers: CefInputModifiers) -> Option<Self> {
        if !modifiers.command || modifiers.control || modifiers.alt {
            return None;
        }
        let Key::Character(value) = key else {
            return None;
        };
        let character = single_cef_character(value)?.to_ascii_lowercase();
        Some(match (character, modifiers.shift) {
            ('z', false) => Self::Undo,
            ('z', true) => Self::Redo,
            ('x', false) => Self::Cut,
            ('c', false) => Self::Copy,
            ('v', false) => Self::Paste,
            ('a', false) => Self::SelectAll,
            _ => return None,
        })
    }

    fn execute(self, page: &CefPageHandle) {
        match self {
            Self::Undo => page.undo(),
            Self::Redo => page.redo(),
            Self::Cut => page.cut(),
            Self::Copy => page.copy(),
            Self::Paste => page.paste(),
            Self::SelectAll => page.select_all(),
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::{CefInputModifiers, MacEditShortcut};
    use super::{
        Code, Key, NamedKey, composition_selection, key_character, native_key_code,
        single_cef_character, windows_virtual_key,
    };

    #[test]
    fn character_keys_identify_themselves_by_their_uppercase_virtual_key() {
        assert_eq!(
            windows_virtual_key(&Key::Character("a".into())),
            u32::from('A')
        );
        assert_eq!(
            windows_virtual_key(&Key::Character(" ".into())),
            u32::from(' ')
        );
        assert_eq!(windows_virtual_key(&Key::Named(NamedKey::ArrowLeft)), 0x25);
        assert_eq!(windows_virtual_key(&Key::Named(NamedKey::BrowserSearch)), 0);
    }

    #[test]
    fn only_one_bmp_character_uses_the_key_character_path() {
        assert_eq!(single_cef_character("W"), Some('W'));
        assert_eq!(single_cef_character(""), None);
        assert_eq!(single_cef_character("UI"), None);
        assert_eq!(single_cef_character("🚀"), None);
    }

    #[test]
    fn editing_keys_preserve_their_character_payloads() {
        assert_eq!(key_character(&Key::Character("a".into())), Some('a'));
        assert_eq!(
            key_character(&Key::Named(NamedKey::Backspace)),
            Some('\u{7f}')
        );
        assert_eq!(key_character(&Key::Named(NamedKey::ArrowLeft)), None);
    }

    /// The W3C physical code resolves to the hardware code Chromium's own table
    /// gives it, and a key with no code on this platform reports none.
    #[test]
    fn physical_codes_resolve_to_chromium_hardware_codes() {
        #[cfg(target_os = "macos")]
        {
            assert_eq!(native_key_code(Code::KeyA), 0x00);
            assert_eq!(native_key_code(Code::Escape), 0x35);
            assert_eq!(native_key_code(Code::ArrowLeft), 0x7b);
            // `Fn` is `0xffff` in Chromium's table on every platform but macOS,
            // and unmapped there too.
            assert_eq!(native_key_code(Code::Lang1), 0);
        }
        #[cfg(target_os = "linux")]
        {
            assert_eq!(native_key_code(Code::KeyA), 0x26);
            assert_eq!(native_key_code(Code::Escape), 0x09);
            assert_eq!(native_key_code(Code::ArrowLeft), 0x71);
        }
        assert_eq!(native_key_code(Code::Unidentified), 0);
    }

    #[test]
    fn composition_carets_convert_from_bytes_to_utf16_code_units() {
        // Three characters, one of them outside the BMP: six bytes into "日本"
        // is two UTF-16 units, and the surrogate pair that follows is two more.
        assert_eq!(composition_selection("日本🚀", Some(6)), 2);
        assert_eq!(composition_selection("日本🚀", None), 4);
        assert_eq!(composition_selection("abc", Some(1)), 1);
    }

    #[test]
    #[should_panic(expected = "not a character boundary")]
    fn a_composition_caret_inside_a_character_is_a_bug_in_the_backend() {
        let _ = composition_selection("日本", Some(1));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_standard_edit_shortcuts_map_to_cef_frame_commands() {
        let command = CefInputModifiers {
            command: true,
            ..Default::default()
        };
        let shifted_command = CefInputModifiers {
            shift: true,
            ..command
        };

        assert_eq!(
            MacEditShortcut::from_input(&Key::Character("a".into()), command),
            Some(MacEditShortcut::SelectAll)
        );
        assert_eq!(
            MacEditShortcut::from_input(&Key::Character("c".into()), command),
            Some(MacEditShortcut::Copy)
        );
        assert_eq!(
            MacEditShortcut::from_input(&Key::Character("x".into()), command),
            Some(MacEditShortcut::Cut)
        );
        assert_eq!(
            MacEditShortcut::from_input(&Key::Character("v".into()), command),
            Some(MacEditShortcut::Paste)
        );
        assert_eq!(
            MacEditShortcut::from_input(&Key::Character("z".into()), command),
            Some(MacEditShortcut::Undo)
        );
        assert_eq!(
            MacEditShortcut::from_input(&Key::Character("Z".into()), shifted_command),
            Some(MacEditShortcut::Redo)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_edit_shortcuts_reject_nonstandard_modifier_combinations() {
        let command_control = CefInputModifiers {
            command: true,
            control: true,
            ..Default::default()
        };
        let shifted_command = CefInputModifiers {
            command: true,
            shift: true,
            ..Default::default()
        };

        assert_eq!(
            MacEditShortcut::from_input(&Key::Character("a".into()), command_control),
            None
        );
        assert_eq!(
            MacEditShortcut::from_input(&Key::Character("a".into()), shifted_command),
            None
        );
        assert_eq!(
            MacEditShortcut::from_input(&Key::Character("a".into()), CefInputModifiers::default()),
            None
        );
    }
}
