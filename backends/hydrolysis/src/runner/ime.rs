//! Ordered IME keystroke ownership, shared by the rendered and semantic
//! input dispatch loops.
//!
//! Platform IMEs mark their own keystrokes by what they emit, and
//! ownership follows the event order inside a batch. A `composing` flag —
//! seeded from the renderer's live composition — becomes true at a
//! non-empty preedit and false at a commit, an empty preedit, or
//! `ImeDisabled`; every key/text event is the IME's while `composing`
//! holds at that point in the sequence. The Enter or Backspace that
//! confirms a composition arrives before its commit — still inside it —
//! and is consumed, while a key arriving after the commit (a shortcut, or
//! plain typing in direct-commit mode) is ordinary input again.
//!
//! Ordering alone cannot place the keystroke that *produced* a
//! composition event, because platforms report it first: wl_keyboard
//! forwards the raw key — and IBus/fcitx5 the keysym-derived text — inside
//! the same flush as the `zwp_text_input_v3` preedit they generated, and
//! AppKit delivers the `keyDown` before the `insertText` it becomes. So
//! each IME event additionally claims its producer: walking backwards it
//! skips key releases, keeps that press's own `TextInput` events, and
//! claims the single nearest modifier-free press — never an earlier one.
//! A key a commit cannot be answering therefore stays ordinary even with
//! another IME event later in the batch.
//!
//! Two ordering details decide the ambiguous cases:
//!
//! - A commit with no live composition is ordinary text, not a composition
//!   boundary: fcitx5/IBus in direct-commit mode deliver plain characters
//!   that never mark a preedit, and AppKit routes every unmarked keystroke
//!   through `insertText`. It ends nothing and owns no neighbouring keys —
//!   except the press that produced it: a press immediately before a
//!   commit is that keystroke delivered twice (winit reports both the
//!   `KeyboardInput` and the `Ime::Commit` for an unmarked `insertText`),
//!   and the commit carries the authoritative text, so the press stays
//!   inside the IME or the character would be inserted a second time.
//!
//! - A chord (Control/Alt/Super held) never produces text: it is the
//!   IME's only while a composition is live, never claimed as a commit's
//!   producer, and it stops the backward claim so a commit cannot reach
//!   past it to an earlier press.
//!
//! - `ImeDisabled` is the platform acknowledging that the app turned IME
//!   off — focus left a text target — not the answer to a keystroke, so it
//!   claims nothing: a Tab that moved focus out of the field in the same
//!   batch is ordinary input and the traversal still happens. It still
//!   ends `composing`, since a dead IME cannot hold a live composition.

use crate::platform::{InputEvent, KeyState};

/// Whether an event can be a keystroke's product — preedit and commit are
/// what an IME emits for the keys it consumed; `ImeDisabled` answers the
/// app's own "IME off" request, so it claims nothing.
fn is_keystroke_product(event: &InputEvent) -> bool {
    matches!(
        event,
        InputEvent::ImePreedit { .. } | InputEvent::ImeCommit { .. }
    )
}

/// Flags, per event in a drained platform batch, whether a key or text
/// event belongs to the IME rather than ordinary input handling.
/// `composing` seeds the walk with the composition the renderer already
/// holds; the returned flags are meaningful for `InputEvent::Key` and
/// `InputEvent::TextInput` entries and `false` for everything else.
pub(crate) fn ime_owned_events(events: &[InputEvent], composing: bool) -> Vec<bool> {
    // Each IME event claims the single keystroke that produced it: the
    // nearest modifier-free press before it, plus that press's own
    // TextInput events, skipping key releases in between. An earlier press
    // is never reached — it is ordinary input unless the composition flag
    // owns it below.
    let mut claimed = vec![false; events.len()];
    for (index, event) in events.iter().enumerate() {
        if !is_keystroke_product(event) {
            continue;
        }
        for behind in (0..index).rev() {
            match &events[behind] {
                InputEvent::Key {
                    state: KeyState::Released,
                    ..
                } => {}
                InputEvent::Key {
                    modifiers,
                    state: KeyState::Pressed,
                    ..
                } => {
                    if !modifiers.control && !modifiers.alt && !modifiers.super_key {
                        claimed[behind] = true;
                    }
                    break;
                }
                InputEvent::TextInput { .. } | InputEvent::KeyText { .. } => claimed[behind] = true,
                _ => break,
            }
        }
    }
    let mut composing = composing;
    let mut owned = Vec::with_capacity(events.len());
    for (index, event) in events.iter().enumerate() {
        owned.push(match event {
            InputEvent::Key { .. } | InputEvent::TextInput { .. } | InputEvent::KeyText { .. } => {
                composing || claimed[index]
            }
            InputEvent::ImePreedit { text, .. } => {
                composing = !text.is_empty();
                false
            }
            InputEvent::ImeCommit { .. } | InputEvent::ImeDisabled => {
                composing = false;
                false
            }
            _ => false,
        });
    }
    owned
}
