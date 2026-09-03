//! WPE input adaptation, expressed against the backend-neutral surface
//! vocabulary.
//!
//! Every backend that embeds a WPE page used to re-derive the same handful of
//! `WPEPlatform` facts for itself: that `WPEModifiers` packs the keyboard chord
//! into bits 0-4 and the pressed buttons into bits 8-12 and wants both halves
//! on every event, that a keyboard event is a raw XKB pair — an evdev-derived
//! hardware keycode and an X11 keysym — rather than anything the W3C vocabulary
//! names, that a scroll is "precise" exactly when its deltas are pixels, and
//! that committed text has to be typed back as synthetic keystrokes because the
//! bridge has no insertion call. Two copies of that knowledge existed and they
//! had already drifted.
//!
//! [`WpeSurfaceInput`] is the single copy. A backend translates its own
//! platform events into [`SurfaceInputEvent`] — which it must do anyway, for
//! every other interactive GPU surface — and hands them here.
//!
//! # Timestamps
//!
//! `WPEPlatform` timestamps every event, and `wpe_view_compute_press_count`
//! reads them to turn two clicks into a double click. The surface vocabulary
//! carries no timestamp, so the adapter keeps its own monotonic millisecond
//! clock from the moment it is created. Only the differences matter to WPE, so
//! a backend that has real platform event times loses nothing by not being able
//! to pass them.
//!
//! # Input methods
//!
//! The bundled bridge ABI (`native/waterui_wpe.h`) exposes focus, pointer,
//! scroll and key events and nothing else: there is no call for pre-edit text.
//! `WPEPlatform` itself has `WPEInputMethodContext`, but reaching it means
//! extending the C bridge and rebuilding the staged runtime, so a composition
//! reaches the page only when it commits — as the keystrokes that would have
//! typed it. Pre-edit is therefore invisible in an embedded WPE page, and this
//! adapter says so rather than pretending otherwise.

use std::str::FromStr as _;
use std::time::Instant;

use waterui_core::Environment;
use waterui_core::layout::{ProposalSize, StretchAxis, ViewDimensions};
use waterui_graphics::gpu_surface::{GpuContext, GpuFrame, GpuView};
use waterui_graphics::input::{
    Code, Key, Modifiers, NamedKey, ScrollUnit, SurfaceInputEvent, SurfacePointerButton,
};
use xkeysym::{Keysym, key as xkb};

use crate::page::{PointerButton, WpePage};

/// The keyboard half of `WPEModifiers`.
const WPE_MODIFIER_CONTROL: u32 = 1 << 0;
const WPE_MODIFIER_SHIFT: u32 = 1 << 1;
const WPE_MODIFIER_ALT: u32 = 1 << 2;
const WPE_MODIFIER_META: u32 = 1 << 3;
const WPE_MODIFIER_CAPS_LOCK: u32 = 1 << 4;

/// The pressed-button half of `WPEModifiers`, one bit per `WPEPlatform` button
/// number.
const WPE_BUTTON_PRIMARY: u32 = 1 << 8;
const WPE_BUTTON_MIDDLE: u32 = 1 << 9;
const WPE_BUTTON_SECONDARY: u32 = 1 << 10;
const WPE_BUTTON_BACK: u32 = 1 << 11;
const WPE_BUTTON_FORWARD: u32 = 1 << 12;

/// Chromium's keycode table — the one the `keycode` crate is generated from —
/// marks "this key has no code on that platform" with this sentinel rather than
/// omitting the row.
const KEYCODE_UNMAPPED: u16 = 0xffff;

/// Drives one WPE page from the backend-neutral surface input vocabulary.
///
/// Holds the state `WPEPlatform`'s event constructors need but the neutral
/// vocabulary does not carry in its events: the modifier chord, which pointer
/// buttons are down, where the pointer was last seen, and the event clock.
///
/// Positions are logical and surface-local, exactly as [`SurfaceInputEvent`]
/// defines them: the page's own top-left is `(0, 0)`.
#[derive(Debug)]
pub struct WpeSurfaceInput {
    page: WpePage,
    /// The keyboard half of the modifier word, replaced whenever the chord
    /// changes.
    chord: u32,
    /// The pressed-button half, accumulated across button events. WPE wants
    /// both halves in one field on every event it receives.
    buttons: u32,
    /// Where the pointer was last seen, because a `WPEPlatform` move event
    /// carries the movement since the previous one and the neutral vocabulary
    /// reports positions only.
    last_pointer: Option<kurbo::Point>,
    clock: Instant,
}

impl WpeSurfaceInput {
    /// Creates an input adapter for one WPE page.
    #[must_use]
    pub fn new(page: WpePage) -> Self {
        Self {
            page,
            chord: 0,
            buttons: 0,
            last_pointer: None,
            clock: Instant::now(),
        }
    }

    /// The page this adapter drives.
    #[must_use]
    pub const fn page(&self) -> &WpePage {
        &self.page
    }

    /// Applies one input event to the page.
    pub fn handle(&mut self, event: &SurfaceInputEvent) {
        match event {
            SurfaceInputEvent::Focus(focused) => self.page.set_focus(*focused),
            SurfaceInputEvent::Modifiers(modifiers) => self.chord = wpe_modifiers(*modifiers),
            SurfaceInputEvent::PointerMove { position } => self.pointer_move(*position),
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
                finished,
            } => self.page.scroll(
                position.x,
                position.y,
                *delta_x,
                *delta_y,
                // "Precise" is WPE's word for pixel deltas: a discrete wheel
                // notch is a step, and WebKit multiplies it by its own line
                // height.
                matches!(unit, ScrollUnit::Pixel),
                *finished,
                self.modifiers(),
                self.time_ms(),
            ),
            SurfaceInputEvent::Key {
                pressed,
                key,
                code,
                modifiers,
                ..
            } => {
                self.chord = wpe_modifiers(*modifiers);
                self.page.key(
                    *pressed,
                    xkb_keycode(*code),
                    keysym(key, *code),
                    self.modifiers(),
                    self.time_ms(),
                );
            }
            // None of these crosses the bridge. The key event above already
            // carried the keysym that types committed text and WebKit's own key
            // handling inserts it, so forwarding `TextInput` as well would type
            // every character twice; and the bridge ABI has no pre-edit call at
            // all (see the module docs), so a composition session stays
            // invisible until it commits.
            SurfaceInputEvent::TextInput(_)
            | SurfaceInputEvent::CompositionStart
            | SurfaceInputEvent::CompositionUpdate { .. }
            | SurfaceInputEvent::CompositionCancel => {}
            SurfaceInputEvent::CompositionCommit(text) => self.commit_text(text),
        }
    }

    /// Both halves of the modifier word, which is how WPE wants it.
    const fn modifiers(&self) -> u32 {
        self.chord | self.buttons
    }

    /// Milliseconds since this adapter was created, wrapped into WPE's `u32`
    /// event timestamp.
    ///
    /// # Panics
    ///
    /// Never: the modulo is taken against the width of the target type.
    fn time_ms(&self) -> u32 {
        let range = u128::from(u32::MAX) + 1;
        u32::try_from(self.clock.elapsed().as_millis() % range)
            .expect("WPE timestamp modulo u32 must fit")
    }

    fn pointer_move(&mut self, position: kurbo::Point) {
        let previous = self.last_pointer.replace(position);
        let (delta_x, delta_y) = previous.map_or((0.0, 0.0), |previous| {
            (position.x - previous.x, position.y - previous.y)
        });
        self.page.pointer_move(
            position.x,
            position.y,
            delta_x,
            delta_y,
            self.modifiers(),
            self.time_ms(),
        );
    }

    fn pointer_button(&mut self, pressed: bool, button: SurfacePointerButton, x: f64, y: f64) {
        let (button, mask) = wpe_pointer_button(button);
        if pressed {
            self.buttons |= mask;
        } else {
            self.buttons &= !mask;
        }
        self.page
            .pointer_button(pressed, button, x, y, self.modifiers(), self.time_ms());
    }

    /// Types committed text as the keystrokes that would have produced it.
    ///
    /// The bridge has no text-insertion call, so each character is sent as its
    /// own keysym with no hardware keycode — there is no physical key behind a
    /// character an input method composed.
    fn commit_text(&self, text: &str) {
        for character in text.chars() {
            let keyval = Keysym::from_char(character).raw();
            self.page
                .key(true, 0, keyval, self.modifiers(), self.time_ms());
            self.page
                .key(false, 0, keyval, self.modifiers(), self.time_ms());
        }
    }
}

/// A WPE presenter that also consumes the input landing on its surface.
///
/// The presenter and the input adapter are separate concerns — one composites
/// the dma-buf stream, the other owns `WPEPlatform`'s event ABI — but a backend
/// that routes input to GPU views by
/// [`wants_input_events`](GpuView::wants_input_events) needs them as one
/// object. See [`gpu_view_with_input`](crate::gpu_view_with_input).
pub struct WpeInputGpuView<V> {
    view: V,
    input: WpeSurfaceInput,
}

impl<V> core::fmt::Debug for WpeInputGpuView<V> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("WpeInputGpuView")
            .field("input", &self.input)
            .finish_non_exhaustive()
    }
}

impl<V> WpeInputGpuView<V> {
    /// Pairs a presenter with the adapter that feeds its page.
    pub const fn new(view: V, input: WpeSurfaceInput) -> Self {
        Self { view, input }
    }
}

impl<V: GpuView> GpuView for WpeInputGpuView<V> {
    #[expect(
        clippy::future_not_send,
        reason = "WPE pages and WaterUI view state are confined to the UI thread"
    )]
    async fn setup(&mut self, ctx: &GpuContext<'_>, env: &mut Environment) {
        self.view.setup(ctx, env).await;
    }

    fn render(&mut self, frame: &mut GpuFrame<'_>) {
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
        // Wrapping a presenter must not take its caret away, even though no WPE
        // presenter reports one today: the bridge exposes no input-method
        // context, so the page's caret never crosses it.
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

/// The `WPEModifiers` word for a keyboard chord.
const fn wpe_modifiers(modifiers: Modifiers) -> u32 {
    let mut value = 0;
    if modifiers.contains(Modifiers::CONTROL) {
        value |= WPE_MODIFIER_CONTROL;
    }
    if modifiers.contains(Modifiers::SHIFT) {
        value |= WPE_MODIFIER_SHIFT;
    }
    if modifiers.contains(Modifiers::ALT) {
        value |= WPE_MODIFIER_ALT;
    }
    if modifiers.contains(Modifiers::META) {
        value |= WPE_MODIFIER_META;
    }
    if modifiers.contains(Modifiers::CAPS_LOCK) {
        value |= WPE_MODIFIER_CAPS_LOCK;
    }
    value
}

/// `WPEPlatform`'s button number and the modifier bit that says it is held.
///
/// Unlike Chromium's windowless ABI, `WPEPlatform` models all five W3C buttons,
/// so nothing has to be synthesised as a navigation gesture.
const fn wpe_pointer_button(button: SurfacePointerButton) -> (PointerButton, u32) {
    match button {
        SurfacePointerButton::Primary => (PointerButton::Primary, WPE_BUTTON_PRIMARY),
        SurfacePointerButton::Middle => (PointerButton::Middle, WPE_BUTTON_MIDDLE),
        SurfacePointerButton::Secondary => (PointerButton::Secondary, WPE_BUTTON_SECONDARY),
        SurfacePointerButton::Back => (PointerButton::Back, WPE_BUTTON_BACK),
        SurfacePointerButton::Forward => (PointerButton::Forward, WPE_BUTTON_FORWARD),
    }
}

/// The XKB hardware keycode for a physical key.
///
/// `WPEPlatform` passes the keycode straight through to `WebKit`, which reads it
/// the way X11 numbers keys — the evdev scancode plus eight. The table is
/// Chromium's own `keycode_converter_data.inc`, by way of the `keycode` crate,
/// rather than a hand-written one; a key with no XKB code reports zero, which
/// is what the synthetic keystrokes of a composition send too.
fn xkb_keycode(code: Code) -> u32 {
    let Ok(mapping) = keycode::KeyMappingCode::from_str(&code.to_string()) else {
        return 0;
    };
    let xkb = keycode::KeyMap::from(mapping).xkb;
    if xkb == KEYCODE_UNMAPPED {
        0
    } else {
        u32::from(xkb)
    }
}

/// The X11 keysym for a logical key.
///
/// This is the value `WebKit` turns back into text, so a key that types
/// something is its character's keysym and everything else is named. The
/// physical key resolves the handedness the W3C logical key drops: `Shift` is
/// `Shift_L` or `Shift_R` depending on which one was pressed.
fn keysym(key: &Key, code: Code) -> u32 {
    match key {
        Key::Character(value) => value.chars().next().map_or(xkb::NoSymbol, |character| {
            Keysym::from_char(character).raw()
        }),
        Key::Named(named) => named_keysym(*named, code),
    }
}

/// The X11 keysym for a W3C named key.
///
/// A key with no keysym reports `NoSymbol`, which `WebKit` ignores; the
/// alternative — inventing one — would type something the user did not press.
const fn named_keysym(key: NamedKey, code: Code) -> u32 {
    match key {
        NamedKey::Shift => handed(code, xkb::Shift_L, xkb::Shift_R),
        NamedKey::Control => handed(code, xkb::Control_L, xkb::Control_R),
        NamedKey::Alt => handed(code, xkb::Alt_L, xkb::Alt_R),
        NamedKey::Meta => handed(code, xkb::Super_L, xkb::Super_R),
        NamedKey::AltGraph => xkb::ISO_Level3_Shift,
        NamedKey::Backspace => xkb::BackSpace,
        NamedKey::Tab => xkb::Tab,
        NamedKey::Enter => xkb::Return,
        NamedKey::Escape => xkb::Escape,
        NamedKey::Home => xkb::Home,
        NamedKey::End => xkb::End,
        NamedKey::PageUp => xkb::Page_Up,
        NamedKey::PageDown => xkb::Page_Down,
        NamedKey::ArrowLeft => xkb::Left,
        NamedKey::ArrowUp => xkb::Up,
        NamedKey::ArrowRight => xkb::Right,
        NamedKey::ArrowDown => xkb::Down,
        NamedKey::Insert => xkb::Insert,
        NamedKey::Delete => xkb::Delete,
        NamedKey::CapsLock => xkb::Caps_Lock,
        NamedKey::NumLock => xkb::Num_Lock,
        NamedKey::ScrollLock => xkb::Scroll_Lock,
        NamedKey::PrintScreen => xkb::Print,
        NamedKey::Pause => xkb::Pause,
        NamedKey::ContextMenu => xkb::Menu,
        NamedKey::F1 => xkb::F1,
        NamedKey::F2 => xkb::F2,
        NamedKey::F3 => xkb::F3,
        NamedKey::F4 => xkb::F4,
        NamedKey::F5 => xkb::F5,
        NamedKey::F6 => xkb::F6,
        NamedKey::F7 => xkb::F7,
        NamedKey::F8 => xkb::F8,
        NamedKey::F9 => xkb::F9,
        NamedKey::F10 => xkb::F10,
        NamedKey::F11 => xkb::F11,
        NamedKey::F12 => xkb::F12,
        _ => xkb::NoSymbol,
    }
}

/// Which side of the keyboard a modifier was pressed on.
///
/// The W3C logical key for a modifier is sideless, so the physical key is the
/// only thing that can say; a code that names neither side is the left one,
/// which is where every layout puts the primary modifier.
const fn handed(code: Code, left: u32, right: u32) -> u32 {
    match code {
        Code::ShiftRight | Code::ControlRight | Code::AltRight | Code::MetaRight => right,
        _ => left,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Code, Key, Modifiers, NamedKey, SurfacePointerButton, keysym, named_keysym, wpe_modifiers,
        wpe_pointer_button, xkb_keycode,
    };
    use crate::page::PointerButton;

    #[test]
    fn the_modifier_word_packs_the_keyboard_chord_into_its_low_bits() {
        assert_eq!(wpe_modifiers(Modifiers::empty()), 0);
        assert_eq!(wpe_modifiers(Modifiers::CONTROL), 1 << 0);
        assert_eq!(wpe_modifiers(Modifiers::SHIFT), 1 << 1);
        assert_eq!(wpe_modifiers(Modifiers::ALT), 1 << 2);
        assert_eq!(wpe_modifiers(Modifiers::META), 1 << 3);
        assert_eq!(wpe_modifiers(Modifiers::CAPS_LOCK), 1 << 4);
        assert_eq!(
            wpe_modifiers(Modifiers::CONTROL | Modifiers::SHIFT),
            (1 << 0) | (1 << 1)
        );
        // A modifier WPE has no bit for changes nothing.
        assert_eq!(wpe_modifiers(Modifiers::FN), 0);
    }

    /// `WPEPlatform` numbers its five buttons 1-5 and holds them in bits 8-12,
    /// so the button number and its modifier bit have to agree.
    #[test]
    fn every_w3c_button_has_a_wpe_button_and_a_matching_modifier_bit() {
        for (button, expected, bit) in [
            (SurfacePointerButton::Primary, PointerButton::Primary, 8),
            (SurfacePointerButton::Middle, PointerButton::Middle, 9),
            (
                SurfacePointerButton::Secondary,
                PointerButton::Secondary,
                10,
            ),
            (SurfacePointerButton::Back, PointerButton::Back, 11),
            (SurfacePointerButton::Forward, PointerButton::Forward, 12),
        ] {
            let (button, mask) = wpe_pointer_button(button);
            assert_eq!(button, expected);
            assert_eq!(mask, 1 << bit);
            assert_eq!(button as u32, bit - 7);
        }
    }

    #[test]
    fn character_keys_resolve_to_their_own_keysyms() {
        assert_eq!(keysym(&Key::Character("a".into()), Code::KeyA), 0x0061);
        assert_eq!(keysym(&Key::Character("A".into()), Code::KeyA), 0x0041);
        assert_eq!(keysym(&Key::Character(" ".into()), Code::Space), 0x0020);
        // An empty logical key types nothing, and WebKit ignores `NoSymbol`.
        assert_eq!(keysym(&Key::Character(String::new()), Code::KeyA), 0);
    }

    #[test]
    fn named_keys_resolve_to_their_x11_keysyms() {
        assert_eq!(named_keysym(NamedKey::Backspace, Code::Backspace), 0xff08);
        assert_eq!(named_keysym(NamedKey::Enter, Code::Enter), 0xff0d);
        assert_eq!(named_keysym(NamedKey::ArrowLeft, Code::ArrowLeft), 0xff51);
        assert_eq!(named_keysym(NamedKey::F1, Code::F1), 0xffbe);
        // A named key with no X11 spelling reports nothing rather than
        // something the user did not press.
        assert_eq!(named_keysym(NamedKey::BrowserSearch, Code::Unidentified), 0);
    }

    /// The W3C logical modifier key is sideless; the physical code is what says
    /// which one the user actually pressed.
    #[test]
    fn modifier_keysyms_take_their_side_from_the_physical_key() {
        assert_eq!(named_keysym(NamedKey::Shift, Code::ShiftLeft), 0xffe1);
        assert_eq!(named_keysym(NamedKey::Shift, Code::ShiftRight), 0xffe2);
        assert_eq!(named_keysym(NamedKey::Control, Code::ControlLeft), 0xffe3);
        assert_eq!(named_keysym(NamedKey::Control, Code::ControlRight), 0xffe4);
        assert_eq!(named_keysym(NamedKey::Alt, Code::AltLeft), 0xffe9);
        assert_eq!(named_keysym(NamedKey::Alt, Code::AltRight), 0xffea);
    }

    /// XKB numbers a key eight higher than evdev does, which is what `WebKit`
    /// reads on the other side of the bridge.
    #[test]
    fn physical_codes_resolve_to_xkb_hardware_keycodes() {
        assert_eq!(xkb_keycode(Code::KeyA), 0x26);
        assert_eq!(xkb_keycode(Code::Escape), 0x09);
        assert_eq!(xkb_keycode(Code::ArrowLeft), 0x71);
        assert_eq!(xkb_keycode(Code::Unidentified), 0);
    }
}
