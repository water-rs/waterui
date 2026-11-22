//! The document `UIKit`'s text system edits: the focused editor's range
//! editing mirror, as a `cocoa_ui` [`TextInputClient`].
//!
//! `UITextInput` queries and edits arrive between frames and during them —
//! showing the keyboard asks for the document while the frame that raised it
//! still runs — so the document is shared state apart from the window
//! session. An edit rewrites the mirror at once, where the very next query
//! reads it back, and the next frame writes the mirror's projection into the
//! focused model (`flush`) before it dispatches input. After each frame the
//! renderer's authoritative snapshot is spliced back in; whatever it changed
//! beyond what `UIKit` itself did is announced to the input method, so its
//! context never drifts from the document.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use cocoa_ui::geometry::Rect;
use cocoa_ui::uikit::text_input::{
    Autocapitalization, Autocorrection, KeyboardType, ReturnKey, TextInputClient,
    TextInputNotifier, TextInputTraits,
};
use kurbo::Point;

use crate::renderer::{FocusedEditorSnapshot, SemanticCore};
use crate::runner::editing::{EditingSession, utf16_len, utf16_to_byte};
use crate::text::ParleyEngine;

/// The shared document state of one scene.
pub(super) struct ImeShared {
    session: RefCell<EditingSession>,
    /// The focused editor as the last frame left it; the geometry queries
    /// resolve against its display layout.
    snapshot: RefCell<Option<FocusedEditorSnapshot<ParleyEngine>>>,
    traits: Cell<TextInputTraits>,
    /// The keyboard's return key was pressed; the next frame performs the
    /// editor's action against the focused model.
    pending_return: Cell<bool>,
    /// An edit rewrote the mirror since the last flush.
    needs_flush: Cell<bool>,
    notifier: TextInputNotifier,
    /// Schedules the frame that flushes an edit into the focused model.
    request_frame: Rc<dyn Fn()>,
}

impl ImeShared {
    pub(super) fn new(notifier: TextInputNotifier, request_frame: Rc<dyn Fn()>) -> Rc<Self> {
        Rc::new(Self {
            session: RefCell::new(EditingSession::new()),
            snapshot: RefCell::new(None),
            traits: Cell::new(TextInputTraits::default()),
            pending_return: Cell::new(false),
            needs_flush: Cell::new(false),
            notifier,
            request_frame,
        })
    }

    /// Writes the mirror's pending edits into the focused model. Runs at the
    /// head of a frame, before input is dispatched, so the frame lays out the
    /// edited document.
    pub(super) fn flush(&self, core: &mut SemanticCore<ParleyEngine>) {
        let mut session = self.session.borrow_mut();
        if self.pending_return.take() {
            let editor = session.editor_id();
            session.perform_editor_action(editor, 0, core);
            self.needs_flush.set(true);
        }
        if self.needs_flush.take() {
            session.flush(core);
        }
    }

    /// Splices the renderer's authoritative snapshot into the mirror after a
    /// frame and tells the input method what moved beneath it.
    pub(super) fn sync_from_renderer(&self, snapshot: Option<FocusedEditorSnapshot<ParleyEngine>>) {
        let (editor_changed, text_changed, selection_changed) = {
            let mut session = self.session.borrow_mut();
            let before_id = session.editor_id();
            let before_text = session.text().to_owned();
            let before_selection = session.selection();
            let before_composing = session.composing();
            session.sync_from_renderer(snapshot.as_ref());
            session.clear_dirty();
            (
                session.editor_id() != before_id,
                session.text() != before_text || session.composing() != before_composing,
                session.selection() != before_selection,
            )
        };
        if let Some(snapshot) = &snapshot {
            self.traits.set(traits_of(snapshot));
        }
        self.snapshot.replace(snapshot);
        if editor_changed {
            self.notifier.reset();
            return;
        }
        if text_changed {
            self.notifier.will_change_text();
            self.notifier.did_change_text();
        }
        if selection_changed {
            self.notifier.will_change_selection();
            self.notifier.did_change_selection();
        }
    }

    /// Runs one edit against the mirror for the editor that owns the
    /// document now, then schedules the frame that flushes it.
    fn ops<R>(&self, edit: impl FnOnce(&mut EditingSession, u64) -> R) -> R {
        let mut session = self.session.borrow_mut();
        let editor = session.editor_id();
        let result = edit(&mut session, editor);
        drop(session);
        self.needs_flush.set(true);
        (self.request_frame)();
        result
    }

    /// Runs `read` against the focused editor's snapshot and the mirror.
    ///
    /// # Panics
    /// When no editor is focused: `UIKit` only asks for geometry of a
    /// document the client reported editable.
    fn geometry<R>(
        &self,
        read: impl FnOnce(&EditingSession, &FocusedEditorSnapshot<ParleyEngine>) -> R,
    ) -> R {
        let session = self.session.borrow();
        let snapshot = self.snapshot.borrow();
        let snapshot = snapshot
            .as_ref()
            .expect("UIKit asked an unfocused document for geometry");
        read(&session, snapshot)
    }
}

/// The keyboard a focused editor asks for.
fn traits_of(snapshot: &FocusedEditorSnapshot<ParleyEngine>) -> TextInputTraits {
    let single_line = snapshot.line_limit == Some(1);
    TextInputTraits {
        keyboard_type: KeyboardType::Default,
        return_key: if snapshot.has_submit {
            ReturnKey::Done
        } else {
            ReturnKey::Default
        },
        secure: snapshot.password,
        autocorrection: if snapshot.password {
            Autocorrection::Disabled
        } else {
            Autocorrection::Default
        },
        autocapitalization: if snapshot.password || !single_line {
            Autocapitalization::None
        } else {
            Autocapitalization::Sentences
        },
        content_type: None,
    }
}

/// The `UITextInput` document over [`ImeShared`].
pub(super) struct DocumentClient(pub(super) Rc<ImeShared>);

/// A `cocoa_ui` rectangle from a window-logical one.
const fn rect(rect: kurbo::Rect) -> Rect {
    Rect::new(rect.x0, rect.y0, rect.width(), rect.height())
}

impl TextInputClient for DocumentClient {
    fn is_editing(&self) -> bool {
        self.0.session.borrow().focused()
    }

    fn traits(&self) -> TextInputTraits {
        self.0.traits.get()
    }

    fn len(&self) -> usize {
        utf16_len(self.0.session.borrow().text())
    }

    fn text(&self, range: Range<usize>) -> String {
        let session = self.0.session.borrow();
        let text = session.text();
        text[utf16_to_byte(text, range.start)..utf16_to_byte(text, range.end)].to_owned()
    }

    fn selection(&self) -> Range<usize> {
        let (start, end) = self.0.session.borrow().selection();
        start.min(end)..start.max(end)
    }

    fn set_selection(&self, range: Range<usize>) {
        self.0.ops(|session, editor| {
            session.set_selection(editor, to_i32(range.start), to_i32(range.end));
        });
    }

    fn marked_range(&self) -> Option<Range<usize>> {
        self.0.session.borrow().composing()
    }

    fn set_marked_text(&self, text: &str, selected: Range<usize>) {
        self.0.ops(|session, editor| {
            // The caret rule is unused: the selection is placed explicitly
            // inside the marked text below.
            session.set_composing_text(editor, text, 1);
            let start = session.composing().map_or_else(
                || session.selection().0.saturating_sub(utf16_len(text)),
                |marked| marked.start,
            );
            session.set_selection(
                editor,
                to_i32(start + selected.start),
                to_i32(start + selected.end),
            );
        });
    }

    fn unmark_text(&self) {
        self.0.ops(EditingSession::finish_composing_text);
    }

    fn replace(&self, range: Range<usize>, text: &str) {
        self.0.ops(|session, editor| {
            let (start, end) = (range.start.min(range.end), range.start.max(range.end));
            let marked = session.composing();
            session.replace_range(editor, start, end, text);
            let caret = start + utf16_len(text);
            session.set_selection(editor, to_i32(caret), to_i32(caret));
            // A replacement that touches the marked text commits it.
            if marked.is_some_and(|marked| start <= marked.end && end >= marked.start) {
                session.finish_composing_text(editor);
            }
        });
    }

    fn first_rect(&self, range: Range<usize>) -> Rect {
        self.0.geometry(|session, snapshot| {
            let mut first = None;
            session.range_rects(snapshot, range.start, range.end, |rect| {
                first.get_or_insert(rect);
            });
            rect(first.unwrap_or_else(|| session.caret_rect_at(snapshot, range.start)))
        })
    }

    fn caret_rect(&self, offset: usize) -> Rect {
        self.0
            .geometry(|session, snapshot| rect(session.caret_rect_at(snapshot, offset)))
    }

    fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        self.0.geometry(|session, snapshot| {
            let mut rects = Vec::new();
            session.range_rects(snapshot, range.start, range.end, |each| {
                rects.push(rect(each));
            });
            rects
        })
    }

    fn closest_offset(&self, point: Point) -> usize {
        self.0
            .geometry(|session, snapshot| session.offset_at_point(snapshot, point.x, point.y))
    }

    fn perform_return(&self) {
        self.0.pending_return.set(true);
        (self.0.request_frame)();
    }
}

/// An offset within an `i32` the editing protocol takes; a document that
/// long is not a text field.
fn to_i32(offset: usize) -> i32 {
    i32::try_from(offset).expect("a text field's offsets fit the editing protocol's i32")
}
