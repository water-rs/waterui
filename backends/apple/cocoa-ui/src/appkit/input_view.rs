//! The first responder for a view that takes its own input.
//!
//! An [`InputView`] sits on top of the view whose pixels host interactive
//! content — a GPU surface — claims the pointer and the keyboard, and speaks
//! `NSTextInputClient` so an input method composes against the content's
//! caret. The view draws nothing; every event is translated into the
//! platform-neutral [`SurfaceEvent`] vocabulary and handed to the installed
//! handler.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSView` subclass implementing
//! `NSTextInputClient` and forwards to `NSView`'s/`NSResponder`'s own
//! implementations where the protocol allows. `AppKit` calls every override on
//! the main thread; `NSTextInputContext` drives the protocol methods on the
//! same thread.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::sel;
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
};
use objc2_app_kit::{
    NSEvent, NSEventPhase, NSMarkedClauseSegmentAttributeName, NSResponder, NSTextInputClient,
    NSTextInputContext, NSTrackingArea, NSTrackingAreaOptions, NSUnderlineColorAttributeName,
    NSUnderlineStyleAttributeName, NSView,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSNotFound, NSPoint, NSRange,
    NSRangePointer, NSRect, NSSize, NSString, NSUInteger,
};

use crate::callback::guarded;
use crate::geometry::Rect;
use crate::input::{PointerButton, ScrollUnit, SurfaceEvent};
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
    tracking_area: RefCell<Option<Retained<NSTrackingArea>>>,
    input_context: RefCell<Option<Retained<NSTextInputContext>>>,
    /// The pre-edit text the input method is currently composing, if any.
    marked_text: RefCell<String>,
    /// The selection the input method last reported, in UTF-16 units.
    marked_selection: RefCell<NSRange>,
    /// `AppKit` reports composition through `NSTextInputClient` callbacks that
    /// run inside `keyDown:`; the key event itself is only sent when the
    /// input method did not consume it.
    handling_key_down: std::cell::Cell<bool>,
    key_down_was_consumed: std::cell::Cell<bool>,
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

define_class!(
    // SAFETY: `NSView` asks a subclass to initialize through its designated
    // initializer, which `InputView::new` does, and the class does not
    // implement `Drop`.
    #[unsafe(super(NSView))]
    #[name = "CocoaUiInputView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = InputViewIvars]
    #[derive(Debug)]
    /// A view that claims keyboard, IME and pointer input for the content
    /// beneath it.
    ///
    /// Its coordinates are flipped — the origin is the top-left corner — so
    /// surface-local points need no mirroring.
    pub struct InputView;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSView` subclass.
    unsafe impl NSObjectProtocol for InputView {}

    // SAFETY: `NSTextInputClient`'s required methods are implemented with
    // their declared signatures; the protocol's document model is the
    // pre-edit buffer alone, which the ivars own.
    unsafe impl NSTextInputClient for InputView {
        // SAFETY: `string` is whatever text representation AppKit delivers —
        // a string or an attributed string.
        #[unsafe(method(insertText:replacementRange:))]
        unsafe fn insert_text_replacement_range(&self, string: &AnyObject, replacement_range: NSRange) {
            let _ = replacement_range;
            guarded("InputView insertText:", || {
                let text = plain_text(string);
                let was_composing = self.has_marked_text();
                self.clear_marked_text();
                if text.is_empty() {
                    if was_composing {
                        self.emit(SurfaceEvent::CompositionCommit(String::new()));
                    }
                    return;
                }
                if self.ivars().handling_key_down.get() {
                    self.ivars().key_down_was_consumed.set(true);
                }
                // Text that ends a composition is that session's commit;
                // text typed outside one is a plain insertion.
                self.emit(if was_composing {
                    SurfaceEvent::CompositionCommit(text)
                } else {
                    SurfaceEvent::TextInput(text)
                });
            });
        }

        // SAFETY: `string` is whatever text representation AppKit delivers.
        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        unsafe fn set_marked_text_selected_range_replacement_range(
            &self,
            string: &AnyObject,
            selected_range: NSRange,
            replacement_range: NSRange,
        ) {
            let _ = replacement_range;
            guarded("InputView setMarkedText:", || {
                let text = plain_text(string);
                let was_composing = self.has_marked_text();
                if self.ivars().handling_key_down.get() {
                    self.ivars().key_down_was_consumed.set(true);
                }
                if text.is_empty() {
                    // AppKit clears the pre-edit with empty marked text; that
                    // abandons the session rather than committing it.
                    self.clear_marked_text();
                    if was_composing {
                        self.emit(SurfaceEvent::CompositionCancel);
                    }
                    return;
                }
                if !was_composing {
                    self.emit(SurfaceEvent::CompositionStart);
                }
                self.ivars().marked_text.replace(text.clone());
                self.ivars().marked_selection.replace(selected_range);
                self.emit(SurfaceEvent::CompositionUpdate {
                    caret: composition_caret(&text, selected_range),
                    text,
                });
            });
        }

        // SAFETY: plain state read.
        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            guarded("InputView unmark_text", || {
                if !self.has_marked_text() {
                    return;
                }
                let text = self.ivars().marked_text.borrow().clone();
                self.clear_marked_text();
                // AppKit's `unmark_text` confirms the pre-edit as typed.
                self.emit(SurfaceEvent::CompositionCommit(text));
            });
        }

        // SAFETY: plain state read.
        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            *self.ivars().marked_selection.borrow()
        }

        // SAFETY: plain state read; the marked range is measured in UTF-16
        // code units as the protocol requires.
        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            let text = self.ivars().marked_text.borrow();
            if text.is_empty() {
                NSRange::new(NSNotFound.cast_unsigned(), 0)
            } else {
                NSRange::new(0, text.chars().map(char::len_utf16).sum::<usize>())
            }
        }

        // SAFETY: plain state read.
        #[unsafe(method(hasMarkedText))]
        fn has_marked_text_impl(&self) -> bool {
            self.has_marked_text()
        }

        // SAFETY: `actual_range` is AppKit's out-pointer, valid or null for
        // this call; the surface owns its document and the vocabulary is
        // one-way, so the host has no text to hand back.
        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        unsafe fn attributed_substring_for_proposed_range_actual_range(
            &self,
            range: NSRange,
            actual_range: NSRangePointer,
        ) -> Option<Retained<NSAttributedString>> {
            let _ = (range, actual_range);
            None
        }

        // SAFETY: returns an array of attributed-string key constants.
        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes_for_marked_text(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            // SAFETY: the attribute-name statics are system constants.
            let keys = unsafe {
                [
                    NSUnderlineStyleAttributeName,
                    NSUnderlineColorAttributeName,
                    NSMarkedClauseSegmentAttributeName,
                ]
            };
            NSArray::from_slice(&keys)
        }

        // SAFETY: `actual_range` is AppKit's out-pointer, valid or null for
        // this call; the caret rect provider answers in surface-local points
        // with y growing down, which this flips into AppKit's coordinates.
        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        unsafe fn first_rect_for_character_range_actual_range(
            &self,
            range: NSRange,
            actual_range: NSRangePointer,
        ) -> NSRect {
            guarded("InputView firstRectForCharacterRange:", || {
                if !actual_range.is_null() {
                    // SAFETY: see above.
                    unsafe { *actual_range = range };
                }
                let Some(window) = self.window() else {
                    return NSRect::ZERO;
                };
                let caret = self.caret().unwrap_or_else(|| {
                    Rect::new(0.0, 0.0, self.bounds().size.width, self.bounds().size.height)
                });
                let flipped = NSRect::new(
                    NSPoint::new(
                        caret.origin.x,
                        self.bounds().size.height - caret.origin.y - caret.size.height,
                    ),
                    NSSize::new(caret.size.width, caret.size.height),
                );
                window.convertRectToScreen(self.convertRect_toView(flipped, None))
            })
        }

        // SAFETY: plain query; the surface owns its document, so no index
        // exists.
        #[unsafe(method(characterIndexForPoint:))]
        fn character_index_for_point(&self, point: NSPoint) -> NSUInteger {
            let _ = point;
            NSNotFound.cast_unsigned()
        }
    }

    impl InputView {
        // SAFETY: see the module safety note.
        #[unsafe(method(isFlipped))]
        fn is_flipped_override(&self) -> bool {
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder_override(&self) -> bool {
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method_id(inputContext))]
        fn input_context_override(&self) -> Option<Retained<NSTextInputContext>> {
            self.ivars().input_context.borrow().clone()
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder_override(&self) -> bool {
            self.emit(SurfaceEvent::Focus(true));
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(resignFirstResponder))]
        fn resign_first_responder_override(&self) -> bool {
            self.emit(SurfaceEvent::Focus(false));
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse_override(&self, event: Option<&NSEvent>) -> bool {
            let _ = event;
            true
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas_override(&self) {
            guarded("InputView updateTrackingAreas", || {
                if let Some(area) = self.ivars().tracking_area.borrow_mut().take() {
                    self.removeTrackingArea(&area);
                }
                // SAFETY: `initWithRect:options:owner:userInfo:` retains its
                // owner argument; the tracking area is dropped with the view.
                // SAFETY: see the module safety note.
                let area = unsafe {
                    NSTrackingArea::initWithRect_options_owner_userInfo(
                        NSTrackingArea::alloc(),
                        NSRect::ZERO,
                        NSTrackingAreaOptions::ActiveInKeyWindow
                            | NSTrackingAreaOptions::InVisibleRect
                            | NSTrackingAreaOptions::MouseEnteredAndExited
                            | NSTrackingAreaOptions::MouseMoved,
                        Some(self.as_ref()),
                        None,
                    )
                };
                self.addTrackingArea(&area);
                self.ivars().tracking_area.replace(Some(area));
                // SAFETY: see the module safety note.
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), updateTrackingAreas] };
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.send_pointer_move(event);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.send_pointer_move(event);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            self.send_pointer_move(event);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &NSEvent) {
            self.send_pointer_move(event);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.claim_first_responder();
            self.send_pointer_button(event, true, PointerButton::Primary);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.send_pointer_button(event, false, PointerButton::Primary);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.claim_first_responder();
            self.send_pointer_button(event, true, PointerButton::Secondary);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.send_pointer_button(event, false, PointerButton::Secondary);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            let Some(button) = extra_button(event.buttonNumber()) else {
                return;
            };
            self.claim_first_responder();
            self.send_pointer_button(event, true, button);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            let Some(button) = extra_button(event.buttonNumber()) else {
                return;
            };
            self.send_pointer_button(event, false, button);
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            guarded("InputView scrollWheel:", || {
                let position = self.local_point(event);
                self.send_modifiers(event);
                // A trackpad glide reports precise deltas already in points;
                // a wheel notch reports lines, and each notch is complete on
                // its own.
                let precise = event.hasPreciseScrollingDeltas();
                let finished = if precise {
                    event.phase() == NSEventPhase::Ended
                        || event.momentumPhase() == NSEventPhase::Ended
                } else {
                    true
                };
                self.emit(SurfaceEvent::Scroll {
                    position,
                    delta_x: event.scrollingDeltaX(),
                    delta_y: event.scrollingDeltaY(),
                    unit: if precise { ScrollUnit::Pixel } else { ScrollUnit::Line },
                    finished,
                });
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            guarded("InputView flagsChanged:", || {
                self.send_modifiers(event);
                // A modifier key is also a key: its own press and release
                // cross as key events so a view can see, say, a bare Command
                // tap.
                let code = keys::surface_code(event);
                if code == keyboard_types::Code::Unidentified {
                    return;
                }
                self.send_key(event, keys::modifier_is_pressed(event, event.keyCode()), code);
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            guarded("InputView keyDown:", || {
                // The input method sees the key first: a composing keystroke
                // belongs to the composition session, not to the view as a
                // key event.
                self.ivars().handling_key_down.set(true);
                self.ivars().key_down_was_consumed.set(false);
                let had_marked_text = self.has_marked_text();
                if let Some(context) = self.ivars().input_context.borrow().as_ref() {
                    context.handleEvent(event);
                }
                self.ivars().handling_key_down.set(false);
                if !self.ivars().key_down_was_consumed.get()
                    && !had_marked_text
                    && !self.has_marked_text()
                {
                    self.send_key(event, true, keys::surface_code(event));
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            self.send_key(event, false, keys::surface_code(event));
        }

        // SAFETY: `doCommandBySelector:` is an `NSResponder` method the
        // input context calls; only `cancelOperation:` is handled.
        #[unsafe(method(doCommandBySelector:))]
        fn do_command_by_selector(&self, selector: objc2::runtime::Sel) {
            guarded("InputView doCommandBySelector:", || {
                if selector != sel!(cancelOperation:) || !self.has_marked_text() {
                    return;
                }
                self.clear_marked_text();
                self.ivars().key_down_was_consumed.set(true);
                self.emit(SurfaceEvent::CompositionCancel);
            });
        }
    }
);

/// The plain text inside a text-input argument: a string, or the characters
/// of an attributed string.
fn plain_text(value: &AnyObject) -> String {
    if let Some(attributed) = value.downcast_ref::<NSAttributedString>() {
        return attributed.string().to_string();
    }
    value.downcast_ref::<NSString>().map_or_else(
        || panic!("AppKit supplied unsupported text input {value:?}"),
        std::string::ToString::to_string,
    )
}

/// The caret's byte offset into the pre-edit text.
///
/// `AppKit` counts UTF-16 code units and the neutral vocabulary counts bytes,
/// so the prefix is re-measured rather than scaled.
fn composition_caret(text: &str, selected_range: NSRange) -> Option<usize> {
    if selected_range.location == NSNotFound.cast_unsigned() {
        return None;
    }
    let mut utf16 = 0usize;
    let mut byte = 0usize;
    for ch in text.chars() {
        if utf16 >= selected_range.location {
            return Some(byte);
        }
        utf16 += ch.len_utf16();
        byte += ch.len_utf8();
    }
    (utf16 >= selected_range.location).then_some(byte)
}

/// `AppKit`'s button numbers past the primary/secondary pair.
///
/// The W3C vocabulary names five buttons; anything past forward has no
/// neutral meaning and is not delivered rather than being reported as a
/// button a view would misread.
const fn extra_button(number: isize) -> Option<PointerButton> {
    match number {
        2 => Some(PointerButton::Middle),
        3 => Some(PointerButton::Back),
        4 => Some(PointerButton::Forward),
        _ => None,
    }
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
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer.
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        let context = NSTextInputContext::initWithClient(
            NSTextInputContext::alloc(mtm),
            objc2::runtime::ProtocolObject::from_ref(&*view),
        );
        view.ivars().input_context.replace(Some(context));
        view
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

    fn emit(&self, event: SurfaceEvent) {
        let handler = self.ivars().event_handler.borrow().clone();
        if let Some(handler) = handler {
            handler(event);
        }
    }

    fn caret(&self) -> Option<Rect> {
        self.ivars()
            .caret_provider
            .borrow()
            .as_ref()
            .and_then(|provider| provider())
    }

    fn has_marked_text(&self) -> bool {
        !self.ivars().marked_text.borrow().is_empty()
    }

    fn clear_marked_text(&self) {
        self.ivars().marked_text.borrow_mut().clear();
        self.ivars()
            .marked_selection
            .replace(NSRange::new(NSNotFound.cast_unsigned(), 0));
    }

    /// The event position in logical, surface-local points with y growing
    /// down.
    fn local_point(&self, event: &NSEvent) -> kurbo::Point {
        let point = self.convertPoint_fromView(event.locationInWindow(), None);
        kurbo::Point::new(point.x, self.bounds().size.height - point.y)
    }

    fn send_pointer_move(&self, event: &NSEvent) {
        let position = self.local_point(event);
        self.send_modifiers(event);
        self.emit(SurfaceEvent::PointerMove { position });
    }

    fn send_pointer_button(&self, event: &NSEvent, pressed: bool, button: PointerButton) {
        let position = self.local_point(event);
        self.send_modifiers(event);
        self.emit(SurfaceEvent::PointerButton {
            pressed,
            button,
            position,
        });
    }

    /// Publishes the chord an event carries before the event itself.
    ///
    /// The neutral vocabulary reports modifiers when they change; `AppKit`
    /// reports them on every event, so this keeps the view's chord current
    /// without the view having to read it off each event.
    fn send_modifiers(&self, event: &NSEvent) {
        self.emit(SurfaceEvent::Modifiers(keys::surface_modifiers(
            event.modifierFlags(),
        )));
    }

    fn send_key(&self, event: &NSEvent, pressed: bool, code: keyboard_types::Code) {
        self.emit(SurfaceEvent::Key {
            pressed,
            key: keys::surface_key(event, code),
            code,
            modifiers: keys::surface_modifiers(event.modifierFlags()),
            repeat: pressed && event.isARepeat(),
        });
    }

    fn claim_first_responder(&self) {
        if let Some(window) = self.window() {
            let this: &NSResponder = self;
            window.makeFirstResponder(Some(this));
        }
    }
}
