//! The document an [`InputView`] edits.
//!
//! `UITextInput` is a protocol over a *document*: the text, a selection, an
//! optional marked (pre-edit) range, and the geometry of all three. The
//! content behind an [`InputView`] — a self-drawn text editor — owns that
//! document; it implements [`TextInputClient`] and the view answers every
//! `UITextInput` query and applies every edit against it. All offsets and
//! ranges are UTF-16 code units, `NSString`'s domain, so a range `UIKit` hands
//! back is a range of the client's document with no conversion.
//!
//! Edits `UIKit` makes (`insertText:`, `deleteBackward`, `setMarkedText:…`)
//! reach the client through its methods and need no notification. Changes
//! the *content* makes on its own — a remote edit, a programmatic selection,
//! a different field gaining focus — are announced through the
//! [`TextInputNotifier`], so the input method's context never drifts from the
//! document.

use std::fmt;
use std::ops::Range;

use objc2::rc::Weak;
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSString;
use objc2_ui_kit::{UITextInput, UITextInputDelegate};

use super::input_view::InputView;
use crate::ContentType;
use crate::geometry::Rect;

/// The keyboard layout a field asks for (`UIKeyboardType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyboardType {
    /// The system default keyboard.
    #[default]
    Default,
    /// A keyboard limited to ASCII characters.
    Ascii,
    /// Numbers and punctuation.
    NumbersAndPunctuation,
    /// A keyboard tuned for entering URLs.
    Url,
    /// A numeric keypad.
    NumberPad,
    /// A telephone keypad.
    PhonePad,
    /// A keypad for names and phone numbers.
    NamePhonePad,
    /// A keyboard tuned for email addresses.
    EmailAddress,
    /// A numeric keypad with a decimal separator.
    DecimalPad,
    /// A keyboard tuned for social-media text.
    Twitter,
    /// A keyboard tuned for web searches.
    WebSearch,
    /// An ASCII-only numeric keypad.
    AsciiNumberPad,
}

/// The label and action of the keyboard's return key (`UIReturnKeyType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReturnKey {
    /// The default return key.
    #[default]
    Default,
    /// "Go".
    Go,
    /// "Google".
    Google,
    /// "Join".
    Join,
    /// "Next".
    Next,
    /// "Route".
    Route,
    /// "Search".
    Search,
    /// "Send".
    Send,
    /// "Yahoo".
    Yahoo,
    /// "Done".
    Done,
    /// "Emergency call".
    EmergencyCall,
    /// "Continue".
    Continue,
}

/// How the keyboard capitalizes typed text (`UITextAutocapitalizationType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Autocapitalization {
    /// Never.
    None,
    /// The first letter of every word.
    Words,
    /// The first letter of every sentence.
    #[default]
    Sentences,
    /// Every character.
    AllCharacters,
}

/// Whether the keyboard corrects typed text (`UITextAutocorrectionType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Autocorrection {
    /// The system default for the keyboard.
    #[default]
    Default,
    /// Never correct.
    Disabled,
    /// Always correct.
    Enabled,
}

/// The `UITextInputTraits` a document asks the keyboard for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextInputTraits {
    /// The keyboard layout.
    pub keyboard_type: KeyboardType,
    /// The return key's label and action.
    pub return_key: ReturnKey,
    /// Whether typed text is hidden and kept out of suggestions.
    pub secure: bool,
    /// Whether typed text is corrected.
    pub autocorrection: Autocorrection,
    /// How typed text is capitalized.
    pub autocapitalization: Autocapitalization,
    /// What the field's content means, for autofill; `None` for free text.
    pub content_type: Option<ContentType>,
}

/// The document behind an [`InputView`], as `UITextInput` sees it.
///
/// Every method runs on the main thread. Offsets and ranges count UTF-16
/// code units from the document's start; a range handed to a method is
/// within `0..=len()`.
pub trait TextInputClient {
    /// Whether an editor is focused. While it is not, the view reports no
    /// text, and hardware keys all reach the content as `Key` events instead
    /// of the text system.
    fn is_editing(&self) -> bool;

    /// The keyboard the focused editor asks for. Read when the keyboard is
    /// shown and again after [`TextInputNotifier::reset`].
    fn traits(&self) -> TextInputTraits;

    /// The document's length.
    fn len(&self) -> usize;

    /// Whether the document is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The text in `range`.
    fn text(&self, range: Range<usize>) -> String;

    /// The selection; empty at the caret.
    fn selection(&self) -> Range<usize>;

    /// Moves the selection to `range`.
    fn set_selection(&self, range: Range<usize>);

    /// The marked (pre-edit) range, if an input method is composing.
    fn marked_range(&self) -> Option<Range<usize>>;

    /// Replaces the marked range — or the selection when nothing is marked —
    /// with `text` and marks the result. `selected` is the new selection,
    /// relative to the start of `text`.
    fn set_marked_text(&self, text: &str, selected: Range<usize>);

    /// Commits the marked text: it stays in the document and stops being
    /// marked.
    fn unmark_text(&self);

    /// Replaces `range` with `text` and collapses the selection after it.
    /// Marked text overlapping `range` is committed by the replacement and
    /// no longer marked.
    fn replace(&self, range: Range<usize>, text: &str);

    /// The rectangle of the first line of `range`, in the [`InputView`]'s
    /// logical coordinates.
    fn first_rect(&self, range: Range<usize>) -> Rect;

    /// The caret rectangle at `offset`, in the view's logical coordinates.
    fn caret_rect(&self, offset: usize) -> Rect;

    /// The rectangles covering `range`, one per line, in the view's logical
    /// coordinates.
    fn selection_rects(&self, range: Range<usize>) -> Vec<Rect>;

    /// The offset nearest `point`, in the view's logical coordinates.
    fn closest_offset(&self, point: kurbo::Point) -> usize;

    /// The editor's return-key action.
    fn perform_return(&self);

    /// The software keyboard's delete key: removes the selection, else the
    /// grapheme cluster before the caret, and does nothing at the document
    /// start. Content that is not a document overrides it with what its own
    /// delete means.
    fn delete_backward(&self) {
        let selection = self.selection();
        let range = if selection.is_empty() {
            if selection.start == 0 {
                return;
            }
            previous_grapheme(self, selection.start)
        } else {
            selection
        };
        self.replace(range, "");
    }
}

/// Announces changes the content makes to its document to the input
/// method, through the view's `inputDelegate`.
///
/// A change is bracketed: `will_change_*` before the document moves,
/// `did_change_*` after. UIKit-initiated edits are never announced. The
/// notifier holds the view weakly; once the view is gone every call is a
/// no-op. Main thread only.
#[derive(Clone)]
pub struct TextInputNotifier {
    view: Weak<InputView>,
}

impl fmt::Debug for TextInputNotifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextInputNotifier")
            .field("view_alive", &self.view.load().is_some())
            .finish()
    }
}

impl TextInputNotifier {
    pub(crate) const fn new(view: Weak<InputView>) -> Self {
        Self { view }
    }

    /// Runs `notify` against the view's input delegate and the view as the
    /// `UITextInput` the delegate is told about.
    fn with_delegate(
        &self,
        notify: impl FnOnce(&ProtocolObject<dyn UITextInputDelegate>, &ProtocolObject<dyn UITextInput>),
    ) {
        let Some(view) = self.view.load() else {
            return;
        };
        let Some(delegate) = view.delegate() else {
            return;
        };
        notify(&delegate, ProtocolObject::from_ref(&*view));
    }

    /// The document's text is about to change.
    pub fn will_change_text(&self) {
        self.with_delegate(|delegate, input| delegate.textWillChange(Some(input)));
    }

    /// The document's text changed.
    pub fn did_change_text(&self) {
        self.with_delegate(|delegate, input| delegate.textDidChange(Some(input)));
    }

    /// The selection is about to change.
    pub fn will_change_selection(&self) {
        self.with_delegate(|delegate, input| delegate.selectionWillChange(Some(input)));
    }

    /// The selection changed.
    pub fn did_change_selection(&self) {
        self.with_delegate(|delegate, input| delegate.selectionDidChange(Some(input)));
    }

    /// The document's identity changed — a different editor took focus, so
    /// nothing the input method knows applies any more. The keyboard
    /// reloads against the client's traits, and a composition left over from
    /// the previous document is committed away.
    pub fn reset(&self) {
        let Some(view) = self.view.load() else {
            return;
        };
        self.will_change_text();
        view.unmark_document();
        self.did_change_text();
        view.reloadInputViews();
    }
}

/// How far back, in UTF-16 code units, a grapheme cluster is looked for. A
/// cluster longer than this (a ZWJ family sequence is under 30) is split at
/// the window's edge.
const GRAPHEME_WINDOW: usize = 64;

/// The grapheme cluster ending at `offset` (`offset > 0`), in UTF-16 units.
///
/// `NSString`'s composed-character-sequence rules are `UIKit`'s own notion
/// of "one character" for deletion and cursor movement, so a surrogate pair,
/// a base with combining marks and an emoji ZWJ sequence each delete whole.
pub fn previous_grapheme<C: TextInputClient + ?Sized>(client: &C, offset: usize) -> Range<usize> {
    let window_start = offset.saturating_sub(GRAPHEME_WINDOW);
    let text = NSString::from_str(&client.text(window_start..offset));
    let length = text.length();
    if length == 0 {
        return offset.saturating_sub(1)..offset;
    }
    let cluster = text.rangeOfComposedCharacterSequenceAtIndex(length - 1);
    (window_start + cluster.location)..offset
}

/// The grapheme cluster starting at `offset` (`offset < len`), in UTF-16
/// units.
pub fn next_grapheme<C: TextInputClient + ?Sized>(client: &C, offset: usize) -> Range<usize> {
    let window_end = offset.saturating_add(GRAPHEME_WINDOW).min(client.len());
    let text = NSString::from_str(&client.text(offset..window_end));
    if text.length() == 0 {
        return offset..offset;
    }
    let cluster = text.rangeOfComposedCharacterSequenceAtIndex(0);
    offset..(offset + cluster.length)
}
