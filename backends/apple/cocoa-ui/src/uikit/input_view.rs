//! The first responder for a view that takes its own input.
//!
//! An [`InputView`] sits on top of the view whose pixels host interactive
//! content — a GPU surface, a text editor — claims touch, keyboard and
//! indirect-scroll input, and speaks `UITextInput` against the content's
//! document, a [`TextInputClient`]. Pointer, key and scroll input is
//! translated into the platform-neutral [`SurfaceEvent`] vocabulary and
//! handed to the installed handler; text input never is — the view applies
//! it to the client's document.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UIView` subclass implementing `UITextInput`
//! plus the abstract `UITextPosition`/`UITextRange`/`UITextSelectionRect`
//! subclasses the protocol requires. `UIKit` calls every override on the
//! main thread; the document lives in the installed client, never in the
//! view.

use std::cell::RefCell;
use std::fmt;
use std::ops::Range;
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{
    NSArray, NSAttributedStringKey, NSComparisonResult, NSDictionary, NSNumber, NSObject, NSRange,
    NSSet, NSString,
};
use objc2_ui_kit::{
    UIEvent, UIEventButtonMask, UIGestureRecognizer, UIGestureRecognizerDelegate,
    UIGestureRecognizerState, UIHoverGestureRecognizer, UIKeyInput, UIKeyboardType,
    UIPanGestureRecognizer, UIPinchGestureRecognizer, UIPress, UIPressesEvent, UIReturnKeyType,
    UIRotationGestureRecognizer, UIScrollTypeMask, UITextAutocapitalizationType,
    UITextAutocorrectionType, UITextInput, UITextInputDelegate, UITextInputStringTokenizer,
    UITextInputTokenizer, UITextInputTraits, UITextLayoutDirection, UITextPosition, UITextRange,
    UITextSelectionRect, UITextStorageDirection, UITouch, UITouchType, UIView,
};

use super::pointer::{CursorInteraction, PointerCursor};
use super::surface_view::gesture_phase;
use super::text_input::{
    Autocapitalization, Autocorrection, KeyboardType, ReturnKey, TextInputClient,
    TextInputNotifier, next_grapheme, previous_grapheme,
};
use crate::callback::guarded;
use crate::geometry::{EdgeInsets, Rect};
use crate::input::{
    GesturePhase, Key, NamedKey, PointerButtons, ScrollUnit, SurfaceEvent, TouchKind, TouchPhase,
};
use crate::keys;

/// Delivers one surface event to the content.
type EventHandler = Rc<dyn Fn(SurfaceEvent)>;

/// Receives the view's new safe-area insets.
type SafeAreaHandler = Rc<dyn Fn(EdgeInsets)>;

/// The ivars of an [`InputView`].
#[derive(Default)]
pub struct InputViewIvars {
    event_handler: RefCell<Option<EventHandler>>,
    /// The document `UITextInput` edits; `None` is an empty, non-editing
    /// document.
    client: RefCell<Option<Rc<dyn TextInputClient>>>,
    /// The input method's change delegate. `UIKit` declares it `weak`.
    input_delegate: RefCell<Option<Weak<ProtocolObject<dyn UITextInputDelegate>>>>,
    layout_handler: RefCell<Option<Rc<dyn Fn()>>>,
    safe_area_handler: RefCell<Option<SafeAreaHandler>>,
    /// The target the pointer recognizers talk to — the view owns it.
    pointer_target: RefCell<Option<Retained<PointerTarget>>>,
    /// The pointer recognizers: the indirect-scroll pair — continuous
    /// (trackpad) and discrete (wheel) so each callback knows its gesture's
    /// source and completion behavior, both in logical points — plus hover
    /// and the trackpad pinch and rotation.
    pointer_recognizers: RefCell<Vec<Retained<UIGestureRecognizer>>>,
    /// The interaction that shows the content's cursor.
    cursor: RefCell<Option<CursorInteraction>>,
    /// The empty input view answered while no editor is focused, created on
    /// first use.
    silent_input: RefCell<Option<Retained<UIView>>>,
}

impl fmt::Debug for InputViewIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InputViewIvars")
            .field("event_handler", &self.event_handler.borrow().is_some())
            .field("client", &self.client.borrow().is_some())
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
    /// A position in the client's document, counted in UTF-16 code units.
    pub struct TextPosition;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UITextPosition`
    // subclass.
    unsafe impl NSObjectProtocol for TextPosition {}
);

impl TextPosition {
    /// A text position at `offset` UTF-16 code units into the document.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, offset: usize) -> Retained<Self> {
        let ivars = TextPositionIvars::default();
        ivars.offset.set(offset);
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: `init` is `UITextPosition`'s initializer.
        unsafe { msg_send![super(this), init] }
    }

    /// The offset this position counts from the document's start.
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
    /// A range of the client's document, counted in UTF-16 code units.
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

    /// The document offsets this range covers.
    #[must_use]
    pub fn offsets(&self) -> (usize, usize) {
        (self.ivars().start.get(), self.ivars().end.get())
    }
}

/// The ivars of a [`SelectionRect`].
#[derive(Debug)]
pub struct SelectionRectIvars {
    rect: objc2_core_foundation::CGRect,
    contains_start: bool,
    contains_end: bool,
}

define_class!(
    // SAFETY: `UITextSelectionRect` is an abstract base whose geometry
    // properties the subclass supplies from its ivars.
    #[unsafe(super(UITextSelectionRect))]
    #[name = "CocoaUiTextSelectionRect"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SelectionRectIvars]
    #[derive(Debug)]
    /// One line's rectangle of a selection.
    pub struct SelectionRect;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UITextSelectionRect`
    // subclass.
    unsafe impl NSObjectProtocol for SelectionRect {}

    impl SelectionRect {
        // SAFETY: plain state read.
        #[unsafe(method(rect))]
        fn rect_override(&self) -> objc2_core_foundation::CGRect {
            self.ivars().rect
        }

        // SAFETY: plain state read; the surface owns its writing direction.
        #[unsafe(method(writingDirection))]
        fn writing_direction_override(&self) -> objc2_ui_kit::NSWritingDirection {
            objc2_ui_kit::NSWritingDirection::Natural
        }

        // SAFETY: plain state read.
        #[unsafe(method(containsStart))]
        fn contains_start_override(&self) -> bool {
            self.ivars().contains_start
        }

        // SAFETY: plain state read.
        #[unsafe(method(containsEnd))]
        fn contains_end_override(&self) -> bool {
            self.ivars().contains_end
        }

        // SAFETY: plain state read; selection rects are horizontal lines.
        #[unsafe(method(isVertical))]
        fn is_vertical_override(&self) -> bool {
            false
        }
    }
);

impl SelectionRect {
    fn new(
        mtm: MainThreadMarker,
        rect: Rect,
        contains_start: bool,
        contains_end: bool,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SelectionRectIvars {
            rect: cg_rect(rect),
            contains_start,
            contains_end,
        });
        // SAFETY: `init` is `UITextSelectionRect`'s initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// The ivars of a [`PointerTarget`].
#[derive(Default)]
struct PointerTargetIvars {
    /// The view the pointer recognizers forward to — weak, since the view
    /// owns both the recognizers and the target.
    view: RefCell<Option<Weak<InputView>>>,
}

impl fmt::Debug for PointerTargetIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PointerTargetIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements; the class holds
    // a weak view reference and does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiInputPointerTarget"]
    #[thread_kind = MainThreadOnly]
    #[ivars = PointerTargetIvars]
    #[derive(Debug)]
    /// Receives the pointer recognizers' actions for an [`InputView`].
    /// `UIKit` delivers trackpad and wheel scrolling to a view as
    /// `UIPanGestureRecognizer`s whose `allowedScrollTypesMask` admits the
    /// scroll types — one recognizer per type, so each callback knows its
    /// gesture's source and completion behavior. Both report logical
    /// points, never a mask-selected unit. Hover, and the trackpad's pinch
    /// and rotation, arrive through their own recognizers.
    struct PointerTarget;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for PointerTarget {}

    // SAFETY: `UIGestureRecognizerDelegate`'s method is implemented with its
    // declared signature.
    unsafe impl UIGestureRecognizerDelegate for PointerTarget {
        // SAFETY: both recognizers are UIKit's.
        #[unsafe(method(gestureRecognizer:shouldRecognizeSimultaneouslyWithGestureRecognizer:))]
        fn gesture_recognizer_should_recognize_simultaneously_with_gesture_recognizer(
            &self,
            _gesture_recognizer: &UIGestureRecognizer,
            _other_gesture_recognizer: &UIGestureRecognizer,
        ) -> bool {
            // A trackpad pinch and rotation are one two-finger motion.
            true
        }
    }

    impl PointerTarget {
        // SAFETY: `scrollContinuous:` is the `UIGestureRecognizer` action
        // signature.
        #[unsafe(method(scrollContinuous:))]
        fn scroll_continuous(&self, gesture: &UIPanGestureRecognizer) {
            guarded("CocoaUiInputPointerTarget scrollContinuous:", || {
                if let Some(view) = self.ivars().view.borrow().as_ref().and_then(Weak::load)
                    && let Some(phase) = gesture_phase(gesture.state())
                {
                    view.handle_trackpad_pan(gesture, phase);
                }
            });
        }

        // SAFETY: `scrollDiscrete:` is the `UIGestureRecognizer` action
        // signature.
        #[unsafe(method(scrollDiscrete:))]
        fn scroll_discrete(&self, gesture: &UIPanGestureRecognizer) {
            guarded("CocoaUiInputPointerTarget scrollDiscrete:", || {
                if let Some(view) = self.ivars().view.borrow().as_ref().and_then(Weak::load) {
                    view.handle_scroll(gesture);
                }
            });
        }

        // SAFETY: `hover:` is the `UIGestureRecognizer` action signature.
        #[unsafe(method(hover:))]
        fn hover(&self, gesture: &UIHoverGestureRecognizer) {
            guarded("CocoaUiInputPointerTarget hover:", || {
                if let Some(view) = self.ivars().view.borrow().as_ref().and_then(Weak::load) {
                    view.handle_hover(gesture);
                }
            });
        }

        // SAFETY: `pinch:` is the `UIGestureRecognizer` action signature.
        #[unsafe(method(pinch:))]
        fn pinch(&self, gesture: &UIPinchGestureRecognizer) {
            guarded("CocoaUiInputPointerTarget pinch:", || {
                if let Some(view) = self.ivars().view.borrow().as_ref().and_then(Weak::load)
                    && let Some(phase) = gesture_phase(gesture.state())
                {
                    view.handle_magnification(gesture, phase);
                }
            });
        }

        // SAFETY: `rotate:` is the `UIGestureRecognizer` action signature.
        #[unsafe(method(rotate:))]
        fn rotate(&self, gesture: &UIRotationGestureRecognizer) {
            guarded("CocoaUiInputPointerTarget rotate:", || {
                if let Some(view) = self.ivars().view.borrow().as_ref().and_then(Weak::load)
                    && let Some(phase) = gesture_phase(gesture.state())
                {
                    view.handle_rotation(gesture, phase);
                }
            });
        }
    }
);

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

    // SAFETY: `UITextInputTraits` carries only optional selectors; the
    // ones this view answers are implemented in the `InputView` block
    // below with the protocol's declared signatures.
    unsafe impl UITextInputTraits for InputView {}

    // SAFETY: `UIKeyInput`'s requirements are implemented with their
    // declared signatures.
    unsafe impl UIKeyInput for InputView {
        // SAFETY: plain state read against the client.
        #[unsafe(method(hasText))]
        fn has_text(&self) -> bool {
            self.client()
                .is_some_and(|client| client.is_editing() && !client.is_empty())
        }

        // SAFETY: `text` is the string UIKit delivers.
        #[unsafe(method(insertText:))]
        fn insert_text(&self, text: &NSString) {
            guarded("InputView insertText:", || {
                let Some(client) = self.editing_client() else {
                    return;
                };
                let text = text.to_string();
                // The return key arrives as a newline; what it means —
                // a line break or a submit — is the editor's call.
                if text == "\n" {
                    client.perform_return();
                    return;
                }
                let range = client
                    .marked_range()
                    .unwrap_or_else(|| client.selection());
                client.replace(range, &text);
            });
        }

        // SAFETY: plain call.
        #[unsafe(method(deleteBackward))]
        fn delete_backward(&self) {
            guarded("InputView deleteBackward", || {
                let Some(client) = self.editing_client() else {
                    return;
                };
                client.delete_backward();
            });
        }
    }

    // SAFETY: `UITextInput`'s required methods are implemented with their
    // declared signatures; the document is the installed client's.
    unsafe impl UITextInput for InputView {
        // SAFETY: `range` is one of this class's `TextRange`s — UIKit
        // passes back only ranges this view handed it.
        #[unsafe(method_id(textInRange:))]
        fn text_in_range(&self, range: &UITextRange) -> Option<Retained<NSString>> {
            self.text_in(range)
        }

        // SAFETY: `range` is one of this class's `TextRange`s.
        #[unsafe(method(replaceRange:withText:))]
        fn replace_range_with_text(&self, range: &UITextRange, text: &NSString) {
            guarded("InputView replaceRange:withText:", || {
                let (Some(client), Some(range)) = (self.client(), downcast_range(range)) else {
                    return;
                };
                client.replace(clamped(range, client.len()), &text.to_string());
            });
        }

        // SAFETY: plain state read.
        #[unsafe(method_id(selectedTextRange))]
        fn selected_text_range(&self) -> Option<Retained<UITextRange>> {
            let selection = self.client().map_or(0..0, |client| client.selection());
            Some(self.range(selection))
        }

        // SAFETY: `selected_text_range` is one of this class's `TextRange`s.
        #[unsafe(method(setSelectedTextRange:))]
        fn set_selected_text_range(&self, selected_text_range: Option<&UITextRange>) {
            guarded("InputView setSelectedTextRange:", || {
                let (Some(client), Some(range)) =
                    (self.client(), selected_text_range.and_then(downcast_range))
                else {
                    return;
                };
                client.set_selection(clamped(range, client.len()));
            });
        }

        // SAFETY: plain state read.
        #[unsafe(method_id(markedTextRange))]
        fn marked_text_range(&self) -> Option<Retained<UITextRange>> {
            self.marked_text_range_value()
        }

        // SAFETY: plain state read.
        #[unsafe(method_id(markedTextStyle))]
        fn marked_text_style(&self) -> Option<Retained<NSDictionary<NSAttributedStringKey, AnyObject>>> {
            None
        }

        // SAFETY: `style` is whatever UIKit supplied; the content draws its
        // own marked-text styling.
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
                let Some(client) = self.editing_client() else {
                    return;
                };
                let text = marked_text.map_or_else(String::new, ToString::to_string);
                if text.is_empty() {
                    // An empty composition abandons it.
                    if let Some(marked) = client.marked_range() {
                        client.replace(marked, "");
                    }
                    return;
                }
                let length = NSString::from_str(&text).length();
                let start = selected_range.location.min(length);
                let end = selected_range
                    .location
                    .saturating_add(selected_range.length)
                    .min(length);
                client.set_marked_text(&text, start..end);
            });
        }

        // SAFETY: plain call.
        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            guarded("InputView unmarkText", || self.unmark_document());
        }

        // SAFETY: `TextPosition::new` runs on the main thread.
        #[unsafe(method_id(beginningOfDocument))]
        fn beginning_of_document(&self) -> Retained<UITextPosition> {
            self.position(0)
        }

        // SAFETY: `TextPosition::new` runs on the main thread.
        #[unsafe(method_id(endOfDocument))]
        fn end_of_document(&self) -> Retained<UITextPosition> {
            self.position(self.document_len())
        }

        // SAFETY: both positions are this class's `TextPosition`s.
        #[unsafe(method_id(textRangeFromPosition:toPosition:))]
        fn text_range_from_position_to_position(
            &self,
            from_position: &UITextPosition,
            to_position: &UITextPosition,
        ) -> Option<Retained<UITextRange>> {
            match (downcast_position(from_position), downcast_position(to_position)) {
                (Some(from), Some(to)) => Some(self.range(from.min(to)..from.max(to))),
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
            let signed = if towards_start(direction) { -offset } else { offset };
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

        // SAFETY: plain state read.
        #[unsafe(method_id(inputDelegate))]
        fn input_delegate_protocol(
            &self,
        ) -> Option<Retained<ProtocolObject<dyn UITextInputDelegate>>> {
            self.delegate()
        }

        // SAFETY: `input_delegate` is whatever UIKit supplied; it is held
        // weakly, as the property is declared `weak`.
        #[unsafe(method(setInputDelegate:))]
        fn set_input_delegate(
            &self,
            input_delegate: Option<&ProtocolObject<dyn UITextInputDelegate>>,
        ) {
            self.ivars()
                .input_delegate
                .replace(input_delegate.map(Weak::new));
        }

        // SAFETY: `initWithTextInput:` takes the view as its text input.
        #[unsafe(method_id(tokenizer))]
        fn tokenizer_protocol(&self) -> Retained<ProtocolObject<dyn UITextInputTokenizer>> {
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
            ProtocolObject::<dyn UITextInputTokenizer>::from_retained(tokenizer)
        }

        // SAFETY: `range` is this class's `TextRange`.
        #[unsafe(method_id(positionWithinRange:farthestInDirection:))]
        fn position_within_range_farthest_in_direction(
            &self,
            range: &UITextRange,
            direction: UITextLayoutDirection,
        ) -> Option<Retained<UITextPosition>> {
            downcast_range(range).map(|(start, end)| {
                self.position(if towards_start(direction) { start } else { end })
            })
        }

        // SAFETY: `position` is this class's `TextPosition`.
        #[unsafe(method_id(characterRangeByExtendingPosition:inDirection:))]
        fn character_range_by_extending_position_in_direction(
            &self,
            position: &UITextPosition,
            direction: UITextLayoutDirection,
        ) -> Option<Retained<UITextRange>> {
            self.extended_range(position, direction)
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

        // SAFETY: `range` is this class's `TextRange`; the client answers
        // in this view's own coordinate system.
        #[unsafe(method(firstRectForRange:))]
        fn first_rect_for_range(&self, range: &UITextRange) -> objc2_core_foundation::CGRect {
            guarded("InputView firstRectForRange:", || {
                match (self.client(), downcast_range(range)) {
                    (Some(client), Some(range)) => {
                        cg_rect(client.first_rect(clamped(range, client.len())))
                    }
                    _ => self.bounds(),
                }
            })
        }

        // SAFETY: `position` is this class's `TextPosition`.
        #[unsafe(method(caretRectForPosition:))]
        fn caret_rect_for_position(&self, position: &UITextPosition) -> objc2_core_foundation::CGRect {
            guarded("InputView caretRectForPosition:", || {
                match (self.client(), downcast_position(position)) {
                    (Some(client), Some(offset)) => {
                        cg_rect(client.caret_rect(offset.min(client.len())))
                    }
                    _ => self.bounds(),
                }
            })
        }

        // SAFETY: `range` is this class's `TextRange`.
        #[unsafe(method_id(selectionRectsForRange:))]
        fn selection_rects_for_range(
            &self,
            range: &UITextRange,
        ) -> Retained<NSArray<UITextSelectionRect>> {
            guarded("InputView selectionRectsForRange:", || {
                let (Some(client), Some(range)) = (self.client(), downcast_range(range)) else {
                    return NSArray::from_slice(&[]);
                };
                let rects = client.selection_rects(clamped(range, client.len()));
                let last = rects.len().saturating_sub(1);
                let rects: Vec<Retained<UITextSelectionRect>> = rects
                    .into_iter()
                    .enumerate()
                    .map(|(index, rect)| {
                        SelectionRect::new(self.mtm(), rect, index == 0, index == last)
                            .into_super()
                    })
                    .collect();
                NSArray::from_retained_slice(&rects)
            })
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
            self.closest_position_within(point, range)
        }

        // SAFETY: `point` comes from UIKit.
        #[unsafe(method_id(characterRangeAtPoint:))]
        fn character_range_at_point(
            &self,
            point: objc2_core_foundation::CGPoint,
        ) -> Option<Retained<UITextRange>> {
            self.character_range_near(point)
        }
    }

    impl InputView {
        // SAFETY: plain state read; the keyboard layout the client asks for.
        #[unsafe(method(keyboardType))]
        fn keyboard_type_override(&self) -> UIKeyboardType {
            match self.traits().keyboard_type {
                KeyboardType::Default => UIKeyboardType::Default,
                KeyboardType::Ascii => UIKeyboardType::ASCIICapable,
                KeyboardType::NumbersAndPunctuation => UIKeyboardType::NumbersAndPunctuation,
                KeyboardType::Url => UIKeyboardType::URL,
                KeyboardType::NumberPad => UIKeyboardType::NumberPad,
                KeyboardType::PhonePad => UIKeyboardType::PhonePad,
                KeyboardType::NamePhonePad => UIKeyboardType::NamePhonePad,
                KeyboardType::EmailAddress => UIKeyboardType::EmailAddress,
                KeyboardType::DecimalPad => UIKeyboardType::DecimalPad,
                KeyboardType::Twitter => UIKeyboardType::Twitter,
                KeyboardType::WebSearch => UIKeyboardType::WebSearch,
                KeyboardType::AsciiNumberPad => UIKeyboardType::ASCIICapableNumberPad,
            }
        }

        // SAFETY: plain state read.
        #[unsafe(method(returnKeyType))]
        fn return_key_type_override(&self) -> UIReturnKeyType {
            match self.traits().return_key {
                ReturnKey::Default => UIReturnKeyType::Default,
                ReturnKey::Go => UIReturnKeyType::Go,
                ReturnKey::Google => UIReturnKeyType::Google,
                ReturnKey::Join => UIReturnKeyType::Join,
                ReturnKey::Next => UIReturnKeyType::Next,
                ReturnKey::Route => UIReturnKeyType::Route,
                ReturnKey::Search => UIReturnKeyType::Search,
                ReturnKey::Send => UIReturnKeyType::Send,
                ReturnKey::Yahoo => UIReturnKeyType::Yahoo,
                ReturnKey::Done => UIReturnKeyType::Done,
                ReturnKey::EmergencyCall => UIReturnKeyType::EmergencyCall,
                ReturnKey::Continue => UIReturnKeyType::Continue,
            }
        }

        // SAFETY: plain state read.
        #[unsafe(method(isSecureTextEntry))]
        fn is_secure_text_entry_override(&self) -> bool {
            self.traits().secure
        }

        // SAFETY: plain state read.
        #[unsafe(method(autocorrectionType))]
        fn autocorrection_type_override(&self) -> UITextAutocorrectionType {
            match self.traits().autocorrection {
                Autocorrection::Default => UITextAutocorrectionType::Default,
                Autocorrection::Disabled => UITextAutocorrectionType::No,
                Autocorrection::Enabled => UITextAutocorrectionType::Yes,
            }
        }

        // SAFETY: plain state read.
        #[unsafe(method(autocapitalizationType))]
        fn autocapitalization_type_override(&self) -> UITextAutocapitalizationType {
            match self.traits().autocapitalization {
                Autocapitalization::None => UITextAutocapitalizationType::None,
                Autocapitalization::Words => UITextAutocapitalizationType::Words,
                Autocapitalization::Sentences => UITextAutocapitalizationType::Sentences,
                Autocapitalization::AllCharacters => UITextAutocapitalizationType::AllCharacters,
            }
        }

        // SAFETY: plain state read; `nil` is a valid content type.
        #[unsafe(method_id(textContentType))]
        fn text_content_type_override(&self) -> Option<Retained<NSString>> {
            self.traits()
                .content_type
                .map(|content_type| Retained::from(content_type.native()))
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(canBecomeFirstResponder))]
        fn can_become_first_responder_override(&self) -> bool {
            true
        }

        // SAFETY: plain state read; `nil` asks for the system keyboard.
        #[unsafe(method_id(inputView))]
        fn input_view_override(&self) -> Option<Retained<UIView>> {
            // The view is the first responder for hardware keys and touch
            // whenever it is on screen, but the soft keyboard belongs to a
            // focused editor: with none, the input view is an empty one, so
            // the keyboard has nothing to show. `reset` reloads the input
            // views when the focused document changes.
            if self.editing_client().is_some() {
                None
            } else {
                let mut silent = self.ivars().silent_input.borrow_mut();
                Some(
                    silent
                        .get_or_insert_with(|| {
                            UIView::initWithFrame(
                                UIView::alloc(self.mtm()),
                                objc2_core_foundation::CGRect::ZERO,
                            )
                        })
                        .clone(),
                )
            }
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
        #[unsafe(method(layoutSubviews))]
        fn layout_subviews_override(&self) {
            guarded("InputView layoutSubviews", || {
                // SAFETY: `super(layoutSubviews)` forwards to UIView.
                let _: () = unsafe { msg_send![super(self), layoutSubviews] };
                let handler = self.ivars().layout_handler.borrow().clone();
                if let Some(handler) = handler {
                    handler();
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(safeAreaInsetsDidChange))]
        fn safe_area_insets_did_change_override(&self) {
            guarded("InputView safeAreaInsetsDidChange", || {
                // SAFETY: `super(safeAreaInsetsDidChange)` forwards to UIView.
                let _: () = unsafe { msg_send![super(self), safeAreaInsetsDidChange] };
                let handler = self.ivars().safe_area_handler.borrow().clone();
                if let Some(handler) = handler {
                    handler(self.safe_area_insets());
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(touchesBegan:withEvent:))]
        fn touches_began(&self, touches: &NSSet<UITouch>, event: Option<&UIEvent>) {
            guarded("InputView touchesBegan:", || {
                if !self.isFirstResponder() {
                    self.becomeFirstResponder();
                }
                self.send_touches(touches, event, TouchPhase::Began);
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(touchesMoved:withEvent:))]
        fn touches_moved(&self, touches: &NSSet<UITouch>, event: Option<&UIEvent>) {
            guarded("InputView touchesMoved:", || {
                self.send_touches(touches, event, TouchPhase::Moved);
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(touchesEnded:withEvent:))]
        fn touches_ended(&self, touches: &NSSet<UITouch>, event: Option<&UIEvent>) {
            guarded("InputView touchesEnded:", || {
                self.send_touches(touches, event, TouchPhase::Ended);
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(touchesCancelled:withEvent:))]
        fn touches_cancelled(&self, touches: &NSSet<UITouch>, event: Option<&UIEvent>) {
            guarded("InputView touchesCancelled:", || {
                self.send_touches(touches, event, TouchPhase::Cancelled);
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(pressesBegan:withEvent:))]
        fn presses_began(&self, presses: &NSSet<UIPress>, event: Option<&UIPressesEvent>) {
            guarded("InputView pressesBegan:", || {
                let forwarded = self.send_presses(presses, PressPhase::Began);
                if !forwarded.is_empty() {
                    let forwarded = NSSet::from_retained_slice(&forwarded);
                    // SAFETY: `super(pressesBegan:withEvent:)` forwards to
                    // UIResponder's default.
                    unsafe {
                        let _: () = msg_send![super(self), pressesBegan: &*forwarded, withEvent: event];
                    }
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(pressesEnded:withEvent:))]
        fn presses_ended(&self, presses: &NSSet<UIPress>, event: Option<&UIPressesEvent>) {
            guarded("InputView pressesEnded:", || {
                let forwarded = self.send_presses(presses, PressPhase::Ended);
                if !forwarded.is_empty() {
                    let forwarded = NSSet::from_retained_slice(&forwarded);
                    // SAFETY: see `pressesBegan:withEvent:`.
                    unsafe {
                        let _: () = msg_send![super(self), pressesEnded: &*forwarded, withEvent: event];
                    }
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(pressesCancelled:withEvent:))]
        fn presses_cancelled(&self, presses: &NSSet<UIPress>, event: Option<&UIPressesEvent>) {
            guarded("InputView pressesCancelled:", || {
                let forwarded = self.send_presses(presses, PressPhase::Cancelled);
                if !forwarded.is_empty() {
                    let forwarded = NSSet::from_retained_slice(&forwarded);
                    // SAFETY: see `pressesBegan:withEvent:`.
                    unsafe {
                        let _: () = msg_send![super(self), pressesCancelled: &*forwarded, withEvent: event];
                    }
                }
            });
        }
    }
);

/// The phase of a hardware-key press batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PressPhase {
    Began,
    Ended,
    Cancelled,
}

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

/// `(start, end)` ordered and clamped into a document of `len` code units.
fn clamped((start, end): (usize, usize), len: usize) -> Range<usize> {
    let (start, end) = (start.min(end).min(len), start.max(end).min(len));
    start..end
}

/// Whether `direction` reads towards the document's start.
fn towards_start(direction: UITextLayoutDirection) -> bool {
    direction == UITextLayoutDirection::Left || direction == UITextLayoutDirection::Up
}

const fn cg_rect(rect: Rect) -> objc2_core_foundation::CGRect {
    objc2_core_foundation::CGRect::new(
        objc2_core_foundation::CGPoint::new(rect.origin.x, rect.origin.y),
        objc2_core_foundation::CGSize::new(rect.size.width, rect.size.height),
    )
}

/// Whether the text system, not the content, handles `key` while an editor
/// is focused: caret movement, deletion and the return key, with or without
/// modifiers.
const fn is_text_system_key(key: &Key) -> bool {
    matches!(
        key,
        Key::Named(
            NamedKey::ArrowLeft
                | NamedKey::ArrowRight
                | NamedKey::ArrowUp
                | NamedKey::ArrowDown
                | NamedKey::Home
                | NamedKey::End
                | NamedKey::PageUp
                | NamedKey::PageDown
                | NamedKey::Backspace
                | NamedKey::Delete
                | NamedKey::Enter
        )
    )
}

/// The `UIKit` touch kind as the surface vocabulary's.
///
/// # Panics
///
/// Panics on `UITouchTypeIndirect`, the Apple TV remote, which no
/// supported platform produces.
fn touch_kind(touch_type: UITouchType) -> TouchKind {
    match touch_type {
        UITouchType::Direct => TouchKind::Direct,
        UITouchType::Pencil => TouchKind::Pencil,
        UITouchType::IndirectPointer => TouchKind::IndirectPointer,
        other => panic!("unsupported touch type {other:?}"),
    }
}

/// The held buttons `UIEvent.buttonMask` reports. Buttons past the
/// secondary have no stable `UIKit` meaning and are not delivered.
fn pointer_buttons(mask: UIEventButtonMask) -> PointerButtons {
    let mut buttons = PointerButtons::empty();
    if mask.contains(UIEventButtonMask::Primary) {
        buttons |= PointerButtons::PRIMARY;
    }
    if mask.contains(UIEventButtonMask::Secondary) {
        buttons |= PointerButtons::SECONDARY;
    }
    buttons
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
        let view: Retained<Self> =
            unsafe { msg_send![super(this), initWithFrame: objc2_core_foundation::CGRect::ZERO] };

        // Indirect scrolling reaches a view as pan gestures: a trackpad
        // glide arrives through the `Continuous` scroll type, a wheel
        // notch through `Discrete` — one recognizer each, so the action
        // knows the gesture's source and completion behavior. The reported
        // translation is always a logical-point displacement; the mask
        // never selects a unit. `maximumNumberOfTouches = 0` keeps a
        // finger drag out of the scroll channel: a touch pan is pointer
        // input, not a scroll.
        let target: Retained<PointerTarget> = {
            let this = PointerTarget::alloc(mtm).set_ivars(PointerTargetIvars::default());
            // SAFETY: `init` is `NSObject`'s designated initializer.
            unsafe { msg_send![super(this), init] }
        };
        target.ivars().view.replace(Some(Weak::new(&*view)));
        // SAFETY: `initWithTarget:action:` retains the recognizer↔target
        // pair; `setAllowedScrollTypesMask:` is `UIPanGestureRecognizer`'s
        // declared scroll-type setter.
        unsafe {
            let continuous = UIPanGestureRecognizer::initWithTarget_action(
                UIPanGestureRecognizer::alloc(mtm),
                Some(&*target),
                Some(sel!(scrollContinuous:)),
            );
            continuous.setAllowedScrollTypesMask(UIScrollTypeMask::Continuous);
            continuous.setMaximumNumberOfTouches(0);
            continuous.setCancelsTouchesInView(false);
            view.addGestureRecognizer(&continuous);

            let discrete = UIPanGestureRecognizer::initWithTarget_action(
                UIPanGestureRecognizer::alloc(mtm),
                Some(&*target),
                Some(sel!(scrollDiscrete:)),
            );
            discrete.setAllowedScrollTypesMask(UIScrollTypeMask::Discrete);
            discrete.setMaximumNumberOfTouches(0);
            discrete.setCancelsTouchesInView(false);
            view.addGestureRecognizer(&discrete);

            // Hover covers a pointer and a hovering pencil alike. Pinch and
            // rotation are the trackpad's only: a pinch of direct touches
            // is recognized from the touch streams, so it must not also
            // arrive here.
            let hover = UIHoverGestureRecognizer::initWithTarget_action(
                UIHoverGestureRecognizer::alloc(mtm),
                Some(&*target),
                Some(sel!(hover:)),
            );
            view.addGestureRecognizer(&hover);

            let indirect_pointer = NSArray::from_retained_slice(&[NSNumber::new_isize(
                UITouchType::IndirectPointer.0,
            )]);
            let delegate = ProtocolObject::from_ref(&*target);
            let pinch = UIPinchGestureRecognizer::initWithTarget_action(
                UIPinchGestureRecognizer::alloc(mtm),
                Some(&*target),
                Some(sel!(pinch:)),
            );
            pinch.setAllowedTouchTypes(&indirect_pointer);
            pinch.setCancelsTouchesInView(false);
            pinch.setDelegate(Some(delegate));
            view.addGestureRecognizer(&pinch);

            let rotation = UIRotationGestureRecognizer::initWithTarget_action(
                UIRotationGestureRecognizer::alloc(mtm),
                Some(&*target),
                Some(sel!(rotate:)),
            );
            rotation.setAllowedTouchTypes(&indirect_pointer);
            rotation.setCancelsTouchesInView(false);
            rotation.setDelegate(Some(delegate));
            view.addGestureRecognizer(&rotation);

            view.ivars().pointer_recognizers.replace(vec![
                continuous.into_super(),
                discrete.into_super(),
                hover.into_super(),
                pinch.into_super(),
                rotation.into_super(),
            ]);
        }
        view.ivars().pointer_target.replace(Some(target));
        view.ivars()
            .cursor
            .replace(Some(CursorInteraction::install(&view)));
        view
    }

    /// Shows `cursor` as the pointer over the view.
    ///
    /// # Panics
    ///
    /// Panics when the view was not built by [`InputView::new`], which
    /// installs the cursor interaction.
    pub fn set_pointer_cursor(&self, cursor: PointerCursor) {
        self.ivars()
            .cursor
            .borrow()
            .as_ref()
            .expect("InputView installs its cursor interaction on creation")
            .set(cursor);
    }

    /// Installs `handler` as the receiver of every surface event.
    pub fn set_event_handler(&self, handler: impl Fn(SurfaceEvent) + 'static) {
        self.ivars().event_handler.replace(Some(Rc::new(handler)));
    }

    /// Installs `client` as the document `UITextInput` edits, replacing any
    /// previous one.
    pub fn set_text_input_client(&self, client: Rc<dyn TextInputClient>) {
        self.ivars().client.replace(Some(client));
    }

    /// Removes the installed document; the view then reports an empty,
    /// non-editing one.
    pub fn clear_text_input_client(&self) {
        self.ivars().client.replace(None);
    }

    /// The handle the content announces its own document changes through.
    #[must_use]
    pub fn text_input_notifier(&self) -> TextInputNotifier {
        TextInputNotifier::new(Weak::new(self))
    }

    /// Device pixels per point where the view is drawn.
    ///
    /// `None` while the view is outside any window scene, where the scale is
    /// not yet known.
    #[must_use]
    pub fn display_scale(&self) -> Option<f64> {
        // SAFETY: a plain trait-collection read, on the main thread.
        let scale = unsafe {
            let view: &UIView = self;
            objc2_ui_kit::UITraitEnvironment::traitCollection(view).displayScale()
        };
        (scale > 0.0).then_some(scale)
    }

    /// The highest rate, in frames per second, the screen the view is drawn
    /// on can display; `None` while the view is outside any window scene.
    #[must_use]
    pub fn max_frames_per_second(&self) -> Option<u32> {
        let scene = self.window()?.windowScene()?;
        u32::try_from(scene.screen().maximumFramesPerSecond()).ok()
    }

    /// The identity of the screen the view is drawn on; `None` while the view
    /// is outside any window scene. Two values are equal exactly when the
    /// screen is the same one, which is all it is good for.
    #[must_use]
    pub fn screen_id(&self) -> Option<usize> {
        let scene = self.window()?.windowScene()?;
        Some(Retained::as_ptr(&scene.screen()) as usize)
    }

    /// Installs `handler`, called after every layout pass of the view.
    pub fn set_layout_handler(&self, handler: impl Fn() + 'static) {
        self.ivars().layout_handler.replace(Some(Rc::new(handler)));
    }

    /// Installs `handler`, called with the new insets whenever the view's
    /// safe area changes — a notch, the home indicator, window chrome, a
    /// rotation or a resize.
    pub fn set_safe_area_handler(&self, handler: impl Fn(EdgeInsets) + 'static) {
        self.ivars()
            .safe_area_handler
            .replace(Some(Rc::new(handler)));
    }

    /// The distances from each edge of the view within which content is
    /// obscured by bars, the sensor housing or window chrome.
    #[must_use]
    pub fn safe_area_insets(&self) -> EdgeInsets {
        let objc2_ui_kit::UIEdgeInsets {
            top,
            left,
            bottom,
            right,
        } = self.safeAreaInsets();
        EdgeInsets::new(top, left, bottom, right)
    }

    /// The input method's change delegate, while one is set and alive.
    pub(crate) fn delegate(&self) -> Option<Retained<ProtocolObject<dyn UITextInputDelegate>>> {
        self.ivars()
            .input_delegate
            .borrow()
            .as_ref()
            .and_then(Weak::load)
    }

    /// Commits the client's marked text, if any.
    pub(crate) fn unmark_document(&self) {
        if let Some(client) = self.client()
            && client.marked_range().is_some()
        {
            client.unmark_text();
        }
    }

    /// The installed document.
    fn client(&self) -> Option<Rc<dyn TextInputClient>> {
        self.ivars().client.borrow().clone()
    }

    /// The installed document, while an editor is focused.
    fn editing_client(&self) -> Option<Rc<dyn TextInputClient>> {
        self.client().filter(|client| client.is_editing())
    }

    /// The keyboard traits the focused editor asks for; the defaults while
    /// there is none.
    fn traits(&self) -> super::text_input::TextInputTraits {
        self.client()
            .map(|client| guarded("InputView traits", || client.traits()))
            .unwrap_or_default()
    }

    fn text_in(&self, range: &UITextRange) -> Option<Retained<NSString>> {
        let client = self.client()?;
        let range = clamped(downcast_range(range)?, client.len());
        Some(NSString::from_str(&client.text(range)))
    }

    fn marked_text_range_value(&self) -> Option<Retained<UITextRange>> {
        self.client()?
            .marked_range()
            .map(|marked| self.range(marked))
    }

    /// The grapheme `position` extends over in `direction`.
    fn extended_range(
        &self,
        position: &UITextPosition,
        direction: UITextLayoutDirection,
    ) -> Option<Retained<UITextRange>> {
        let offset = downcast_position(position)?;
        let client = self.client()?;
        let len = client.len();
        if offset > len {
            return None;
        }
        let extended = if towards_start(direction) {
            if offset == 0 {
                return None;
            }
            previous_grapheme(&*client, offset)
        } else {
            if offset == len {
                return None;
            }
            next_grapheme(&*client, offset)
        };
        Some(self.range(extended))
    }

    fn closest_position_within(
        &self,
        point: objc2_core_foundation::CGPoint,
        range: &UITextRange,
    ) -> Option<Retained<UITextPosition>> {
        let (start, end) = downcast_range(range)?;
        let offset = downcast_position(&self.closest_position_for_point(point))?;
        Some(self.position(offset.clamp(start.min(end), start.max(end))))
    }

    fn character_range_near(
        &self,
        point: objc2_core_foundation::CGPoint,
    ) -> Option<Retained<UITextRange>> {
        let client = self.client()?;
        let offset = guarded("InputView characterRangeAtPoint:", || {
            client.closest_offset(kurbo::Point::new(point.x, point.y))
        })
        .min(client.len());
        if offset == client.len() {
            return Some(self.range(offset..offset));
        }
        Some(self.range(next_grapheme(&*client, offset)))
    }

    fn document_len(&self) -> usize {
        self.client().map_or(0, |client| client.len())
    }

    fn position(&self, offset: usize) -> Retained<UITextPosition> {
        TextPosition::new(self.mtm(), offset).into_super()
    }

    fn range(&self, range: Range<usize>) -> Retained<UITextRange> {
        TextRange::new(self.mtm(), range.start, range.end).into_super()
    }

    fn offset_position(
        &self,
        position: &UITextPosition,
        offset: isize,
    ) -> Option<Retained<UITextPosition>> {
        let moved = downcast_position(position)?.checked_add_signed(offset)?;
        (moved <= self.document_len()).then(|| self.position(moved))
    }

    fn closest_position_for_point(
        &self,
        point: objc2_core_foundation::CGPoint,
    ) -> Retained<UITextPosition> {
        let offset = self.client().map_or(0, |client| {
            guarded("InputView closestPositionToPoint:", || {
                client.closest_offset(kurbo::Point::new(point.x, point.y))
            })
            .min(client.len())
        });
        self.position(offset)
    }

    fn emit(&self, event: SurfaceEvent) {
        let handler = self.ivars().event_handler.borrow().clone();
        if let Some(handler) = handler {
            handler(event);
        }
    }

    /// Sends `gesture` through the indirect-scroll path the recognizer
    /// `source` selects — `Continuous` dispatches the registered
    /// `scrollContinuous:` selector on the owned scroll target,
    /// `Discrete` `scrollDiscrete:` — so a native test drives the actual
    /// action implementation with a seeded sender. It does not synthesize
    /// hardware scroll input or inspect `UIKit`'s private target
    /// registry; a headless process cannot make `UIKit` report scroll
    /// input to a recognizer.
    ///
    /// # Panics
    ///
    /// Panics when `source` is neither `Continuous` nor `Discrete`.
    #[cfg(feature = "native-test")]
    pub fn scroll_probe_for_test(
        &self,
        gesture: &UIPanGestureRecognizer,
        source: UIScrollTypeMask,
    ) {
        let target = self.ivars().pointer_target.borrow();
        let target = target
            .as_ref()
            .expect("InputView always owns a pointer target");
        match source {
            // SAFETY: the ivars own `target`; the action takes one
            // `UIPanGestureRecognizer`.
            UIScrollTypeMask::Continuous => unsafe {
                let _: () = msg_send![&**target, scrollContinuous: gesture];
            },
            // SAFETY: the ivars own `target`; the action takes one
            // `UIPanGestureRecognizer`.
            UIScrollTypeMask::Discrete => unsafe {
                let _: () = msg_send![&**target, scrollDiscrete: gesture];
            },
            _ => panic!("unsupported scroll source mask {source:?}"),
        }
    }

    /// A wheel notch as a scroll event: the translation since the last
    /// callback is the delta — `setTranslation` zeroed it — negated into the
    /// W3C sign `SurfaceEvent` uses (a pan's translation counts the
    /// pointer's own motion, positive right and down, while
    /// `delta_x`/`delta_y` count content movement, positive left and up).
    /// `translationInView:` is always a view-coordinate displacement in
    /// logical points — `Pixel`. A notch is complete on its own, so every
    /// callback is `finished`. A trackpad glide is a phased
    /// [`SurfaceEvent::TrackpadPan`] instead, so the receiver owns its
    /// momentum.
    fn handle_scroll(&self, gesture: &UIPanGestureRecognizer) {
        let this: &UIView = self;
        let point = gesture.locationInView(Some(this));
        let translation = gesture.translationInView(Some(this));
        gesture.setTranslation_inView(objc2_core_foundation::CGPoint::ZERO, Some(this));
        self.emit(SurfaceEvent::Scroll {
            position: kurbo::Point::new(point.x, point.y),
            delta_x: -translation.x,
            delta_y: -translation.y,
            unit: ScrollUnit::Pixel,
            finished: true,
        });
    }

    /// A trackpad glide's report as a phased pan: the translation since the
    /// last callback, negated into the content-motion sign `SurfaceEvent`
    /// uses — see [`handle_scroll`](Self::handle_scroll).
    fn handle_trackpad_pan(&self, gesture: &UIPanGestureRecognizer, phase: GesturePhase) {
        let this: &UIView = self;
        let point = gesture.locationInView(Some(this));
        let translation = gesture.translationInView(Some(this));
        gesture.setTranslation_inView(objc2_core_foundation::CGPoint::ZERO, Some(this));
        self.emit(SurfaceEvent::TrackpadPan {
            position: kurbo::Point::new(point.x, point.y),
            delta_x: -translation.x,
            delta_y: -translation.y,
            phase,
        });
    }

    /// A hover recognizer's report: the position while the pointer or pencil
    /// hovers, `None` when it leaves.
    ///
    /// `UIKit` reports no device for a hover; only a pencil has a height
    /// above the glass, so a non-zero `zOffset` is the pencil.
    fn handle_hover(&self, gesture: &UIHoverGestureRecognizer) {
        let kind = if gesture.zOffset() > 0.0 {
            TouchKind::Pencil
        } else {
            TouchKind::IndirectPointer
        };
        let position = match gesture.state() {
            UIGestureRecognizerState::Began | UIGestureRecognizerState::Changed => {
                let this: &UIView = self;
                let point = gesture.locationInView(Some(this));
                Some(kurbo::Point::new(point.x, point.y))
            }
            UIGestureRecognizerState::Ended
            | UIGestureRecognizerState::Cancelled
            | UIGestureRecognizerState::Failed => None,
            _ => return,
        };
        self.emit(SurfaceEvent::Hover { position, kind });
    }

    /// A trackpad pinch as the scale change since the last callback:
    /// `scale` is reset to 1 after each read, so it is `1 + delta`.
    fn handle_magnification(&self, gesture: &UIPinchGestureRecognizer, phase: GesturePhase) {
        let this: &UIView = self;
        let point = gesture.locationInView(Some(this));
        let delta = gesture.scale() - 1.0;
        gesture.setScale(1.0);
        self.emit(SurfaceEvent::Magnification {
            position: kurbo::Point::new(point.x, point.y),
            delta,
            phase,
        });
    }

    /// A trackpad rotation as the angle change since the last callback, in
    /// radians: `rotation` is reset to 0 after each read.
    fn handle_rotation(&self, gesture: &UIRotationGestureRecognizer, phase: GesturePhase) {
        let this: &UIView = self;
        let point = gesture.locationInView(Some(this));
        let delta = gesture.rotation();
        gesture.setRotation(0.0);
        self.emit(SurfaceEvent::Rotation {
            position: kurbo::Point::new(point.x, point.y),
            delta,
            phase,
        });
    }

    /// Every touch in `touches` as a [`SurfaceEvent::Touch`] in `phase`.
    ///
    /// The phase comes from the `UIResponder` callback, so a cancellation is
    /// reported as such and never as a release. `event` supplies the held
    /// buttons of an indirect pointer.
    fn send_touches(&self, touches: &NSSet<UITouch>, event: Option<&UIEvent>, phase: TouchPhase) {
        let this: &UIView = self;
        for touch in touches {
            let kind = touch_kind(touch.r#type());
            let point = touch.locationInView(Some(this));
            let buttons = match (kind, event) {
                (TouchKind::IndirectPointer, Some(event)) => pointer_buttons(event.buttonMask()),
                _ => PointerButtons::empty(),
            };
            self.emit(SurfaceEvent::Touch {
                id: Retained::as_ptr(&touch).addr() as u64,
                phase,
                position: kurbo::Point::new(point.x, point.y),
                kind,
                buttons,
            });
        }
    }

    /// A press batch as `Modifiers` + `Key` events, one pair per key the
    /// content receives, or one [`SurfaceEvent::KeyboardCancel`] for a
    /// cancelled batch.
    ///
    /// While an editor is focused the text system's keys are left for
    /// `UIKit`; those presses, and any without a key, are returned to be
    /// forwarded to `super`.
    fn send_presses(
        &self,
        press_set: &NSSet<UIPress>,
        phase: PressPhase,
    ) -> Vec<Retained<UIPress>> {
        let editing = self.editing_client().is_some();
        let mut forwarded = Vec::new();
        let mut delivered = false;
        for press in press_set {
            let Some(key) = press.key(self.mtm()) else {
                forwarded.push(press);
                continue;
            };
            let logical = keys::surface_key(&key);
            if editing && is_text_system_key(&logical) {
                forwarded.push(press);
                continue;
            }
            delivered = true;
            if phase == PressPhase::Cancelled {
                continue;
            }
            let modifiers = keys::surface_modifiers(key.modifierFlags());
            self.emit(SurfaceEvent::Modifiers(modifiers));
            self.emit(SurfaceEvent::Key {
                pressed: phase == PressPhase::Began,
                key: logical,
                code: keys::surface_code(&key),
                modifiers,
                repeat: false,
            });
        }
        if delivered && phase == PressPhase::Cancelled {
            self.emit(SurfaceEvent::KeyboardCancel);
        }
        forwarded
    }
}
