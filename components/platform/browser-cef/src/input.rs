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
//! A press that produces text is followed by *its*
//! [`SurfaceInputEvent::TextInput`] — pressed key, then text, then the
//! release — the order the web platform gives `keydown` and `beforeinput`
//! and the only order the surface vocabulary takes. The adapter sends the
//! `RAWKEYDOWN` at press time and the character when the text lands: a
//! `CHAR` event for the single BMP character CEF's key ABI carries,
//! `commit_text` for the run it cannot hold (an emoji, a ligature). The
//! committed text still wins over the character the logical key implies —
//! it is the authority on what a dead key or an accented layout typed.
//!
//! A backend that emits no text at all (GTK) is equally well served: the
//! logical key carries the character. "Is text coming" needs no timer —
//! every backend delivers its events down one ordered queue, so the *next
//! event of any kind* after a press is the answer: the press produced none,
//! and its logical character goes out then.

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
/// wheel remainder, and the press→text pairing (see the module docs).
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
    pairing: KeyPairing,
}

impl CefSurfaceInput {
    /// Creates an input adapter for one CEF page.
    #[must_use]
    pub fn new(page: CefPageHandle) -> Self {
        Self {
            page,
            modifiers: CefInputModifiers::default(),
            wheel_remainder: (0.0, 0.0),
            pairing: KeyPairing::default(),
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
    /// Panics when a composition caret is not on a character boundary of its
    /// text.
    pub fn handle(&mut self, event: &SurfaceInputEvent) {
        // Any event that is not text or a key answers "the press produced
        // no text" for an owed press — the owed character goes out first
        // (see the module docs for why no timer is needed).
        if !matches!(
            event,
            SurfaceInputEvent::Key { .. } | SurfaceInputEvent::TextInput(_)
        ) {
            let calls = self.pairing.flush();
            self.apply_key_calls(calls);
        }
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
                let calls = self.pairing.text(text);
                self.apply_key_calls(calls);
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

    /// Plays the calls the press→text pairing resolved against the page.
    fn apply_key_calls(&self, calls: Vec<KeyCall>) {
        for call in calls {
            match call {
                KeyCall::Transition {
                    pressed,
                    input,
                    modifiers,
                } => self.page.key(pressed, input, modifiers),
                KeyCall::Char {
                    input,
                    character,
                    modifiers,
                } => self.page.key_char(input, character, modifiers),
                KeyCall::Commit { text } => self.page.commit_text(&text, None),
            }
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
        let calls = self.pairing.key(pressed, key, code, self.modifiers);
        self.apply_key_calls(calls);
        #[cfg(target_os = "macos")]
        if pressed && let Some(command) = MacEditShortcut::from_input(key, self.modifiers) {
            command.execute(&self.page);
        }
    }
}

/// The press→text pairing as a pure event transform.
///
/// A `CefPageHandle` cannot be constructed without a running browser, so the
/// pairing emits the ABI calls it decides on as [`KeyCall`] data and
/// [`CefSurfaceInput::apply_key_calls`] plays them — a test can drive the
/// exact [`SurfaceInputEvent`] stream a backend produces and read the ABI
/// back.
#[derive(Debug, Default)]
struct KeyPairing {
    /// The most recent press still owed a character: its `RAWKEYDOWN` went
    /// out at press time; the `CHAR` rides the press's own `TextInput`, or
    /// the logical key's character goes out when the next event of any kind
    /// arrives without one — the only distinction "text is coming" from
    /// "no text is coming" a shared ordered queue offers.
    owed: Option<OwedChar>,
}

/// A press whose `RAWKEYDOWN` went out with the character still owed.
#[derive(Debug)]
struct OwedChar {
    /// The press's keycodes — the `CHAR` carries them so CEF correlates it
    /// with the same physical key.
    input: CefKeyInput,
    /// The logical key — the fallback character when the backend reports no
    /// text for the press at all (GTK emits key events only).
    key: Key,
    /// The chord held at press time: the `CHAR` reflects what was held then,
    /// not whatever a later event moved the chord to.
    modifiers: CefInputModifiers,
}

/// One ABI call the pairing asks the page to make.
#[derive(Debug, PartialEq)]
enum KeyCall {
    /// A key transition carrying no character — `RAWKEYDOWN` on press,
    /// `KEYUP` on release.
    Transition {
        /// `true` for a press, `false` for a release.
        pressed: bool,
        /// The key's CEF metadata, always without a character field.
        input: CefKeyInput,
        /// The modifier word to send with the event.
        modifiers: CefInputModifiers,
    },
    /// The `CHAR` for a press — its paired `TextInput` when one arrived, or
    /// the character the logical key implies when the next event arrived
    /// first (the no-text backend case).
    Char {
        /// The owed press's keycodes.
        input: CefKeyInput,
        /// The single BMP character to type.
        character: char,
        /// The modifier word frozen at press time.
        modifiers: CefInputModifiers,
    },
    /// `commit_text` for what a single `CHAR` cannot carry — a multi-unit
    /// or supplementary-plane run — or for text that arrived with no press
    /// behind it.
    Commit {
        /// The text to insert.
        text: String,
    },
}

impl KeyPairing {
    /// Ends the owed-press wait without pairing: the press produced no
    /// text, so its logical key's own character goes out now.
    ///
    /// Returns nothing for an owed press whose logical key types nothing
    /// (a modifier, a dead key): such a press is a `RAWKEYDOWN`/`KEYUP`
    /// pair with no `CHAR`, exactly what the platform reported.
    fn flush(&mut self) -> Vec<KeyCall> {
        let Some(owed) = self.owed.take() else {
            return Vec::new();
        };
        key_character(&owed.key)
            .map(|character| KeyCall::Char {
                input: owed.input,
                character,
                modifiers: owed.modifiers,
            })
            .into_iter()
            .collect()
    }

    /// The calls a key transition resolves to. A press flushes the previous
    /// owed press first — two presses with no text between them mean the
    /// first produced none — then goes out as `RAWKEYDOWN` and becomes the
    /// owed one. A release flushes and goes out as `KEYUP`.
    fn key(
        &mut self,
        pressed: bool,
        key: &Key,
        code: Code,
        modifiers: CefInputModifiers,
    ) -> Vec<KeyCall> {
        let mut calls = self.flush();
        let input = CefKeyInput {
            native_keycode: native_key_code(code),
            keyval: windows_virtual_key(key),
            character: None,
        };
        calls.push(KeyCall::Transition {
            pressed,
            input,
            modifiers,
        });
        if pressed {
            self.owed = Some(OwedChar {
                input,
                key: key.clone(),
                modifiers,
            });
        }
        calls
    }

    /// The calls a `TextInput` resolves to. Following a press it is that
    /// press's text — a `CHAR` when it is one BMP character, a commit when
    /// it is not. With no owed press it is standalone text and commits
    /// verbatim.
    fn text(&mut self, text: &str) -> Vec<KeyCall> {
        match self.owed.take() {
            Some(owed) => vec![match single_cef_character(text) {
                Some(character) => KeyCall::Char {
                    input: owed.input,
                    character,
                    modifiers: owed.modifiers,
                },
                None => KeyCall::Commit {
                    text: text.to_owned(),
                },
            }],
            None => vec![KeyCall::Commit {
                text: text.to_owned(),
            }],
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

    fn is_opaque(&self) -> bool {
        self.view.is_opaque()
    }

    fn wants_input_events(&self) -> bool {
        true
    }

    fn input(&mut self, event: &SurfaceInputEvent) {
        self.input.handle(event);
    }

    fn ime_caret(&self) -> Option<kurbo::Rect> {
        // Wrapping a presenter must not take its caret away, even though no CEF
        // presenter reports one today: Chromium knows where the composition is
        // and the host would have to be told.
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
    use super::MacEditShortcut;
    use super::{
        CefInputModifiers, Code, Key, KeyCall, KeyPairing, Modifiers, NamedKey, SurfaceInputEvent,
        composition_selection, key_character, native_key_code, single_cef_character,
        windows_virtual_key,
    };

    /// A `Key` event the way a backend reports it.
    fn key_event(pressed: bool, key: Key, code: Code) -> SurfaceInputEvent {
        SurfaceInputEvent::Key {
            pressed,
            key,
            code,
            modifiers: Modifiers::empty(),
            repeat: false,
        }
    }

    /// The calls a sequence of surface events resolves to — the test's view
    /// of the page.
    fn calls_for(events: &[SurfaceInputEvent]) -> Vec<KeyCall> {
        let modifiers = CefInputModifiers::default();
        let mut pairing = KeyPairing::default();
        events
            .iter()
            .flat_map(|event| match event {
                SurfaceInputEvent::Key {
                    pressed, key, code, ..
                } => pairing.key(*pressed, key, *code, modifiers),
                SurfaceInputEvent::TextInput(text) => pairing.text(text),
                _ => pairing.flush(),
            })
            .collect()
    }

    /// What the calls would insert — `CHAR` characters and commits, in
    /// order. This is the text the page ends up with.
    fn typed_text(calls: &[KeyCall]) -> String {
        calls
            .iter()
            .filter_map(|call| match call {
                KeyCall::Char { character, .. } => Some(character.to_string()),
                KeyCall::Commit { text } => Some(text.clone()),
                KeyCall::Transition { .. } => None,
            })
            .collect()
    }

    /// Press → text → release for `a` and `b` types "ab" — each `CHAR`
    /// rides the keycodes of the press its text belongs to, never the next
    /// key's.
    #[test]
    fn text_following_a_press_pairs_with_that_press() {
        let calls = calls_for(&[
            key_event(true, Key::Character("a".into()), Code::KeyA),
            SurfaceInputEvent::TextInput("a".into()),
            key_event(false, Key::Character("a".into()), Code::KeyA),
            key_event(true, Key::Character("b".into()), Code::KeyB),
            SurfaceInputEvent::TextInput("b".into()),
            key_event(false, Key::Character("b".into()), Code::KeyB),
        ]);
        assert_eq!(typed_text(&calls), "ab");
        let char_keyvals: Vec<u32> = calls
            .iter()
            .filter_map(|call| match call {
                KeyCall::Char { input, .. } => Some(input.keyval),
                _ => None,
            })
            .collect();
        assert_eq!(char_keyvals, [u32::from('A'), u32::from('B')]);
        // The character event follows its own press, not the next one.
        assert!(matches!(
            &calls[..3],
            [
                KeyCall::Transition { pressed: true, .. },
                KeyCall::Char {
                    character: 'a', ..
                },
                KeyCall::Transition { pressed: false, .. },
            ]
        ));
    }

    /// Dead-acute then `e`: the dead key types nothing itself, and the
    /// committed "é" — not the accent, not the `e` — is the text.
    #[test]
    fn a_dead_key_sequence_types_the_combined_character_once() {
        let calls = calls_for(&[
            key_event(true, Key::Named(NamedKey::Dead), Code::Unidentified),
            key_event(false, Key::Named(NamedKey::Dead), Code::Unidentified),
            key_event(true, Key::Character("e".into()), Code::KeyE),
            SurfaceInputEvent::TextInput("é".into()),
            key_event(false, Key::Character("e".into()), Code::KeyE),
        ]);
        assert_eq!(typed_text(&calls), "é");
        // The dead press is exactly a RAWKEYDOWN/KEYUP pair — no CHAR.
        assert_eq!(calls.len(), 5, "expected transitions + one CHAR: {calls:?}");
    }

    /// A backend that emits no text at all (GTK) still types the logical
    /// key's character — sent when the next event, here the release,
    /// arrives without a `TextInput`.
    #[test]
    fn a_press_with_no_text_types_the_logical_key_character() {
        let calls = calls_for(&[
            key_event(true, Key::Character("a".into()), Code::KeyA),
            key_event(false, Key::Character("a".into()), Code::KeyA),
        ]);
        assert_eq!(typed_text(&calls), "a");
        assert!(matches!(
            &calls[..],
            [
                KeyCall::Transition { pressed: true, .. },
                KeyCall::Char {
                    character: 'a', ..
                },
                KeyCall::Transition { pressed: false, .. },
            ]
        ));
    }

    /// Any event — not just a key — answers "no text is coming" for an
    /// owed press.
    #[test]
    fn any_event_flushes_an_owed_press() {
        let calls = calls_for(&[
            key_event(true, Key::Character("a".into()), Code::KeyA),
            SurfaceInputEvent::Focus(false),
        ]);
        assert_eq!(typed_text(&calls), "a");
    }

    /// Text a single `CHAR` cannot carry — supplementary-plane runs — goes
    /// to the page as a commit, paired to its press the same way.
    #[test]
    fn text_beyond_one_utf16_unit_commits_instead_of_charring() {
        let calls = calls_for(&[
            key_event(true, Key::Character(" ".into()), Code::Space),
            SurfaceInputEvent::TextInput("🚀".into()),
            key_event(false, Key::Character(" ".into()), Code::Space),
        ]);
        assert!(matches!(
            &calls[..],
            [
                KeyCall::Transition { pressed: true, .. },
                KeyCall::Commit { text },
                KeyCall::Transition { pressed: false, .. },
            ] if text == "🚀"
        ));
    }

    /// Text with no press behind it is an insertion, not a keystroke —
    /// committed verbatim.
    #[test]
    fn standalone_text_commits_verbatim() {
        let calls = calls_for(&[SurfaceInputEvent::TextInput("pasted".into())]);
        assert_eq!(
            calls.as_slice(),
            &[KeyCall::Commit {
                text: "pasted".to_owned()
            }]
        );
    }

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
