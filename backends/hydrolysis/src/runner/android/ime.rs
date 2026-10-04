//! The IME bridge: `InputConnection` updates arrive from Kotlin and are
//! pushed into the session as [`InputEvent`]s; the focused field's editing
//! state (cursor rect, purpose) is pushed back through
//! `sync_text_input_state`, which the host's `HydrolysisInputConnection`
//! consumes to position the candidates strip.
//!
//! Composing ranges map to `ImePreedit`, a commit maps to `ImeCommit` —
//! the same events a desktop IME sends, so the renderer's text-editing
//! path needs no Android awareness.

use crate::platform::{InputEvent, KeyCode, KeyState, Modifiers};

use super::host::AndroidHostWindow;

/// The Kotlin IME controller's editing state — whether the host currently
/// reports a composing (marked) region active.
#[derive(Default)]
pub(crate) struct ImeBridge {
    composing: bool,
}

impl ImeBridge {
    /// `setComposingText` — a composing (marked) range: preedit, with the
    /// caret at the reported offset in bytes.
    pub(crate) fn set_composing_text(
        &mut self,
        platform: &mut AndroidHostWindow,
        text: String,
        caret: usize,
    ) {
        self.composing = true;
        platform.push_event(InputEvent::ImePreedit {
            text,
            caret: Some(caret),
        });
    }

    /// `commitText` — confirmed insertion.
    pub(crate) fn commit_text(&mut self, platform: &mut AndroidHostWindow, text: String) {
        self.composing = false;
        platform.push_event(InputEvent::ImeCommit { text });
    }

    /// `finishComposingText` — composing ends; the preedit buffer drops.
    pub(crate) fn finish_composing(&mut self, platform: &mut AndroidHostWindow) {
        if !self.composing {
            return;
        }
        self.composing = false;
        platform.push_event(InputEvent::ImePreedit {
            text: String::new(),
            caret: None,
        });
    }

    /// `sendKeyEvent` — a hardware/IME key press the InputConnection routed,
    /// already decoded by Kotlin to a W3C key name.
    pub(crate) fn key_event(
        &mut self,
        platform: &mut AndroidHostWindow,
        key: String,
        pressed: bool,
        modifiers: Modifiers,
    ) {
        let code = KeyCode::Named(key);
        platform.push_event(InputEvent::Key {
            logical_key: code.to_w3c_key(),
            key: code,
            physical_code: keyboard_types::Code::Unidentified,
            repeat: false,
            state: if pressed {
                KeyState::Pressed
            } else {
                KeyState::Released
            },
            modifiers,
        });
    }
}
