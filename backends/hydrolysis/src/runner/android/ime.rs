//! The Android IME bridge: the Kotlin `HydrolysisInputConnection` forwards
//! every `InputConnection` mutator as one `nativeEditOp` call, dispatched here
//! onto the session's [`EditingSession`] — the authoritative UTF-16 mirror in
//! `runner::editing` — and then flushed into the renderer. State flows the
//! other way through `editing_sync`: after every op and every frame the
//! renderer's focused-editor snapshot is spliced into the mirror, and the
//! resulting pushes (`onNativeEditingState`, `onNativeCursorAnchorInfo`)
//! carry the IME-visible state and cursor geometry back to the connection.
//!
//! `sendKeyEvent` is not part of the range protocol: a hardware key the IME
//! routes through the connection still arrives as an ordinary
//! [`InputEvent::Key`]. Editing operations are never simulated keypresses.

use serde::Serialize;

use crate::platform::{InputEvent, KeyState, Modifiers, PlatformWindow};
use crate::runner::editing::{AnchorInfo, EditingSession, EditingState, EditorContextAction};

use super::host::AndroidSession;

/// The `InputConnection` mutators, as `nativeEditOp` opcodes. These match the
/// `EDIT_OP_*` constants in `HydrolysisInputConnection.kt`; the JNI schema
/// bump guards against a skewed pair.
const OP_SET_COMPOSING_REGION: i32 = 0;
const OP_FINISH_COMPOSING_TEXT: i32 = 1;
const OP_SET_SELECTION: i32 = 2;
const OP_DELETE_SURROUNDING: i32 = 3;
const OP_DELETE_SURROUNDING_POINTS: i32 = 4;
const OP_BEGIN_BATCH: i32 = 5;
const OP_END_BATCH: i32 = 6;
const OP_CONTEXT_ACTION: i32 = 7;
const OP_EDITOR_ACTION: i32 = 8;
const OP_CURSOR_UPDATES: i32 = 9;
const OP_SET_COMPOSING_TEXT: i32 = 10;
const OP_COMMIT_TEXT: i32 = 11;

/// `performContextMenuAction`'s `android.R.id` space, compressed onto the
/// four actions the renderer executes — Kotlin maps the R ids onto these.
const CONTEXT_ACTION_SELECT_ALL: i32 = 0;
const CONTEXT_ACTION_CUT: i32 = 1;
const CONTEXT_ACTION_COPY: i32 = 2;
const CONTEXT_ACTION_PASTE: i32 = 3;

/// The session's editing state on the Kotlin side: the protocol mirror plus
/// the state the renderer last pushed.
pub struct ImeBridge {
    pub(crate) session: EditingSession,
    /// The anchor info already delivered to the IME — `updateCursorAnchorInfo`
    /// never fires with unchanged geometry.
    last_anchor: Option<AnchorInfo>,
}

impl Default for ImeBridge {
    fn default() -> Self {
        Self {
            session: EditingSession::new(),
            last_anchor: None,
        }
    }
}

impl ImeBridge {
    /// `sendKeyEvent` — a hardware/IME key press the `InputConnection` routed,
    /// already decoded by Kotlin to a W3C key value.
    pub fn key_event(
        platform: &mut super::host::AndroidHostWindow,
        key: &str,
        pressed: bool,
        modifiers: Modifiers,
    ) {
        let (code, logical_key) = crate::runner::editing::hardware_key(key);
        platform.push_event(InputEvent::Key {
            logical_key,
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

/// The `EditingState` push/pull payload — the `Editable` mirror's complete
/// authoritative state plus the attributes `EditorInfo` needs.
#[derive(Serialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the JSON mirrors the EditingState wire payload; each bool is an independent protocol flag"
)]
struct EditingStateJson {
    editor_id: u64,
    focused: bool,
    revision: u64,
    text: String,
    sel_start: usize,
    sel_end: usize,
    comp_start: i64,
    comp_end: i64,
    password: bool,
    single_line: bool,
    has_submit: bool,
}

pub fn editing_state_json(state: &EditingState) -> String {
    serde_json::to_string(&EditingStateJson {
        editor_id: state.editor_id,
        focused: state.focused,
        revision: state.revision,
        text: state.text.clone(),
        sel_start: state.sel_start,
        sel_end: state.sel_end,
        comp_start: state.comp_start,
        comp_end: state.comp_end,
        password: state.password,
        single_line: state.single_line,
        has_submit: state.has_submit,
    })
    .expect("editing state serialization cannot fail")
}

/// `CursorAnchorInfo` as JSON: rectangles in logical units (`[l, t, r, b]`),
/// character bounds as `[utf16 index, l, t, r, b]` — Kotlin scales by the
/// window density into view pixels.
#[derive(Serialize)]
struct AnchorInfoJson {
    insertion: [f64; 4],
    editor_bounds: Option<[f64; 4]>,
    sel_start: usize,
    sel_end: usize,
    comp_start: i64,
    comp_end: i64,
    composing_text: Option<String>,
    char_bounds: Vec<[f64; 5]>,
}

fn anchor_json(info: &AnchorInfo) -> String {
    const fn rect(r: kurbo::Rect) -> [f64; 4] {
        [r.x0, r.y0, r.x1, r.y1]
    }
    serde_json::to_string(&AnchorInfoJson {
        insertion: rect(info.insertion),
        editor_bounds: info.editor_bounds.map(rect),
        sel_start: info.sel_start,
        sel_end: info.sel_end,
        comp_start: info.comp_start,
        comp_end: info.comp_end,
        composing_text: info.composing_text.clone(),
        char_bounds: info
            .char_bounds
            .iter()
            .map(|(index, r)| {
                let [x0, y0, x1, y1] = rect(*r);
                [crate::num_cast::usize_as_f64(*index), x0, y0, x1, y1]
            })
            .collect(),
    })
    .expect("anchor info serialization cannot fail")
}

impl AndroidSession {
    /// `nativeEditOp`: dispatch one `InputConnection` mutator onto the editing
    /// session, then — when it was accepted — flush the mirror into the
    /// renderer and sync/push the resulting state. A stale `editor_id`
    /// (a connection that outlived its focused editor) is rejected and left
    /// the mirror untouched.
    pub(crate) fn edit_op(
        &mut self,
        editor_id: u64,
        op: i32,
        arg1: i32,
        arg2: i32,
        text: &str,
    ) -> bool {
        let editing = &mut self.ime.session;
        let renderer = &mut *self.runtime.renderer;
        let handled = match op {
            OP_SET_COMPOSING_REGION => editing.set_composing_region(editor_id, arg1, arg2),
            OP_FINISH_COMPOSING_TEXT => editing.finish_composing_text(editor_id),
            OP_SET_SELECTION => editing.set_selection(editor_id, arg1, arg2),
            OP_DELETE_SURROUNDING => editing.delete_surrounding_text(editor_id, arg1, arg2),
            OP_DELETE_SURROUNDING_POINTS => {
                editing.delete_surrounding_text_in_code_points(editor_id, arg1, arg2)
            }
            OP_BEGIN_BATCH => editing.begin_batch_edit(editor_id),
            OP_END_BATCH => editing.end_batch_edit(editor_id),
            OP_CONTEXT_ACTION => match arg1 {
                CONTEXT_ACTION_SELECT_ALL => editing.perform_context_action(
                    editor_id,
                    EditorContextAction::SelectAll,
                    renderer,
                ),
                CONTEXT_ACTION_CUT => {
                    editing.perform_context_action(editor_id, EditorContextAction::Cut, renderer)
                }
                CONTEXT_ACTION_COPY => {
                    editing.perform_context_action(editor_id, EditorContextAction::Copy, renderer)
                }
                CONTEXT_ACTION_PASTE => {
                    editing.perform_context_action(editor_id, EditorContextAction::Paste, renderer)
                }
                _ => false,
            },
            OP_EDITOR_ACTION => editing.perform_editor_action(editor_id, arg1, renderer),
            OP_CURSOR_UPDATES => editing.set_cursor_update_subscription(editor_id, arg1, arg2),
            OP_SET_COMPOSING_TEXT => editing.set_composing_text(editor_id, text, arg1),
            OP_COMMIT_TEXT => editing.commit_text(editor_id, text, arg1),
            _ => false,
        };
        if handled {
            self.editing_flush_and_sync();
        }
        handled
    }

    /// The mirror's pending projection lands on the renderer, then the
    /// renderer-side result (normalized text, clamped selection) is synced
    /// back and pushed. The frame the edit produced must be scheduled —
    /// editing ops arrive between vsyncs, so this requests one.
    pub(crate) fn editing_flush_and_sync(&mut self) {
        self.ime.session.flush(&mut self.runtime.renderer);
        self.runtime.request_refresh();
        self.runtime.platform.request_redraw();
        self.editing_sync();
    }

    /// Reconcile the mirror with the renderer's authoritative snapshot and
    /// push whatever changed: the editing state to the connection's
    /// `Editable` mirror, and — while `requestCursorUpdates` is subscribed —
    /// the cursor anchor info. Both pushes are change-gated: an unchanged
    /// frame sends nothing.
    pub(crate) fn editing_sync(&mut self) {
        // A platform-view child taking UI focus — tapping a field inside the
        // system WebView — owns the IME now: clear the renderer's stale
        // `WaterUI` text-input claim on the gain edge so the state never
        // reports a Hydrolysis field focused while the page holds focus.
        // Only the edge clears: a claim set during the hold is the user's
        // hand-off tap on a Hydrolysis field and must survive to the show.
        // Either edge refreshes and redraws — a released claim has to reach
        // the frame too, not only the gain.
        if self.runtime.platform.platform_view_focus.take_changed() {
            if self.runtime.platform.platform_view_focus.is_holding() {
                let _ = self.runtime.renderer.clear_ui_focus();
            }
            self.runtime.request_refresh();
            self.runtime.platform.request_redraw();
        }
        let snapshot = self.runtime.renderer.focused_editor_snapshot();
        self.ime.session.sync_from_renderer(snapshot.as_ref());
        if self.ime.session.is_dirty() {
            let json = editing_state_json(&self.ime.session.state());
            self.runtime.platform.bridge.editing_state_changed(&json);
            self.ime.session.clear_dirty();
        }
        let Some(snap) = snapshot else { return };
        let Some(info) = self.ime.session.anchor_info(&snap) else {
            return;
        };
        if self.ime.last_anchor.as_ref() == Some(&info) {
            return;
        }
        self.runtime
            .platform
            .bridge
            .cursor_anchor_changed(&anchor_json(&info));
        self.ime.last_anchor = Some(info);
    }
}
