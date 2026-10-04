//! The Android `InputConnection` range-editing protocol as a Rust state
//! machine — the editing half of the step-5 work in water-rs/hydrolysis#246.
//!
//! The `ImePreedit`/`ImeCommit` input events cannot express Android's
//! complete protocol: an `InputConnection` carries a text *revision*, a
//! selection, a composing region, an input purpose, an editor action and
//! caret geometry, and its mutating calls (`setComposingText`, `commitText`,
//! `setComposingRegion`, `setSelection`, `deleteSurroundingText`,
//! `deleteSurroundingTextInCodePoints`, batch edits, context actions) are
//! indexed in UTF-16 units — Android `CharSequence` semantics — while the
//! renderer's models and selection slots are byte-indexed UTF-8.
//!
//! [`EditingSession`] keeps the IME's view of the focused field as an
//! explicit mirror (`text`, UTF-16 selection edges, composing range). The
//! host-facing Kotlin `BaseInputConnection` holds the same state in a real
//! `Editable`; every one of its mutating calls reaches this module through a
//! single versioned JNI op. Ops rewrite the mirror; [`Self::flush`] writes
//! the mirror's projection — committed text, selection slot, pre-edit — into
//! the focused model, which stays authoritative: an external binding update
//! is adopted by the next [`Self::sync_from_renderer`] and pushed back to the
//! connection.
//!
//! Stale connections cannot write: each focused-field generation mints an
//! `editor_id` (bumped on every focused-identity change), the host tags every
//! op with the id its connection bound at creation, and ops from a stale id
//! are rejected.
//!
//! Cursor-anchor reporting (`requestCursorUpdates` /
//! `updateCursorAnchorInfo`) resolves insertion-marker and character bounds
//! from the field's display layout — committed text with the composition
//! spliced in, or the mask glyphs for a secure field — and is pushed only
//! while the IME is subscribed and the info changed; a scroll or insets move
//! produces a push, an identical frame produces none.

use std::ops::Range;

use crate::renderer::{FocusedEditorSnapshot, InteractionKey, SemanticCore, TextContextMenuAction};

/// `InputConnection.CURSOR_UPDATE_IMMEDIATE` — send the anchor info once.
pub const CURSOR_UPDATE_IMMEDIATE: i32 = 0x01;
/// `InputConnection.CURSOR_UPDATE_MONITOR` — keep sending it on every change.
pub const CURSOR_UPDATE_MONITOR: i32 = 0x02;
/// `CursorAnchorInfoRequest` filter bit for per-character bounds (API 33+).
const CURSOR_UPDATE_FILTER_CHARACTER_BOUNDS: i32 = 0x02;
/// Filter bit for the editor's own bounds.
const CURSOR_UPDATE_FILTER_EDITOR_BOUNDS: i32 = 0x04;
/// Filter bit for the composing text payload.
const CURSOR_UPDATE_FILTER_EDITING_TEXT: i32 = 0x08;
/// `EditorInfo.IME_ACTION_UNSPECIFIED` — a plain Enter.
const IME_ACTION_UNSPECIFIED: i32 = 0;

/// Character-bounds window half-width around the caret/composition, in
/// UTF-16 units; bounded so a huge document can't make one sync send the
/// whole text's geometry.
const CHAR_BOUNDS_RADIUS: usize = 64;

/// A context-menu editing action the host connection forwards
/// (`performContextMenuAction`) — the `android.R.id` text actions the
/// protocol carries; the renderer maps them onto the same primitives the
/// rendered context menu executes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(
    not(target_os = "android"),
    allow(dead_code, reason = "constructed by the Android JNI bridge")
)]
pub enum EditorContextAction {
    SelectAll,
    Cut,
    Copy,
    Paste,
}

/// The state a [`EditingSession`] projects into the focused renderer model:
/// committed text plus its selection slot (byte offsets) and the pre-edit
/// (with the platform-reported caret inside it) — the same triplet the
/// `ImePreedit` input event carries.
pub struct EditorProjection {
    pub(crate) committed: String,
    pub(crate) anchor: usize,
    pub(crate) focus: usize,
    pub(crate) preedit: Option<(String, Option<usize>)>,
}

/// One push to the host connection: the IME-visible editing state — text,
/// selection and composing range in UTF-16 units (the `Editable`'s indexing
/// domain), plus the attributes it needs to build `EditorInfo`.
#[cfg_attr(
    not(target_os = "android"),
    allow(dead_code, reason = "serialized by the Android JNI bridge")
)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the wire state mirrors the Android editor contract fields one-for-one; regrouping them would obscure the mapping"
)]
pub struct EditingState {
    pub(crate) editor_id: u64,
    pub(crate) focused: bool,
    pub(crate) revision: u64,
    pub(crate) text: String,
    pub(crate) sel_start: usize,
    pub(crate) sel_end: usize,
    /// Composing range, `(-1, -1)` when absent.
    pub(crate) comp_start: i64,
    pub(crate) comp_end: i64,
    pub(crate) password: bool,
    pub(crate) single_line: bool,
    pub(crate) has_submit: bool,
}

/// The cursor-anchor payload a subscribed IME receives: the insertion
/// marker, the composing range, and — when the request's filter asks for
/// them — the editor bounds, character bounds and composing text.
/// `PartialEq` is what lets the bridge suppress unchanged pushes.
#[derive(PartialEq)]
pub struct AnchorInfo {
    pub(crate) insertion: kurbo::Rect,
    pub(crate) editor_bounds: Option<kurbo::Rect>,
    pub(crate) sel_start: usize,
    pub(crate) sel_end: usize,
    /// Composing range, `(-1, -1)` when absent.
    pub(crate) comp_start: i64,
    pub(crate) comp_end: i64,
    pub(crate) composing_text: Option<String>,
    /// Per-character bounds, keyed by start index in UTF-16 units.
    pub(crate) char_bounds: Vec<(usize, kurbo::Rect)>,
}

/// UTF-16 length of `text` — `CharSequence::length` semantics.
pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// The byte index `u16_index` UTF-16 units into `text`, floored to the
/// enclosing character's start and clamped to the end — a surrogate-pair
/// interior index lands on the pair's first byte.
pub fn utf16_to_byte(text: &str, u16_index: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units + ch.len_utf16() > u16_index {
            return byte;
        }
        units += ch.len_utf16();
    }
    text.len()
}

/// UTF-16 units covered by `text[..byte_index]` — the inverse of
/// [`utf16_to_byte`] for char-boundary inputs.
pub fn byte_to_utf16(text: &str, byte_index: usize) -> usize {
    utf16_len(&text[..byte_index.min(text.len())])
}

/// A selection-ordered pair `(min, max)`.
fn ordered(start: usize, end: usize) -> (usize, usize) {
    (start.min(end), start.max(end))
}

/// AOSP span mapping through a `replace(start, end, new)`: a POINT endpoint
/// inside the replaced range collapses to the range's start — selection
/// spans are POINT.
const fn map_point(p: usize, start: usize, end: usize, new_len: usize) -> usize {
    if p <= start {
        p
    } else if p < end {
        start
    } else {
        p + new_len - (end - start)
    }
}

/// A MARK endpoint inside the replaced range moves to the end of the
/// inserted text — composing spans are MARK.
const fn map_mark(p: usize, start: usize, end: usize, new_len: usize) -> usize {
    if p <= start {
        p
    } else if p < end {
        start + new_len
    } else {
        p + new_len - (end - start)
    }
}

/// The IME-visible editing session for the focused text input. One instance
/// lives on the `AndroidSession`; it outlives individual `InputConnection`
/// objects (an IME may open several per focus lifetime — `editor_id`, not
/// the connection object, is the staleness discriminator).
#[derive(Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the session mirrors the InputConnection editing contract; each bool is an independent protocol flag"
)]
pub struct EditingSession {
    /// Minted per focused-field generation; every host op carries it.
    editor_id: u64,
    /// The focused field's identity; `None` = no text input is focused.
    key: Option<InteractionKey>,
    /// The IME-visible text: committed text with the composition spliced in.
    text: String,
    /// Selection edges in UTF-16 units; `sel_start` may lead `sel_end`,
    /// preserving the direction a keyboard selection was made.
    sel_start: usize,
    sel_end: usize,
    /// Composing range in UTF-16 units.
    composing: Option<Range<usize>>,
    /// Bumped by every authoritative change the mirror adopts.
    revision: u64,
    /// Nested `beginBatchEdit` depth; writes to the renderer are deferred to
    /// the outermost `endBatchEdit`, matching Android's batching contract.
    batch_depth: u32,
    /// At least one mirror mutation waits on [`Self::flush`].
    pending_apply: bool,
    /// The mirror changed since the last host push/pull.
    dirty: bool,
    password: bool,
    single_line: bool,
    has_submit: bool,
    /// `requestCursorUpdates` subscription: mode bits and (API 33+) filter.
    cursor_update_mode: i32,
    cursor_update_filter: i32,
}

impl EditingSession {
    pub fn new() -> Self {
        Self::default()
    }

    /// The editor generation the host tags ops with; `None`-focused sessions
    /// still mint ids, and ops against the wrong one are rejected.
    #[cfg_attr(not(test), allow(dead_code, reason = "editing protocol tests only"))]
    pub const fn editor_id(&self) -> u64 {
        self.editor_id
    }

    /// Whether a text input currently owns the editing session.
    /// The IME reads its own Editable mirror, so this getter is exercised
    /// by the protocol tests only.
    #[cfg_attr(not(test), allow(dead_code, reason = "editing protocol tests only"))]
    pub const fn focused(&self) -> bool {
        self.key.is_some()
    }

    /// The mirror text — what the host `Editable` holds.
    /// The IME reads its own Editable mirror, so this getter is exercised
    /// by the protocol tests only.
    #[cfg_attr(not(test), allow(dead_code, reason = "editing protocol tests only"))]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Selection edges in UTF-16 units `(selStart, selEnd)`; the order
    /// preserves the selection's direction.
    /// The IME reads its own Editable mirror, so this getter is exercised
    /// by the protocol tests only.
    #[cfg_attr(not(test), allow(dead_code, reason = "editing protocol tests only"))]
    pub const fn selection(&self) -> (usize, usize) {
        (self.sel_start, self.sel_end)
    }

    /// The composing range in UTF-16 units.
    /// The IME reads its own Editable mirror, so this getter is exercised
    /// by the protocol tests only.
    #[cfg_attr(not(test), allow(dead_code, reason = "editing protocol tests only"))]
    pub fn composing(&self) -> Option<Range<usize>> {
        self.composing.clone()
    }

    /// An op's editor id is live only while the connection's generation is.
    const fn live(&self, editor_id: u64) -> bool {
        self.key.is_some() && editor_id == self.editor_id
    }

    /// The current push/pull payload for the host.
    pub fn state(&self) -> EditingState {
        let (comp_start, comp_end) = self.composing.as_ref().map_or((-1, -1), |c| {
            (
                crate::num_cast::usize_as_i64(c.start),
                crate::num_cast::usize_as_i64(c.end),
            )
        });
        EditingState {
            editor_id: self.editor_id,
            focused: self.key.is_some(),
            revision: self.revision,
            text: self.text.clone(),
            sel_start: self.sel_start,
            sel_end: self.sel_end,
            comp_start,
            comp_end,
            password: self.password,
            single_line: self.single_line,
            has_submit: self.has_submit,
        }
    }

    /// Whether [`Self::state`] differs from the last one the host saw.
    #[cfg_attr(
        not(target_os = "android"),
        allow(dead_code, reason = "drives the Android push loop")
    )]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clears the dirty flag once the host has been handed `state()`.
    #[cfg_attr(
        not(target_os = "android"),
        allow(dead_code, reason = "drives the Android push loop")
    )]
    pub const fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// Whether cursor-anchor pushes were requested (`requestCursorUpdates`).
    /// `IMMEDIATE` asks for one update, `MONITOR` for every change; both are
    /// served by change-gated pushes, so an unchanged frame sends nothing.
    pub const fn cursor_updates_subscribed(&self) -> bool {
        self.cursor_update_mode & (CURSOR_UPDATE_IMMEDIATE | CURSOR_UPDATE_MONITOR) != 0
    }

    /// Adopt the renderer's authoritative snapshot: focused-field switches
    /// mint a fresh `editor_id`, and a live editor's committed
    /// text/selection/pre-edit is spliced into the mirror — an external
    /// binding update lands here too, mid-composition included. Returns
    /// whether the IME-visible state changed.
    pub fn sync_from_renderer(&mut self, snapshot: Option<&FocusedEditorSnapshot>) -> bool {
        let new_key = snapshot.map(|snap| snap.key.clone());
        let editor_changed = new_key != self.key;
        if editor_changed {
            self.editor_id += 1;
            self.key = new_key;
            // A batch never spans editors — a connection dying mid-batch
            // cannot wedge the next editor's applies.
            self.batch_depth = 0;
            self.pending_apply = false;
        }
        let Some(snap) = snapshot else {
            let changed = !self.text.is_empty()
                || self.sel_start != 0
                || self.sel_end != 0
                || self.composing.is_some()
                || self.has_submit;
            self.text.clear();
            self.sel_start = 0;
            self.sel_end = 0;
            self.composing = None;
            self.password = false;
            self.single_line = false;
            self.has_submit = false;
            if changed {
                self.revision += 1;
                self.dirty = true;
            }
            return changed || editor_changed;
        };
        let (text, composing, sel) = Self::project_snapshot(snap);
        let single_line = snap.line_limit == Some(1);
        let changed = text != self.text
            || sel != (self.sel_start, self.sel_end)
            || composing != self.composing
            || single_line != self.single_line
            || snap.password != self.password
            || snap.has_submit != self.has_submit;
        // While mirror mutations await their deferred batch flush the
        // renderer's snapshot is stale — adopting it would destroy the
        // ops' edits. The mirror stays authoritative until the flush runs.
        if !self.pending_apply {
            self.text = text;
            self.composing = composing;
            self.sel_start = sel.0;
            self.sel_end = sel.1;
            self.single_line = single_line;
            self.password = snap.password;
            self.has_submit = snap.has_submit;
            if changed {
                self.revision += 1;
                self.dirty = true;
            }
        }
        changed || editor_changed
    }

    /// Project an authoritative snapshot into mirror coordinates: committed
    /// text with the pre-edit spliced at the selection slot, the composition
    /// as a UTF-16 range over the splice, and the composition caret as the
    /// collapsed selection.
    fn project_snapshot(
        snap: &FocusedEditorSnapshot,
    ) -> (String, Option<Range<usize>>, (usize, usize)) {
        let committed = &snap.text;
        snap.preedit.as_ref().map_or_else(
            || {
                (
                    committed.clone(),
                    None,
                    (
                        byte_to_utf16(committed, snap.anchor),
                        byte_to_utf16(committed, snap.focus),
                    ),
                )
            },
            |preedit| {
                let (s, e) = ordered(snap.anchor, snap.focus);
                let comp_start = byte_to_utf16(committed, s);
                let mut text = String::with_capacity(committed.len() + preedit.len());
                text.push_str(&committed[..s]);
                text.push_str(preedit);
                text.push_str(&committed[e..]);
                let comp = comp_start..comp_start + utf16_len(preedit);
                let caret = comp.start
                    + byte_to_utf16(preedit, snap.preedit_caret.unwrap_or(preedit.len()));
                (text, Some(comp), (caret, caret))
            },
        )
    }

    /// The renderer-side projection of the mirror: committed text (the
    /// composition removed), the selection slot as committed-byte offsets —
    /// always the composition's committed position while composing, since
    /// the slot is where the renderer splices the pre-edit — and the
    /// pre-edit with the caret inside it.
    pub fn projection(&self) -> EditorProjection {
        let Some(comp) = &self.composing else {
            return EditorProjection {
                anchor: utf16_to_byte(&self.text, self.sel_start),
                focus: utf16_to_byte(&self.text, self.sel_end),
                committed: self.text.clone(),
                preedit: None,
            };
        };
        let cs = utf16_to_byte(&self.text, comp.start);
        let ce = utf16_to_byte(&self.text, comp.end);
        let mut committed = String::with_capacity(self.text.len() - (ce - cs));
        committed.push_str(&self.text[..cs]);
        committed.push_str(&self.text[ce..]);
        let preedit_text = self.text[cs..ce].to_string();
        // A collapsed selection inside the composition is the composition
        // caret; outside it clamps to the composition's edges (the renderer
        // has no way to draw a caret outside the splice).
        let caret = (self.sel_start == self.sel_end).then(|| {
            utf16_to_byte(
                &preedit_text,
                self.sel_start.clamp(comp.start, comp.end) - comp.start,
            )
        });
        EditorProjection {
            committed,
            anchor: cs,
            focus: cs,
            preedit: Some((preedit_text, caret)),
        }
    }

    /// Write the mirror's projection into the focused model — deferred to
    /// the outermost `endBatchEdit` while a batch is open, matching Android's
    /// "the other side sees nothing until `endBatchEdit`" contract.
    pub fn flush(&mut self, core: &mut SemanticCore) -> bool {
        if self.batch_depth > 0 {
            self.pending_apply = true;
            return false;
        }
        self.pending_apply = false;
        let projection = self.projection();
        core.apply_editor_projection(
            &projection.committed,
            projection.anchor,
            projection.focus,
            projection
                .preedit
                .as_ref()
                .map(|(text, caret)| (text.as_str(), *caret)),
        )
    }

    /// `setComposingText`: replace the composing range — or the selection —
    /// with `text`, mark the insertion as composing, and place the caret per
    /// the `newCursorPosition` rule (`> 0` counts forward from the inserted
    /// text's end, `<= 0` back from its start, clamped into the text).
    pub fn set_composing_text(
        &mut self,
        editor_id: u64,
        text: &str,
        new_cursor_position: i32,
    ) -> bool {
        if !self.live(editor_id) {
            return false;
        }
        let (a, b) = self.composing.clone().map_or_else(
            || ordered(self.sel_start, self.sel_end),
            |c| (c.start, c.end),
        );
        self.replace(a, b, text);
        let len = utf16_len(text);
        self.composing = (len > 0).then_some(a..a + len);
        self.place_caret(a, len, new_cursor_position);
        self.pending_apply = true;
        self.dirty = true;
        true
    }

    /// `commitText`: identical region/caret semantics to
    /// [`Self::set_composing_text`], leaving no composing span behind.
    pub fn commit_text(&mut self, editor_id: u64, text: &str, new_cursor_position: i32) -> bool {
        if !self.live(editor_id) {
            return false;
        }
        let (a, b) = self.composing.clone().map_or_else(
            || ordered(self.sel_start, self.sel_end),
            |c| (c.start, c.end),
        );
        self.replace(a, b, text);
        self.composing = None;
        self.place_caret(a, utf16_len(text), new_cursor_position);
        self.pending_apply = true;
        self.dirty = true;
        true
    }

    /// `setComposingRegion`: mark a range of the existing text as composing —
    /// how an IME *uncommits* text it intends to revise (CJK reconversion).
    /// The covered text becomes pre-edit at the next flush.
    pub fn set_composing_region(&mut self, editor_id: u64, start: i32, end: i32) -> bool {
        if !self.live(editor_id) {
            return false;
        }
        let len = utf16_len(&self.text);
        let a = crate::num_cast::i32_as_usize(start.max(0));
        let b = crate::num_cast::i32_as_usize(end.max(0));
        let (a, b) = ordered(a.min(len), b.min(len));
        self.composing = (a != b).then_some(a..b);
        self.pending_apply = true;
        self.dirty = true;
        true
    }

    /// `finishComposingText`: the composition is committed where it stands —
    /// its text stays in place, the composing marks lift, selection untouched.
    pub const fn finish_composing_text(&mut self, editor_id: u64) -> bool {
        if !self.live(editor_id) {
            return false;
        }
        self.composing = None;
        self.pending_apply = true;
        self.dirty = true;
        true
    }

    /// `setSelection`: move both selection edges; the order is kept, so a
    /// reversed (anchor-led) selection round-trips.
    pub fn set_selection(&mut self, editor_id: u64, start: i32, end: i32) -> bool {
        if !self.live(editor_id) {
            return false;
        }
        let len = utf16_len(&self.text);
        self.sel_start = (crate::num_cast::i32_as_usize(start.max(0))).min(len);
        self.sel_end = (crate::num_cast::i32_as_usize(end.max(0))).min(len);
        self.pending_apply = true;
        self.dirty = true;
        true
    }

    /// `deleteSurroundingText` in UTF-16 units: the protected region is the
    /// selection extended over the composing range; `before`/`after` are
    /// deleted outside it, never inside it.
    pub fn delete_surrounding_text(&mut self, editor_id: u64, before: i32, after: i32) -> bool {
        if !self.live(editor_id) {
            return false;
        }
        self.delete_around_protected(
            crate::num_cast::i32_as_usize(before.max(0)),
            crate::num_cast::i32_as_usize(after.max(0)),
        );
        self.pending_apply = true;
        self.dirty = true;
        true
    }

    /// `deleteSurroundingTextInCodePoints`: same protocol with code-point
    /// counts — a surrogate-pair emoji counts one, not two.
    pub fn delete_surrounding_text_in_code_points(
        &mut self,
        editor_id: u64,
        before: i32,
        after: i32,
    ) -> bool {
        if !self.live(editor_id) {
            return false;
        }
        let (a, b) = self.protected_region();
        let byte_a = utf16_to_byte(&self.text, a);
        let byte_b = utf16_to_byte(&self.text, b);
        let before_units: usize = self.text[..byte_a]
            .chars()
            .rev()
            .take(crate::num_cast::i32_as_usize(before.max(0)))
            .map(char::len_utf16)
            .sum();
        let after_units: usize = self.text[byte_b..]
            .chars()
            .take(crate::num_cast::i32_as_usize(after.max(0)))
            .map(char::len_utf16)
            .sum();
        self.delete_around_protected(before_units, after_units);
        self.pending_apply = true;
        self.dirty = true;
        true
    }

    /// `beginBatchEdit`: defer renderer writes to the outermost end.
    pub const fn begin_batch_edit(&mut self, editor_id: u64) -> bool {
        if !self.live(editor_id) {
            return false;
        }
        self.batch_depth += 1;
        true
    }

    /// `endBatchEdit` on the outermost batch; the deferred write runs on the
    /// caller's next [`Self::flush`].
    pub const fn end_batch_edit(&mut self, editor_id: u64) -> bool {
        if !self.live(editor_id) || self.batch_depth == 0 {
            return false;
        }
        self.batch_depth -= 1;
        true
    }

    /// `performContextMenuAction` for the actions a platform menu can send —
    /// select-all/cut/copy/paste reach the same primitives the rendered
    /// context menu executes, so a secure field still refuses cut/copy.
    pub fn perform_context_action(
        &self,
        editor_id: u64,
        action: EditorContextAction,
        core: &mut SemanticCore,
    ) -> bool {
        if !self.live(editor_id) {
            return false;
        }
        let action = match action {
            EditorContextAction::SelectAll => TextContextMenuAction::SelectAll,
            EditorContextAction::Cut => TextContextMenuAction::Cut,
            EditorContextAction::Copy => TextContextMenuAction::Copy,
            EditorContextAction::Paste => TextContextMenuAction::Paste,
        };
        core.perform_focused_context_action(action)
    }

    /// `performEditorAction`: `IME_ACTION_UNSPECIFIED` behaves like Enter —
    /// submit when the field declares it, a newline otherwise; an explicit
    /// action only ever submits, returning false when the field declines it
    /// (the system then hides the IME or moves focus itself).
    pub fn perform_editor_action(
        &mut self,
        editor_id: u64,
        action: i32,
        core: &SemanticCore,
    ) -> bool {
        if !self.live(editor_id) {
            return false;
        }
        if core.perform_editor_submit() {
            return true;
        }
        if action == IME_ACTION_UNSPECIFIED {
            return self.commit_text(editor_id, "\n", 1);
        }
        false
    }

    /// `requestCursorUpdates(mode[, filter])`.
    pub const fn set_cursor_update_subscription(
        &mut self,
        editor_id: u64,
        mode: i32,
        filter: i32,
    ) -> bool {
        if !self.live(editor_id) {
            return false;
        }
        self.cursor_update_mode = mode;
        self.cursor_update_filter = filter;
        true
    }

    /// The anchor info for the current subscription — `None` while no
    /// subscription is active or nothing is focused. Character bounds are
    /// resolved from the snapshot's display layout — committed + pre-edit for
    /// a field, the mask glyphs for a secure field.
    pub fn anchor_info(&self, snapshot: &FocusedEditorSnapshot) -> Option<AnchorInfo> {
        if !self.cursor_updates_subscribed() {
            return None;
        }
        let (comp_start, comp_end) = self.composing.as_ref().map_or((-1, -1), |c| {
            (
                crate::num_cast::usize_as_i64(c.start),
                crate::num_cast::usize_as_i64(c.end),
            )
        });
        let want_chars = self.cursor_update_filter == 0
            || self.cursor_update_filter & CURSOR_UPDATE_FILTER_CHARACTER_BOUNDS != 0;
        let want_editor_bounds = self.cursor_update_filter == 0
            || self.cursor_update_filter & CURSOR_UPDATE_FILTER_EDITOR_BOUNDS != 0;
        let want_text = self.cursor_update_filter == 0
            || self.cursor_update_filter & CURSOR_UPDATE_FILTER_EDITING_TEXT != 0;
        let composing_text = (want_text && self.composing.is_some()).then(|| {
            let comp = self.composing.as_ref().unwrap_or_else(|| unreachable!());
            self.text[utf16_to_byte(&self.text, comp.start)..utf16_to_byte(&self.text, comp.end)]
                .to_string()
        });
        Some(AnchorInfo {
            insertion: snapshot.cursor_area,
            editor_bounds: want_editor_bounds.then_some(snapshot.bounds),
            sel_start: self.sel_start,
            sel_end: self.sel_end,
            comp_start,
            comp_end,
            composing_text,
            char_bounds: if want_chars {
                self.character_bounds(snapshot)
            } else {
                Vec::new()
            },
        })
    }

    /// Character bounds around the caret/composition, resolved against the
    /// snapshot's display layout and offset by `text_bounds`. A surrogate
    /// pair occupies two UTF-16 units but one entry — Android keys
    /// `CursorAnchorInfo` bounds by the character's start index.
    fn character_bounds(&self, snapshot: &FocusedEditorSnapshot) -> Vec<(usize, kurbo::Rect)> {
        let display = snapshot.display_text.as_str();
        if display.is_empty() {
            return Vec::new();
        }
        let anchor = self
            .composing
            .as_ref()
            .map_or_else(|| self.sel_start.max(self.sel_end), |c| c.end);
        let window_start = anchor.saturating_sub(CHAR_BOUNDS_RADIUS);
        let window_end = (anchor + CHAR_BOUNDS_RADIUS).min(utf16_len(&self.text));
        // Iterate the mirror text (the IME's index domain); map each
        // character to its byte index in the display layout — for a secure
        // field the display is mask glyphs, one byte per real character.
        let mut entries = Vec::new();
        let mut u16_pos = 0usize;
        for (_, ch) in self.text.char_indices() {
            let char_u16 = ch.len_utf16();
            let in_window = u16_pos + char_u16 > window_start && u16_pos < window_end;
            if in_window {
                let display_start = self.display_byte(snapshot, display, u16_pos);
                let display_next = self.display_byte(snapshot, display, u16_pos + char_u16);
                let rect = char_rect(layout_of(snapshot), display, display_start, display_next);
                {
                    entries.push((
                        u16_pos,
                        kurbo::Rect::new(
                            snapshot.text_bounds.x0 + rect.x0,
                            snapshot.text_bounds.y0 + rect.y0,
                            snapshot.text_bounds.x0 + rect.x1,
                            snapshot.text_bounds.y0 + rect.y1,
                        ),
                    ));
                }
            }
            u16_pos += char_u16;
            if u16_pos >= window_end {
                break;
            }
        }
        entries
    }

    /// The display-layout byte index for a mirror UTF-16 index: direct for a
    /// text field (display == mirror text), the code-point ordinal for a
    /// secure field (mask glyphs are one byte each).
    fn display_byte(
        &self,
        snapshot: &FocusedEditorSnapshot,
        display: &str,
        u16_index: usize,
    ) -> usize {
        if snapshot.password {
            self.text[..utf16_to_byte(&self.text, u16_index)]
                .chars()
                .count()
        } else {
            utf16_to_byte(display, u16_index.min(utf16_len(display)))
        }
    }

    /// `getTextBeforeCursor(n, 0)`: up to `n` UTF-16 units before the
    /// protected region's start — the composition counts, since Android's
    /// no-styles contract reports it as part of the text.
    /// The IME reads its own Editable mirror, so this getter is exercised
    /// by the protocol tests only.
    #[cfg_attr(not(test), allow(dead_code, reason = "editing protocol tests only"))]
    pub fn text_before_cursor(&self, units: usize) -> String {
        let (start, _) = self.protected_region();
        let from = start.saturating_sub(units);
        self.text[utf16_to_byte(&self.text, from)..utf16_to_byte(&self.text, start)].to_string()
    }

    /// `getTextAfterCursor(n, 0)`.
    /// The IME reads its own Editable mirror, so this getter is exercised
    /// by the protocol tests only.
    #[cfg_attr(not(test), allow(dead_code, reason = "editing protocol tests only"))]
    pub fn text_after_cursor(&self, units: usize) -> String {
        let (_, end) = self.protected_region();
        let to = (end + units).min(utf16_len(&self.text));
        self.text[utf16_to_byte(&self.text, end)..utf16_to_byte(&self.text, to)].to_string()
    }

    /// `getSelectedText(0)` — `None` for a caret (Android's contract for a
    /// collapsed selection).
    /// The IME reads its own Editable mirror, so this getter is exercised
    /// by the protocol tests only.
    #[cfg_attr(not(test), allow(dead_code, reason = "editing protocol tests only"))]
    pub fn selected_text(&self) -> Option<String> {
        let (s, e) = ordered(self.sel_start, self.sel_end);
        (s != e).then(|| {
            self.text[utf16_to_byte(&self.text, s)..utf16_to_byte(&self.text, e)].to_string()
        })
    }

    /// The protected region `deleteSurroundingText` treats as the cursor:
    /// the selection grown over the composing range.
    fn protected_region(&self) -> (usize, usize) {
        let (mut a, mut b) = ordered(self.sel_start, self.sel_end);
        if let Some(comp) = &self.composing {
            a = a.min(comp.start);
            b = b.max(comp.end);
        }
        (a, b)
    }

    /// Delete `before`/`after` UTF-16 units around the protected region,
    /// mapping spans through each remove exactly as the host `Editable` does
    /// (selection endpoints are `SPAN_POINT` spans, composing endpoints
    /// `SPAN_MARK` spans).
    fn delete_around_protected(&mut self, before: usize, after: usize) {
        let (a, _b) = self.protected_region();
        let start = a.saturating_sub(before);
        self.replace(start, a, "");
        // The protected end moved left by what the first delete removed —
        // recompute it rather than carrying `b` through the mapping.
        let (_a2, b2) = self.protected_region();
        let end = (b2 + after).min(utf16_len(&self.text));
        self.replace(b2, end, "");
    }

    /// `Editable.replace` with AOSP endpoint mapping: POINT endpoints inside
    /// the replaced range collapse to its start, MARK endpoints move to the
    /// end of the inserted text.
    fn replace(&mut self, start: usize, end: usize, replacement: &str) {
        let new_len = utf16_len(replacement);
        self.sel_start = map_point(self.sel_start, start, end, new_len);
        self.sel_end = map_point(self.sel_end, start, end, new_len);
        if let Some(comp) = &mut self.composing {
            comp.start = map_mark(comp.start, start, end, new_len);
            comp.end = map_mark(comp.end, start, end, new_len);
        }
        let byte_start = utf16_to_byte(&self.text, start);
        let byte_end = utf16_to_byte(&self.text, end);
        self.text.replace_range(byte_start..byte_end, replacement);
        self.revision += 1;
    }

    /// The AOSP caret rule for `setComposingText`/`commitText`:
    /// `newCursorPosition > 0` counts forward from the inserted text's end
    /// (`1` = right after it); `<= 0` counts back from the insert position.
    fn place_caret(&mut self, insert_start: usize, insert_len: usize, new_cursor_position: i32) {
        let total = utf16_len(&self.text);
        let pos = if new_cursor_position > 0 {
            insert_start + insert_len + (crate::num_cast::i32_as_usize(new_cursor_position) - 1)
        } else {
            insert_start.saturating_sub(crate::num_cast::i32_as_usize(-new_cursor_position))
        }
        .min(total);
        self.sel_start = pos;
        self.sel_end = pos;
    }
}

fn layout_of(snapshot: &FocusedEditorSnapshot) -> &parley::Layout<[u8; 4]> {
    &snapshot.display_layout
}

/// A character's bounding rect from its display layout: the caret geometry
/// at its start extended to the next character's caret x — the convention
/// `CursorAnchorInfo.Builder.addCharacterBounds` expects.
fn char_rect(
    layout: &parley::Layout<[u8; 4]>,
    display: &str,
    byte_start: usize,
    byte_next: usize,
) -> kurbo::Rect {
    let affinity = |byte: usize| {
        if byte >= display.len() {
            parley::Affinity::Upstream
        } else {
            parley::Affinity::Downstream
        }
    };
    let start = parley::Cursor::from_byte_index(layout, byte_start, affinity(byte_start))
        .geometry(layout, 1.0);
    let end = parley::Cursor::from_byte_index(layout, byte_next, affinity(byte_next))
        .geometry(layout, 1.0);
    let x0 = start.x0.min(end.x0);
    let x1 = start.x0.max(end.x0).max(x0 + 1.0);
    kurbo::Rect::new(x0, start.y0, x1, start.y1)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::HeadlessRuntime;
    use crate::platform::{InputEvent, PointerButton, PointerKind};
    use crate::renderer::tests::{MinimalTestTheme, test_environment};
    use core::time::Duration;
    use nami::Signal as _;
    use std::cell::RefCell;
    use std::time::Instant;
    use waterui::ViewExt as _;
    use waterui_controls::text_field::field;
    use waterui_core::handler::AnyViewBuilder;
    use waterui_core::{AnyView, Binding, Str};
    use waterui_form::secure::{Secure, secure};

    const POINTER_ID: u64 = 9;

    fn runtime_with(view: AnyView) -> HeadlessRuntime {
        let view = RefCell::new(Some(view));
        let builder = AnyViewBuilder::<AnyView>::new(move || {
            view.borrow_mut()
                .take()
                .expect("the test view is built once")
        });
        HeadlessRuntime::new_for_tests(
            test_environment(),
            builder,
            400,
            640,
            MinimalTestTheme::default(),
        )
    }

    fn settled(runtime: &mut HeadlessRuntime, start: Instant) {
        for frame in 0..6 {
            let _ = runtime.pump_at(false, start + Duration::from_millis(frame * 16));
        }
    }

    fn press(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
        runtime.push_input_event(InputEvent::PointerDown {
            id: POINTER_ID,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        });
        runtime.push_input_event(InputEvent::PointerUp {
            id: POINTER_ID,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        });
    }

    /// Presses the center of the text input registered at `index`.
    fn press_text_input(runtime: &mut HeadlessRuntime, index: usize) {
        let center = runtime.renderer().text_editing.text_input_targets[index]
            .bounds
            .center();
        press(
            runtime,
            crate::num_cast::f64_as_f32(center.x),
            crate::num_cast::f64_as_f32(center.y),
        );
    }

    /// A focused single-line field bound to `value`, its editing session
    /// already synced.
    fn focused_field(initial: &str) -> (HeadlessRuntime, Binding<Str>, EditingSession, u64) {
        let value = Binding::container(Str::from(initial.to_owned()));
        let view = AnyView::new(field("Name", &value).size(300.0, 60.0));
        let mut runtime = runtime_with(view);
        let start = Instant::now();
        settled(&mut runtime, start);
        press_text_input(&mut runtime, 0);
        settled(&mut runtime, start + Duration::from_millis(100));
        assert!(
            runtime.focused_text_input_state().is_some(),
            "pressing the field must focus it"
        );
        let mut session = EditingSession::new();
        assert!(session.sync_from_renderer(runtime.renderer().focused_editor_snapshot().as_ref()));
        let id = session.editor_id();
        (runtime, value, session, id)
    }

    /// Sync the session from the renderer, returning whether it changed.
    fn sync(session: &mut EditingSession, runtime: &HeadlessRuntime) -> bool {
        session.sync_from_renderer(runtime.renderer().focused_editor_snapshot().as_ref())
    }

    fn selection_slot(runtime: &HeadlessRuntime) -> (usize, usize) {
        let targets = &runtime.renderer().text_editing.text_input_targets;
        let slot = targets[0].selection.borrow();
        (slot.anchor, slot.focus)
    }

    fn renderer_preedit(runtime: &HeadlessRuntime) -> Option<String> {
        runtime
            .renderer()
            .text_editing
            .ime_preedit
            .as_ref()
            .map(ToString::to_string)
    }

    #[test]
    fn utf16_byte_round_trips() {
        // \u{e7} = ç, \u{301} = combining acute — the grapheme "ḉ" is two
        // code points, one grapheme, two UTF-16 units.
        let text = "ab\u{1F600}\u{e7}\u{301}\u{65E5}\u{1F469}\u{200D}\u{1F4BB}";
        // "ab"=2, U+1F600 surrogate pair=2, ç+́=2, 日=1, ZWJ emoji=2+1+2=5
        assert_eq!(utf16_len(text), 12);
        for (byte, _) in text.char_indices().chain([(text.len(), ' ')]) {
            assert_eq!(utf16_to_byte(text, byte_to_utf16(text, byte)), byte);
        }
        // A surrogate-interior index floors to the pair's start.
        let emoji_start = utf16_to_byte(text, 2);
        assert_eq!(utf16_to_byte(text, 3), emoji_start);
        assert_eq!(&text[emoji_start..emoji_start + 4], "\u{1F600}");
        // Past-the-end clamps.
        assert_eq!(utf16_to_byte(text, 999), text.len());
    }

    #[test]
    fn composition_commit_and_caret_rule() {
        let (mut runtime, value, mut session, id) = focused_field("ab");
        // Mirror adopts the committed text and collapsed caret at end.
        assert_eq!(session.text(), "ab");
        assert_eq!(session.selection(), (2, 2));

        // setComposingText replaces the (collapsed) selection and marks the
        // inserted text; ncp=1 puts the caret right after it.
        assert!(session.set_composing_text(id, "\u{304D}\u{3087}\u{3046}", 1));
        assert_eq!(session.text(), "ab\u{304D}\u{3087}\u{3046}");
        assert_eq!(session.composing(), Some(2..5));
        assert_eq!(session.selection(), (5, 5));

        // The renderer projection: committed text unchanged, the composition
        // becomes the pre-edit with a byte caret inside it.
        let p = session.projection();
        assert_eq!(p.committed, "ab");
        assert_eq!(
            p.preedit.as_ref().map(|(t, _)| t.as_str()),
            Some("\u{304D}\u{3087}\u{3046}")
        );
        session.flush(runtime.renderer_mut());
        assert_eq!(
            renderer_preedit(&runtime).as_deref(),
            Some("\u{304D}\u{3087}\u{3046}")
        );

        // commitText(final, 1) lands the conversion and clears composition.
        assert!(session.commit_text(id, "\u{4ECA}\u{65E5}", 1));
        assert_eq!(session.text(), "ab\u{4ECA}\u{65E5}");
        assert_eq!(session.composing(), None);
        assert_eq!(session.selection(), (4, 4));
        session.flush(runtime.renderer_mut());
        assert_eq!(renderer_preedit(&runtime), None);
        // The binding sees committed text only.
        assert_eq!(value.snapshot().to_string(), "ab\u{4ECA}\u{65E5}");
    }

    #[test]
    fn composition_replaces_selection_and_survives_utf16() {
        let (mut runtime, value, mut session, id) = focused_field("a\u{1F600}b");
        // "a😀b": sel around the emoji — UTF-16 indices 1..3.
        assert!(session.set_selection(id, 1, 3));
        assert_eq!(session.selected_text().as_deref(), Some("\u{1F600}"));
        assert!(session.commit_text(id, "x", 1));
        assert_eq!(session.text(), "axb");
        session.flush(runtime.renderer_mut());
        assert_eq!(value.snapshot().to_string(), "axb");
    }

    #[test]
    fn delete_surrounding_text_counts_units_and_code_points() {
        // "a😀日b" — UTF-16 layout: a [0,1), 😀 [1,3), 日 [3,4), b [4,5).
        let (mut runtime, value, mut session, id) = focused_field("a\u{1F600}\u{65E5}b");
        // Caret after the emoji; deleting 2 UTF-16 units removes only it.
        assert!(session.set_selection(id, 3, 3));
        assert!(session.delete_surrounding_text(id, 2, 0));
        assert_eq!(session.text(), "a\u{65E5}b");
        session.flush(runtime.renderer_mut());
        assert_eq!(value.snapshot().to_string(), "a\u{65E5}b");

        // Restore and delete the same span by code points: the emoji and
        // the 'a' are two code points.
        value.set(Str::from("a\u{1F600}\u{65E5}b"));
        settled(&mut runtime, Instant::now());
        sync(&mut session, &runtime);
        assert!(session.set_selection(id, 3, 3));
        assert!(session.delete_surrounding_text_in_code_points(id, 2, 0));
        assert_eq!(session.text(), "\u{65E5}b");
        session.flush(runtime.renderer_mut());
        assert_eq!(value.snapshot().to_string(), "\u{65E5}b");
    }

    #[test]
    fn delete_surrounding_text_never_crosses_the_protected_region() {
        let (mut runtime, _value, mut session, id) = focused_field("abcdef");
        assert!(session.set_composing_text(id, "", 1));
        assert!(session.set_composing_region(id, 2, 4));
        assert!(session.set_selection(id, 3, 3));
        // Protected = sel ∪ comp = 2..4; deleting 10 before/after must not
        // touch inside it.
        assert!(session.delete_surrounding_text(id, 10, 10));
        assert_eq!(session.text(), "cd");
        session.flush(runtime.renderer_mut());
    }

    #[test]
    fn reversed_selection_round_trips() {
        let (mut runtime, _value, mut session, id) = focused_field("hello");
        // selStart > selEnd preserves the anchor-led direction.
        assert!(session.set_selection(id, 4, 1));
        assert_eq!(session.selection(), (4, 1));
        session.flush(runtime.renderer_mut());
        // The slot carries the same byte range (clamped), direction retained.
        let (anchor, focus) = selection_slot(&runtime);
        assert_eq!((anchor, focus), (4, 1));
        sync(&mut session, &runtime);
        assert_eq!(session.selection(), (4, 1));
    }

    #[test]
    fn composing_region_uncommits_text_for_reconversion() {
        let (mut runtime, _value, mut session, id) =
            focused_field("\u{3053}\u{3093}\u{306B}\u{3061}\u{306F}");
        // Mark "こんにち" (UTF-16 units 0..4) composing — a Japanese IME's
        // reconversion of already-committed text.
        assert!(session.set_composing_region(id, 0, 4));
        session.flush(runtime.renderer_mut());
        // The renderer sees the covered text as a pre-edit, not committed.
        assert_eq!(
            renderer_preedit(&runtime).as_deref(),
            Some("\u{3053}\u{3093}\u{306B}\u{3061}")
        );
        // The model still holds the full string — pre-edit is a projection.
        assert_eq!(session.text(), "\u{3053}\u{3093}\u{306B}\u{3061}\u{306F}");
        // finishComposingText drops the marks; text stays.
        assert!(session.finish_composing_text(id));
        session.flush(runtime.renderer_mut());
        assert_eq!(renderer_preedit(&runtime), None);
    }

    #[test]
    fn batch_edits_apply_once_at_the_outermost_end() {
        let (mut runtime, value, mut session, id) = focused_field("x");
        assert!(session.begin_batch_edit(id));
        assert!(session.begin_batch_edit(id));
        assert!(session.commit_text(id, "ab", 1));
        assert!(session.set_selection(id, 0, 0));
        session.flush(runtime.renderer_mut());
        assert_eq!(
            value.snapshot().to_string(),
            "x",
            "inside a batch nothing applies"
        );
        assert!(session.end_batch_edit(id));
        session.flush(runtime.renderer_mut());
        assert_eq!(value.snapshot().to_string(), "x", "inner end still defers");
        assert!(session.end_batch_edit(id));
        session.flush(runtime.renderer_mut());
        assert_eq!(value.snapshot().to_string(), "xab");
    }

    #[test]
    fn batched_commit_survives_a_mid_batch_sync() {
        // An IME batches the selection-replacement dance: the batch's
        // deferred flush means the renderer's snapshot is stale until
        // endBatchEdit. Adopting it mid-batch would clobber the op's edit.
        let (mut runtime, value, mut session, id) = focused_field("select me");
        assert!(session.set_selection(id, 0, 9));
        session.flush(runtime.renderer_mut());
        sync(&mut session, &runtime);
        assert!(session.begin_batch_edit(id));
        assert!(session.commit_text(id, "d", 1));
        assert_eq!(session.text(), "d");
        // The sync the host runs after every op must not roll the mirror
        // back to the stale committed text.
        sync(&mut session, &runtime);
        assert_eq!(session.text(), "d");
        assert_eq!(session.selection(), (1, 1));
        assert!(session.end_batch_edit(id));
        session.flush(runtime.renderer_mut());
        assert_eq!(value.snapshot().to_string(), "d");
    }

    #[test]
    fn stale_editor_ids_are_rejected() {
        let (mut runtime, _value, mut session, id) = focused_field("keep");
        // Focus moves away: the editor generation bumps and the old
        // connection's ops must be rejected. Simulate it by pressing empty
        // space (focused = None) then refocusing the field.
        press(&mut runtime, 10.0, 600.0);
        settled(&mut runtime, Instant::now());
        assert!(!sync(&mut session, &runtime) || !session.focused());
        assert!(!session.focused());
        // Ops tagged with the old id — now no focused editor — are rejected.
        assert!(!session.commit_text(id, "rogue", 1));
        assert!(!session.set_selection(id, 0, 1));
        // Refocus the field: a new editor id mints; the stale id stays dead.
        press_text_input(&mut runtime, 0);
        settled(&mut runtime, Instant::now());
        sync(&mut session, &runtime);
        let new_id = session.editor_id();
        assert_ne!(new_id, id);
        assert!(session.focused());
        assert!(!session.commit_text(id, "rogue", 1));
        assert_eq!(session.text(), "keep");
        assert!(session.commit_text(new_id, "+", 1));
    }

    #[test]
    fn external_binding_update_mid_composition_is_adopted() {
        let (mut runtime, value, mut session, id) = focused_field("base");
        assert!(session.set_composing_text(id, "\u{304D}", 1));
        session.flush(runtime.renderer_mut());
        assert_eq!(renderer_preedit(&runtime).as_deref(), Some("\u{304D}"));
        // The app mutates the binding under the live composition.
        value.set(Str::from("external"));
        settled(&mut runtime, Instant::now());
        sync(&mut session, &runtime);
        // The mirror picks up the new committed text and keeps the
        // composition spliced where the slot sits (byte 4 of "external").
        assert_eq!(session.text(), "exte\u{304D}rnal");
        assert!(session.composing().is_some());
        session.flush(runtime.renderer_mut());
        // A later commit lands on the externally-set text.
        assert!(session.commit_text(id, "\u{6728}", 1));
        session.flush(runtime.renderer_mut());
        assert!(value.snapshot().to_string().contains("\u{6728}"));
    }

    #[test]
    fn surrounding_text_and_selected_text_queries() {
        let (_runtime, _value, mut session, id) = focused_field("h\u{65E5}\u{1F600}lo");
        // UTF-16 indices: h=0, =1, emoji=2..4, l=4, o=5.
        assert!(session.set_selection(id, 2, 4));
        assert_eq!(session.selected_text().as_deref(), Some("\u{1F600}"));
        assert_eq!(session.text_before_cursor(2), "h\u{65E5}");
        assert_eq!(session.text_after_cursor(10), "lo");
        assert!(session.set_selection(id, 5, 5));
        assert_eq!(session.selected_text(), None);
    }

    #[test]
    fn editor_action_submits_a_declared_field() {
        let value = Binding::container(Str::default());
        let submitted = Binding::bool(false);
        let view = AnyView::new({
            let submitted_for_action = submitted.clone();
            field("Name", &value)
                .on_submit(move || submitted_for_action.set(true))
                .size(300.0, 60.0)
        });
        let mut runtime = runtime_with(view);
        let start = Instant::now();
        settled(&mut runtime, start);
        press_text_input(&mut runtime, 0);
        settled(&mut runtime, start + Duration::from_millis(100));
        let mut session = EditingSession::new();
        sync(&mut session, &runtime);
        let id = session.editor_id();
        // IME_ACTION_DONE = 6 — the field declares on_submit, so it submits.
        assert!(session.perform_editor_action(id, 6, runtime.renderer_mut()));
        assert!(submitted.snapshot());
    }

    #[test]
    fn context_menu_actions_reach_the_focused_model() {
        let (mut runtime, _value, session, id) = focused_field("select me");
        assert!(session.perform_context_action(
            id,
            EditorContextAction::SelectAll,
            runtime.renderer_mut()
        ));
        let (anchor, focus) = selection_slot(&runtime);
        assert_eq!((anchor.min(focus), anchor.max(focus)), (0, 9));
    }

    #[test]
    fn secure_field_projects_through_the_same_session() {
        let secret = Binding::container(Secure::new("hunter2".to_string()));
        let view = AnyView::new(secure("Password", &secret).size(300.0, 60.0));
        let mut runtime = runtime_with(view);
        let start = Instant::now();
        settled(&mut runtime, start);
        press_text_input(&mut runtime, 0);
        settled(&mut runtime, start + Duration::from_millis(100));
        let mut session = EditingSession::new();
        assert!(sync(&mut session, &runtime));
        let id = session.editor_id();
        assert_eq!(session.text(), "hunter2");
        assert!(session.state().password);
        // Committing is plain text; composing is refused by the password
        // purpose at the renderer level.
        assert!(session.commit_text(id, "x", 1));
        session.flush(runtime.renderer_mut());
        assert_eq!(secret.snapshot().expose(), "hunter2x");
        // Cut/copy stay inert on a secure field.
        assert!(session.perform_context_action(
            id,
            EditorContextAction::SelectAll,
            runtime.renderer_mut()
        ));
        assert!(!session.perform_context_action(
            id,
            EditorContextAction::Copy,
            runtime.renderer_mut()
        ));
    }

    #[test]
    fn cursor_anchor_info_is_subscription_and_change_gated() {
        let (runtime, _value, mut session, id) = focused_field("abc");
        // No subscription: no info.
        let snapshot = runtime.renderer().focused_editor_snapshot().unwrap();
        assert!(session.anchor_info(&snapshot).is_none());
        // Monitor subscription: info arrives with insertion + char bounds.
        assert!(session.set_cursor_update_subscription(id, CURSOR_UPDATE_MONITOR, 0));
        let info = session.anchor_info(&snapshot).expect("subscribed");
        assert_eq!(info.insertion, snapshot.cursor_area);
        assert_eq!(info.editor_bounds, Some(snapshot.bounds));
        assert_eq!(info.comp_start, -1);
        // Three characters inside the window → three bounds entries keyed by
        // UTF-16 index.
        let indices: Vec<usize> = info.char_bounds.iter().map(|(i, _)| *i).collect();
        assert_eq!(indices, vec![0, 1, 2]);
        // An INSERTION_MARKER-only filter drops the character bounds.
        assert!(session.set_cursor_update_subscription(id, CURSOR_UPDATE_MONITOR, 0x01));
        let filtered = session.anchor_info(&snapshot).expect("subscribed");
        assert_eq!(filtered.char_bounds, []);
        assert!(filtered.editor_bounds.is_none());
    }
}
