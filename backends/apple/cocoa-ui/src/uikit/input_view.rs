//! The first responder for a view that takes its own input.
//!
//! An [`InputView`] sits on top of the view whose pixels host interactive
//! content — a GPU surface — claims touch and keyboard input, and speaks
//! `UITextInput` so an input method composes against the content's caret.
//! The view draws nothing; every event is translated into the
//! platform-neutral [`SurfaceEvent`] vocabulary and handed to the installed
//! handler.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UIView` subclass implementing `UITextInput`
//! plus the abstract `UITextPosition`/`UITextRange` subclasses the protocol
//! requires. `UIKit` calls every override on the main thread; the pre-edit
//! buffer lives in the view's ivars so the protocol's document model holds
//! only the composition text.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{
    NSArray, NSAttributedStringKey, NSComparisonResult, NSDictionary, NSRange, NSSet, NSString,
};
use objc2_ui_kit::{
    UIEvent, UIKeyInput, UIPress, UIPressesEvent, UITextInput, UITextInputDelegate,
    UITextInputStringTokenizer, UITextInputTokenizer, UITextInputTraits, UITextLayoutDirection,
    UITextPosition, UITextRange, UITextSelectionRect, UITextStorageDirection, UITouch, UIView,
};

use crate::callback::guarded;
use crate::geometry::Rect;
use crate::input::{PointerButton, SurfaceEvent};
use crate::keys;

/// Delivers one surface event to the content.
type EventHandler = Rc<dyn Fn(SurfaceEvent)>;

/// The content's text caret, in logical surface-local points, if it has one.
type CaretProvider = Rc<dyn Fn() -> Option<Rect>>;

/// The ivars of an [`InputView`].
#[derive(Default)]
pub struct InputViewIvars {
    event_handler: RefCell<Option<EventHandler>>,
    caret_provider: RefCell<Option<CaretProvider>>,
    /// The pre-edit text the input method is currently composing, if any.
    marked_text: RefCell<String>,
    /// The selection the input method last reported, in UTF-16 units.
    marked_selection: std::cell::Cell<usize>,
}

impl fmt::Debug for InputViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InputViewIvars")
            .field("event_handler", &self.event_handler.borrow().is_some())
            .field("caret_provider", &self.caret_provider.borrow().is_some())
            .field("marked_text", &self.marked_text)
            .finish_non_exhaustive()
    }
}

/// The ivars of a [`TextPosition`].
#[derive(Debug, Default)]
pub struct TextPositionIvars {
    offset: std::cell::Cell<usize>,
}

define_class!(
    // SAFETY: `UITextPosition` is an abstract base asking for `init`; the
    // subclass carries its offset as an ivar and implements no methods.
    #[unsafe(super(UITextPosition))]
    #[name = "CocoaUiTextPosition"]
    #[thread_kind = MainThreadOnly]
    #[ivars = TextPositionIvars]
    #[derive(Debug)]
    /// A position in the pre-edit buffer, counted in UTF-16 code units.
    pub struct TextPosition;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UITextPosition`
    // subclass.
    unsafe impl NSObjectProtocol for TextPosition {}
);

impl TextPosition {
    /// A text position at `offset` UTF-16 code units into the buffer.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, offset: usize) -> Retained<Self> {
        let ivars = TextPositionIvars::default();
        ivars.offset.set(offset);
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: `init` is `UITextPosition`'s initializer.
        unsafe { msg_send![super(this), init] }
    }

    /// The offset this position counts from the buffer's start.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.ivars().offset.get()
    }
}

/// The ivars of a [`TextRange`].
#[derive(Debug, Default)]
pub struct TextRangeIvars {
    start: std::cell::Cell<usize>,
    end: std::cell::Cell<usize>,
}

define_class!(
    // SAFETY: `UITextRange` is an abstract base whose `start`, `end` and
    // `isEmpty` the subclass supplies.
    #[unsafe(super(UITextRange))]
    #[name = "CocoaUiTextRange"]
    #[thread_kind = MainThreadOnly]
    #[ivars = TextRangeIvars]
    #[derive(Debug)]
    /// A range of the pre-edit buffer, counted in UTF-16 code units.
    pub struct TextRange;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UITextRange` subclass.
    unsafe impl NSObjectProtocol for TextRange {}

    impl TextRange {
        // SAFETY: see the module safety note.
        #[unsafe(method_id(start))]
        fn start_override(&self) -> Retained<UITextPosition> {
            // SAFETY: `TextPosition::new` runs on the main thread.
            TextPosition::new(self.mtm(), self.ivars().start.get()).into_super()
        }

        // SAFETY: see the module safety note.
        #[unsafe(method_id(end))]
        fn end_override(&self) -> Retained<UITextPosition> {
            TextPosition::new(self.mtm(), self.ivars().end.get()).into_super()
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(isEmpty))]
        fn is_empty_override(&self) -> bool {
            self.ivars().start.get() == self.ivars().end.get()
        }
    }
);

impl TextRange {
    /// A text range covering `[start, end)` UTF-16 code units.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, start: usize, end: usize) -> Retained<Self> {
        let ivars = TextRangeIvars {
            start: std::cell::Cell::new(start),
            end: std::cell::Cell::new(end),
        };
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: `init` is `UITextRange`'s initializer.
        unsafe { msg_send![super(this), init] }
    }

    /// The buffer offsets this range covers.
    #[must_use]
    pub fn offsets(&self) -> (usize, usize) {
        (self.ivars().start.get(), self.ivars().end.get())
    }
}

define_class!(
    // SAFETY: `UIView` asks a subclass to initialize through its designated
    // initializer, which `InputView::new` does, and the class does not
    // implement `Drop`.
    #[unsafe(super(UIView))]
    #[name = "CocoaUiInputView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = InputViewIvars]
    #[derive(Debug)]
    /// A view that claims touch, keyboard and IME input for the content
    /// beneath it.
    pub struct InputView;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIView` subclass.
    unsafe impl NSObjectProtocol for InputView {}

    // SAFETY: `UITextInputTraits` carries only optional selectors.
    unsafe impl UITextInputTraits for InputView {}

    // SAFETY: `UIKeyInput`'s requirements are implemented with their
    // declared signatures.
    unsafe impl UIKeyInput for InputView {
        // SAFETY: plain state read; the pre-edit buffer is the only text the
        // host knows.
        #[unsafe(method(hasText))]
        fn has_text(&self) -> bool {
            self.has_marked_text()
        }

        // SAFETY: `text` is the string UIKit delivers.
        #[unsafe(method(insertText:))]
        fn insert_text(&self, text: &NSString) {
            guarded("InputView insertText:", || {
                let was_composing = self.has_marked_text();
                self.clear_marked_text();
                // Text that ends a composition is that session's commit;
                // text typed outside one is a plain insertion.
                self.emit(if was_composing {
                    SurfaceEvent::CompositionCommit(text.to_string())
                } else {
                    SurfaceEvent::TextInput(text.to_string())
                });
            });
        }

        // SAFETY: plain call.
        #[unsafe(method(deleteBackward))]
        fn delete_backward(&self) {
            guarded("InputView deleteBackward", || {
                // The software keyboard's delete key is a key press, not an
                // edit the host can perform: the surface owns the document.
                for pressed in [true, false] {
                    self.emit(SurfaceEvent::Key {
                        pressed,
                        key: keyboard_types::Key::Named(keyboard_types::NamedKey::Backspace),
                        code: keyboard_types::Code::Backspace,
                        modifiers: keyboard_types::Modifiers::empty(),
                        repeat: false,
                    });
                }
            });
        }
    }

    // SAFETY: `UITextInput`'s required methods are implemented with their
    // declared signatures; the protocol's document model is the pre-edit
    // buffer alone, which the ivars own.
    unsafe impl UITextInput for InputView {
        // SAFETY: `range` is one of this class's `TextRange`s — UIKit
        // passes back only ranges this view handed it.
        #[unsafe(method_id(textInRange:))]
        fn text_in_range(&self, range: &UITextRange) -> Option<Retained<NSString>> {
            downcast_range(range).and_then(|(start, end)| {
                let text = self.ivars().marked_text.borrow();
                text.get(utf16_byte_offset(&text, start)..utf16_byte_offset(&text, end))
                    .map(NSString::from_str)
            })
        }

        // SAFETY: `range` is one of this class's `TextRange`s.
        #[unsafe(method(replaceRange:withText:))]
        fn replace_range_with_text(&self, range: &UITextRange, text: &NSString) {
            let _ = range;
            // The host holds no document to edit; a replacement is the input
            // method rewriting its own pre-edit, which arrives as
            // `setMarkedText:` instead.
            self.insert_marked_text(text);
        }

        // SAFETY: plain state read.
        #[unsafe(method_id(selectedTextRange))]
        fn selected_text_range(&self) -> Option<Retained<UITextRange>> {
            let selection = self.ivars().marked_selection.get();
            // SAFETY: `TextRange::new` runs on the main thread.
            Some(
                TextRange::new(self.mtm(), selection, selection).into_super(),
            )
        }

        // SAFETY: `selected_text_range` is one of this class's `TextRange`s.
        #[unsafe(method(setSelectedTextRange:))]
        fn set_selected_text_range(&self, selected_text_range: Option<&UITextRange>) {
            let Some((start, _)) = selected_text_range.and_then(downcast_range) else {
                return;
            };
            self.ivars().marked_selection.set(start);
        }

        // SAFETY: plain state read.
        #[unsafe(method_id(markedTextRange))]
        fn marked_text_range(&self) -> Option<Retained<UITextRange>> {
            if self.has_marked_text() {
                let length = utf16_len(&self.ivars().marked_text.borrow());
                // SAFETY: `TextRange::new` runs on the main thread.
                Some(TextRange::new(self.mtm(), 0, length).into_super())
            } else {
                None
            }
        }

        // SAFETY: plain state read.
        #[unsafe(method_id(markedTextStyle))]
        fn marked_text_style(&self) -> Option<Retained<NSDictionary<NSAttributedStringKey, AnyObject>>> {
            None
        }

        // SAFETY: `style` is whatever UIKit supplied; the host ignores
        // styling.
        #[unsafe(method(setMarkedTextStyle:))]
        fn set_marked_text_style(
            &self,
            style: Option<&NSDictionary<NSAttributedStringKey, AnyObject>>,
        ) {
            let _ = style;
        }

        // SAFETY: `marked_text` may be null.
        #[unsafe(method(setMarkedText:selectedRange:))]
        fn set_marked_text_selected_range(
            &self,
            marked_text: Option<&NSString>,
            selected_range: NSRange,
        ) {
            guarded("InputView setMarkedText:", || {
                let text = marked_text.map_or_else(String::new, std::string::ToString::to_string);
                let was_composing = self.has_marked_text();
                if text.is_empty() {
                    self.clear_marked_text();
                    if was_composing {
                        self.emit(SurfaceEvent::CompositionCancel);
                    }
                    return;
                }
                if !was_composing {
                    self.emit(SurfaceEvent::CompositionStart);
                }
                let selection =
                    selected_range.location.min(utf16_len(&text));
                self.ivars().marked_text.replace(text.clone());
                self.ivars().marked_selection.set(selection);
                self.emit(SurfaceEvent::CompositionUpdate {
                    caret: composition_caret(&text, selection),
                    text,
                });
            });
        }

        // SAFETY: plain call.
        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            guarded("InputView unmarkText", || {
                if !self.has_marked_text() {
                    return;
                }
                let text = self.ivars().marked_text.borrow().clone();
                self.clear_marked_text();
                self.emit(SurfaceEvent::CompositionCommit(text));
            });
        }

        // SAFETY: `TextPosition::new` runs on the main thread.
        #[unsafe(method_id(beginningOfDocument))]
        fn beginning_of_document(&self) -> Retained<UITextPosition> {
            TextPosition::new(self.mtm(), 0).into_super()
        }

        // SAFETY: `TextPosition::new` runs on the main thread.
        #[unsafe(method_id(endOfDocument))]
        fn end_of_document(&self) -> Retained<UITextPosition> {
            let length = utf16_len(&self.ivars().marked_text.borrow());
            TextPosition::new(self.mtm(), length).into_super()
        }

        // SAFETY: both positions are this class's `TextPosition`s.
        #[unsafe(method_id(textRangeFromPosition:toPosition:))]
        fn text_range_from_position_to_position(
            &self,
            from_position: &UITextPosition,
            to_position: &UITextPosition,
        ) -> Option<Retained<UITextRange>> {
            match (downcast_position(from_position), downcast_position(to_position)) {
                (Some(from), Some(to)) => Some(
                    TextRange::new(self.mtm(), from.min(to), from.max(to)).into_super(),
                ),
                _ => None,
            }
        }

        // SAFETY: `position` is this class's `TextPosition`.
        #[unsafe(method_id(positionFromPosition:offset:))]
        fn position_from_position_offset(
            &self,
            position: &UITextPosition,
            offset: isize,
        ) -> Option<Retained<UITextPosition>> {
            self.offset_position(position, offset)
        }

        // SAFETY: `position` is this class's `TextPosition`.
        #[unsafe(method_id(positionFromPosition:inDirection:offset:))]
        fn position_from_position_in_direction_offset(
            &self,
            position: &UITextPosition,
            direction: UITextLayoutDirection,
            offset: isize,
        ) -> Option<Retained<UITextPosition>> {
            let signed = if direction == UITextLayoutDirection::Left
                || direction == UITextLayoutDirection::Up
            {
                -offset
            } else {
                offset
            };
            self.offset_position(position, signed)
        }

        // SAFETY: both positions are this class's `TextPosition`s.
        #[unsafe(method(comparePosition:toPosition:))]
        fn compare_position_to_position(
            &self,
            position: &UITextPosition,
            other: &UITextPosition,
        ) -> NSComparisonResult {
            let Some(position) = downcast_position(position) else {
                return NSComparisonResult::Same;
            };
            let Some(other) = downcast_position(other) else {
                return NSComparisonResult::Same;
            };
            match position.cmp(&other) {
                std::cmp::Ordering::Less => NSComparisonResult::Ascending,
                std::cmp::Ordering::Equal => NSComparisonResult::Same,
                std::cmp::Ordering::Greater => NSComparisonResult::Descending,
            }
        }

        // SAFETY: both positions are this class's `TextPosition`s.
        #[unsafe(method(offsetFromPosition:toPosition:))]
        fn offset_from_position_to_position(
            &self,
            from_position: &UITextPosition,
            to_position: &UITextPosition,
        ) -> isize {
            let Some(from) = downcast_position(from_position) else {
                return 0;
            };
            let Some(to) = downcast_position(to_position) else {
                return 0;
            };
            to.cast_signed() - from.cast_signed()
        }

        // SAFETY: `input_delegate` is whatever UIKit supplied.
        #[unsafe(method_id(inputDelegate))]
        fn input_delegate(
            &self,
        ) -> Option<Retained<objc2::runtime::ProtocolObject<dyn UITextInputDelegate>>> {
            None
        }

        // SAFETY: `input_delegate` is whatever UIKit supplied.
        #[unsafe(method(setInputDelegate:))]
        fn set_input_delegate(
            &self,
            input_delegate: Option<&objc2::runtime::ProtocolObject<dyn UITextInputDelegate>>,
        ) {
            let _ = input_delegate;
        }

        // SAFETY: `initWithTextInput:` takes the view as its text input.
        #[unsafe(method_id(tokenizer))]
        fn tokenizer_protocol(&self) -> Retained<objc2::runtime::ProtocolObject<dyn UITextInputTokenizer>> {
            let this: &objc2_ui_kit::UIResponder = self;
            // SAFETY: `initWithTextInput:` wires the tokenizer to this input.
            let tokenizer = unsafe {
                UITextInputStringTokenizer::initWithTextInput(
                    UITextInputStringTokenizer::alloc(self.mtm()),
                    this,
                )
            };
            // SAFETY: `UITextInputStringTokenizer` implements
            // `UITextInputTokenizer`.
            objc2::runtime::ProtocolObject::<dyn UITextInputTokenizer>::from_retained(tokenizer)
        }

        // SAFETY: `range` is this class's `TextRange`.
        #[unsafe(method_id(positionWithinRange:farthestInDirection:))]
        fn position_within_range_farthest_in_direction(
            &self,
            range: &UITextRange,
            direction: UITextLayoutDirection,
        ) -> Option<Retained<UITextPosition>> {
            downcast_range(range).map(|(start, end)| {
                let towards_start = direction == UITextLayoutDirection::Left
                    || direction == UITextLayoutDirection::Up;
                // SAFETY: `TextPosition::new` runs on the main thread.
                TextPosition::new(self.mtm(), if towards_start { start } else { end })
                    .into_super()
            })
        }

        // SAFETY: `position` is this class's `TextPosition`.
        #[unsafe(method_id(characterRangeByExtendingPosition:inDirection:))]
        fn character_range_by_extending_position_in_direction(
            &self,
            position: &UITextPosition,
            direction: UITextLayoutDirection,
        ) -> Option<Retained<UITextRange>> {
            downcast_position(position).and_then(|offset| {
                let towards_start = direction == UITextLayoutDirection::Left
                    || direction == UITextLayoutDirection::Up;
                let other = if towards_start {
                    offset.checked_sub(1)?
                } else {
                    offset + 1
                };
                if other > utf16_len(&self.ivars().marked_text.borrow()) {
                    None
                } else {
                    Some(
                        TextRange::new(self.mtm(), offset.min(other), offset.max(other))
                            .into_super(),
                    )
                }
            })
        }

        // SAFETY: plain query; the surface lays out its own text and owns
        // its writing direction.
        #[unsafe(method(baseWritingDirectionForPosition:inDirection:))]
        fn base_writing_direction_for_position_in_direction(
            &self,
            position: &UITextPosition,
            direction: UITextStorageDirection,
        ) -> objc2_ui_kit::NSWritingDirection {
            let _ = (position, direction);
            objc2_ui_kit::NSWritingDirection::Natural
        }

        // SAFETY: `range` is this class's `TextRange`.
        #[unsafe(method(setBaseWritingDirection:forRange:))]
        fn set_base_writing_direction_for_range(
            &self,
            writing_direction: objc2_ui_kit::NSWritingDirection,
            range: &UITextRange,
        ) {
            // The surface lays out its own text and owns its writing
            // direction.
            let _ = (writing_direction, range);
        }

        // SAFETY: `range` is this class's `TextRange`; the caret provider
        // answers in surface-local points, already in this view's coordinate
        // system.
        #[unsafe(method(firstRectForRange:))]
        fn first_rect_for_range(&self, range: &UITextRange) -> objc2_core_foundation::CGRect {
            let _ = range;
            self.caret_rect()
        }

        // SAFETY: `position` is this class's `TextPosition`.
        #[unsafe(method(caretRectForPosition:))]
        fn caret_rect_for_position(&self, position: &UITextPosition) -> objc2_core_foundation::CGRect {
            let _ = position;
            self.caret_rect()
        }

        // SAFETY: `range` is this class's `TextRange`; the host tracks no
        // selection rects.
        #[unsafe(method_id(selectionRectsForRange:))]
        fn selection_rects_for_range(
            &self,
            range: &UITextRange,
        ) -> Retained<NSArray<UITextSelectionRect>> {
            let _ = range;
            NSArray::from_slice(&[])
        }

        // SAFETY: `point` is in this view's coordinates.
        #[unsafe(method_id(closestPositionToPoint:))]
        fn closest_position_to_point(
            &self,
            point: objc2_core_foundation::CGPoint,
        ) -> Option<Retained<UITextPosition>> {
            Some(self.closest_position_for_point(point))
        }

        // SAFETY: `point`/`range` come from UIKit.
        #[unsafe(method_id(closestPositionToPoint:withinRange:))]
        fn closest_position_to_point_within_range(
            &self,
            point: objc2_core_foundation::CGPoint,
            range: &UITextRange,
        ) -> Option<Retained<UITextPosition>> {
            let _ = range;
            Some(self.closest_position_for_point(point))
        }

        // SAFETY: `point` comes from UIKit; no hit-testable document exists.
        #[unsafe(method_id(characterRangeAtPoint:))]
        fn character_range_at_point(
            &self,
            point: objc2_core_foundation::CGPoint,
        ) -> Option<Retained<UITextRange>> {
            let _ = point;
            None
        }
    }

    impl InputView {
        // SAFETY: see the module safety note.
        #[unsafe(method(canBecomeFirstResponder))]
        fn can_become_first_responder_override(&self) -> bool {
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder_override(&self) -> bool {
            // SAFETY: `super(becomeFirstResponder)` forwards to UIView.
            let became: bool = unsafe { msg_send![super(self), becomeFirstResponder] };
            if became {
                self.emit(SurfaceEvent::Focus(true));
            }
            became
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder_override(&self) -> bool {
            // SAFETY: `super(resignFirstResponder)` forwards to UIView.
            let resigned: bool = unsafe { msg_send![super(self), resignFirstResponder] };
            if resigned {
                self.emit(SurfaceEvent::Focus(false));
            }
            resigned
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(touchesBegan:withEvent:))]
        fn touches_began(&self, touches: &NSSet<UITouch>, event: Option<&UIEvent>) {
            let _ = event;
            if !self.isFirstResponder() {
                self.becomeFirstResponder();
            }
            self.send_pointer(touches, None);
            self.send_pointer(touches, Some(true));
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(touchesMoved:withEvent:))]
        fn touches_moved(&self, touches: &NSSet<UITouch>, event: Option<&UIEvent>) {
            let _ = event;
            self.send_pointer(touches, None);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(touchesEnded:withEvent:))]
        fn touches_ended(&self, touches: &NSSet<UITouch>, event: Option<&UIEvent>) {
            let _ = event;
            self.send_pointer(touches, Some(false));
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(touchesCancelled:withEvent:))]
        fn touches_cancelled(&self, touches: &NSSet<UITouch>, event: Option<&UIEvent>) {
            let _ = event;
            self.send_pointer(touches, Some(false));
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(pressesBegan:withEvent:))]
        fn presses_began(&self, presses: &NSSet<UIPress>, event: Option<&UIPressesEvent>) {
            if !self.send_presses(presses, true) {
                // SAFETY: `super(pressesBegan:withEvent:)` forwards to
                // UIResponder's default.
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), pressesBegan: presses, withEvent: event] };
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(pressesEnded:withEvent:))]
        fn presses_ended(&self, presses: &NSSet<UIPress>, event: Option<&UIPressesEvent>) {
            if !self.send_presses(presses, false) {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), pressesEnded: presses, withEvent: event] };
            }
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(pressesCancelled:withEvent:))]
        fn presses_cancelled(&self, presses: &NSSet<UIPress>, event: Option<&UIPressesEvent>) {
            if !self.send_presses(presses, false) {
                // SAFETY: forwards the unhandled cancellation to `UIResponder`.
                let _: () =
                    // SAFETY: `pressesCancelled:withEvent:` is a `UIView` responder method.
                    unsafe { msg_send![super(self), pressesCancelled: presses, withEvent: event] };
            }
        }
    }
);

/// A `UITextPosition` subclass's offset, if `position` is one this view
/// created.
fn downcast_position(position: &UITextPosition) -> Option<usize> {
    position
        .downcast_ref::<TextPosition>()
        .map(TextPosition::offset)
}

/// A `UITextRange` subclass's offsets, if `range` is one this view created.
fn downcast_range(range: &UITextRange) -> Option<(usize, usize)> {
    range.downcast_ref::<TextRange>().map(TextRange::offsets)
}

/// The buffer's length in UTF-16 code units.
fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// The byte offset of `utf16_offset` code units into `text`, saturating at
/// the end.
fn utf16_byte_offset(text: &str, utf16_offset: usize) -> usize {
    let mut units = 0usize;
    let mut byte = 0usize;
    for ch in text.chars() {
        if units >= utf16_offset {
            return byte;
        }
        units += ch.len_utf16();
        byte += ch.len_utf8();
    }
    byte
}

/// The caret's byte offset into the pre-edit text.
fn composition_caret(text: &str, utf16_offset: usize) -> Option<usize> {
    if utf16_offset > utf16_len(text) {
        return None;
    }
    Some(utf16_byte_offset(text, utf16_offset))
}

impl InputView {
    /// An input responder, forwarding every surface event to the installed
    /// handler.
    ///
    /// The content's own view places it on top of its pixels — covering the
    /// same frame — so hits land here while the content keeps rendering.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(InputViewIvars::default());
        // SAFETY: `initWithFrame:` is `UIView`'s designated initializer.
        unsafe { msg_send![super(this), initWithFrame: objc2_core_foundation::CGRect::ZERO] }
    }

    /// Installs `handler` as the receiver of every surface event.
    pub fn set_event_handler(&self, handler: impl Fn(SurfaceEvent) + 'static) {
        self.ivars().event_handler.replace(Some(Rc::new(handler)));
    }

    /// Installs `provider` as the source of the content's text caret, in
    /// logical surface-local points.
    pub fn set_caret_provider(&self, provider: impl Fn() -> Option<Rect> + 'static) {
        self.ivars().caret_provider.replace(Some(Rc::new(provider)));
    }

    fn insert_marked_text(&self, text: &NSString) {
        self.insert_text(sel!(insertText:), text);
    }

    fn offset_position(
        &self,
        position: &UITextPosition,
        offset: isize,
    ) -> Option<Retained<UITextPosition>> {
        match downcast_position(position).and_then(|start| start.checked_add_signed(offset)) {
            Some(moved) if moved <= utf16_len(&self.ivars().marked_text.borrow()) => {
                Some(TextPosition::new(self.mtm(), moved).into_super())
            }
            _ => None,
        }
    }

    fn closest_position_for_point(
        &self,
        point: objc2_core_foundation::CGPoint,
    ) -> Retained<UITextPosition> {
        let _ = point;
        let selection = self.ivars().marked_selection.get();
        TextPosition::new(self.mtm(), selection).into_super()
    }

    fn emit(&self, event: SurfaceEvent) {
        let handler = self.ivars().event_handler.borrow().clone();
        if let Some(handler) = handler {
            handler(event);
        }
    }

    fn caret_rect(&self) -> objc2_core_foundation::CGRect {
        self.ivars()
            .caret_provider
            .borrow()
            .as_ref()
            .and_then(|provider| provider())
            .map_or_else(
                || self.bounds(),
                |rect| {
                    objc2_core_foundation::CGRect::new(
                        objc2_core_foundation::CGPoint::new(rect.origin.x, rect.origin.y),
                        objc2_core_foundation::CGSize::new(rect.size.width, rect.size.height),
                    )
                },
            )
    }

    fn has_marked_text(&self) -> bool {
        !self.ivars().marked_text.borrow().is_empty()
    }

    fn clear_marked_text(&self) {
        self.ivars().marked_text.borrow_mut().clear();
        self.ivars().marked_selection.set(0);
    }

    /// `UIKit`'s first touch as a pointer event: every touch is primary.
    fn send_pointer(&self, touches: &NSSet<UITouch>, pressed: Option<bool>) {
        let Some(touch) = touches.into_iter().next() else {
            return;
        };
        let this: &UIView = self;
        let point = touch.locationInView(Some(this));
        let position = kurbo::Point::new(point.x, point.y);
        self.emit(
            pressed.map_or(SurfaceEvent::PointerMove { position }, |pressed| {
                SurfaceEvent::PointerButton {
                    pressed,
                    button: PointerButton::Primary,
                    position,
                }
            }),
        );
    }

    /// A press batch as `Modifiers` + `Key` events, one pair per key;
    /// whether every press delivered an event.
    fn send_presses(&self, press_set: &NSSet<UIPress>, pressed: bool) -> bool {
        let mut delivered = false;
        for press_item in press_set {
            let Some(key) = press_item.key(self.mtm()) else {
                continue;
            };
            let modifiers = keys::surface_modifiers(key.modifierFlags());
            self.emit(SurfaceEvent::Modifiers(modifiers));
            let code = keys::surface_code(&key);
            self.emit(SurfaceEvent::Key {
                pressed,
                key: keys::surface_key(&key),
                code,
                modifiers,
                repeat: false,
            });
            delivered = true;
        }
        delivered
    }
}
