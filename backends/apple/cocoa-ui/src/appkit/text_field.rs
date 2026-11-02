//! The `AppKit` text field: an `NSTextField` driving single-line text input.
//!
//! The control draws the native rounded bezel — platform fill, hairline
//! border, and focus ring. The field is its own `NSTextFieldDelegate`: user edits
//! surface through `on_change` (`controlTextDidChange`) and Return through
//! `on_submit` (`doCommandBySelector:`), and the handlers live on the
//! class's ivars rather than on a target-action pair because editing
//! notifications only reach a delegate.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSTextField` subclass, conforms it to
//! `NSTextFieldDelegate` so editing notifications reach its handlers, and
//! calls `objc2` bindings marked unsafe because `AppKit` text APIs are
//! main-thread only — which the `MainThreadOnly` thread kind and
//! [`MainThreadMarker`] constructor guarantee. `delegate` is an assign
//! reference, so the field pointing at itself creates no cycle.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::Sel;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::sel;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSControlTextEditingDelegate, NSEvent, NSMenu, NSSecureTextField, NSTextAlignment,
    NSTextContent, NSTextDelegate, NSTextField, NSTextFieldBezelStyle, NSTextFieldDelegate,
    NSTextInputClient, NSTextView, NSTextViewDelegate,
};
use objc2_core_foundation::CGRect;
use objc2_foundation::{
    NSAttributedString, NSNotification, NSObjectProtocol, NSRange, NSString, NSUInteger,
    NSUndoManagerDidRedoChangeNotification, NSUndoManagerDidUndoChangeNotification,
    NSUndoManagerWillRedoChangeNotification, NSUndoManagerWillUndoChangeNotification,
};

use crate::callback::guarded;
use crate::geometry::Size;
use crate::notification::{self, NotificationName, NotificationObserver};

type Handler = Rc<dyn Fn(&TextField)>;
/// A [`SecureField`] handler, typed at the secure field.
type SecureHandler = Rc<dyn Fn(&SecureField)>;
/// Runs when the field's editor asks for its context menu — the field, the
/// editing `NSTextView` and the standard edit menu to append to.
type EditMenuHandler = Rc<dyn Fn(&TextField, &NSTextView, &NSMenu)>;

/// The editing handlers a [`TextField`] fires — slots so a handler may be
/// installed after construction and replaced while the field is live.
pub struct TextFieldIvars {
    change: RefCell<Option<Handler>>,
    submit: RefCell<Option<Handler>>,
    edit_menu: RefCell<Option<EditMenuHandler>>,
    /// The field editor's `NSUndoManager` notifications observed for the
    /// current editing session — installed at begin, cleared at end.
    undo_observers: RefCell<Vec<NotificationObserver>>,
    /// An undo or redo is running between its `will` and `did` posts —
    /// the `controlTextDidChange` it may emit must not double-report.
    history_transaction: Cell<bool>,
}

impl fmt::Debug for TextFieldIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextFieldIvars")
            .field("has_change", &self.change.borrow().is_some())
            .field("has_submit", &self.submit.borrow().is_some())
            .field("has_edit_menu", &self.edit_menu.borrow().is_some())
            .finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSTextField`'s designated initializer is `initWithFrame:`,
    // which `TextField::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSTextField))]
    #[name = "CocoaUiTextField"]
    #[thread_kind = MainThreadOnly]
    #[ivars = TextFieldIvars]
    #[derive(Debug)]
    /// An editable `NSTextField` that is its own editing delegate.
    pub struct TextField;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSTextField` subclass.
    unsafe impl NSObjectProtocol for TextField {}

    // SAFETY: the delegate methods carry the signatures
    // `NSControlTextEditingDelegate` declares; both fire only for user
    // edits — programmatic writes through `setAttributedStringValue` do not
    // post `controlTextDidChange`, so an external update cannot echo back.
    unsafe impl NSControlTextEditingDelegate for TextField {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, _notification: &NSNotification) {
            guarded("TextField controlTextDidChange:", || {
                // An ordered `AppKit` may report the undo's own edit mid-
                // transaction; the `did` post reports the final state once.
                if self.ivars().history_transaction.get() {
                    return;
                }
                // Cloned out of the cell so the handler may replace itself.
                let handler = self.ivars().change.borrow().clone();
                if let Some(handler) = handler {
                    handler(self);
                }
            });
        }

        #[unsafe(method(controlTextDidBeginEditing:))]
        fn control_text_did_begin_editing(&self, _notification: &NSNotification) {
            guarded("TextField controlTextDidBeginEditing:", || {
                let editor = self
                    .field_editor()
                    .expect("controlTextDidBeginEditing posts with a live field editor");
                // The editor's own undo manager — `None` when the session
                // runs with native undo disabled.
                let Some(undo_manager) = editor.undoManager() else {
                    return;
                };
                let mtm = MainThreadMarker::from(&*editor);
                let weak = Weak::new(self);
                let begin = |weak: &Weak<TextField>| {
                    let weak = weak.clone();
                    move || {
                        if let Some(this) = weak.load() {
                            this.ivars().history_transaction.set(true);
                        }
                    }
                };
                let finish = |weak: &Weak<TextField>| {
                    let weak = weak.clone();
                    move || {
                        if let Some(this) = weak.load() {
                            this.ivars().history_transaction.set(false);
                            let handler = this.ivars().change.borrow().clone();
                            if let Some(handler) = handler {
                                handler(&this);
                            }
                        }
                    }
                };
                // SAFETY: reads `NSUndoManager`'s published string
                // constants, exported for the process's lifetime.
                let (will_undo, did_undo, will_redo, did_redo) = unsafe {
                    (
                        NotificationName::framework(NSUndoManagerWillUndoChangeNotification),
                        NotificationName::framework(NSUndoManagerDidUndoChangeNotification),
                        NotificationName::framework(NSUndoManagerWillRedoChangeNotification),
                        NotificationName::framework(NSUndoManagerDidRedoChangeNotification),
                    )
                };
                let observers = &mut *self.ivars().undo_observers.borrow_mut();
                let object: &AnyObject = &undo_manager;
                observers.push(notification::observe_object(
                    mtm,
                    &will_undo,
                    object,
                    begin(&weak),
                ));
                observers.push(notification::observe_object(
                    mtm,
                    &did_undo,
                    object,
                    finish(&weak),
                ));
                observers.push(notification::observe_object(
                    mtm,
                    &will_redo,
                    object,
                    begin(&weak),
                ));
                observers.push(notification::observe_object(
                    mtm,
                    &did_redo,
                    object,
                    finish(&weak),
                ));
            });
        }

        #[unsafe(method(controlTextDidEndEditing:))]
        fn control_text_did_end_editing(&self, _notification: &NSNotification) {
            guarded("TextField controlTextDidEndEditing:", || {
                self.ivars().undo_observers.borrow_mut().clear();
                self.ivars().history_transaction.set(false);
            });
        }

        // SAFETY: `command` is the selector AppKit hands the delegate; it is
        // only compared, never performed, so any selector value is valid.
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn control_text_view_do_command_by_selector(
            &self,
            _control: &NSTextField,
            _text_view: &NSTextView,
            command: Sel,
        ) -> bool {
            guarded("TextField doCommandBySelector:", || {
                if command != sel!(insertNewline:) {
                    return false;
                }
                let handler = self.ivars().submit.borrow().clone();
                handler.is_some_and(|handler| {
                    // Consumed: Return submits rather than ending editing.
                    handler(self);
                    true
                })
                // Unconsumed: Return ends editing, exactly the stock
                // `NSTextField` behavior `false` restores.
            })
        }
    }

    // SAFETY: every `NSTextFieldDelegate` method sits on the super-protocol
    // this class already implements; nothing extra is required.
    unsafe impl NSTextFieldDelegate for TextField {}

    // SAFETY: `NSTextViewDelegate`'s super-protocol is all-optional.
    unsafe impl NSTextDelegate for TextField {}

    // SAFETY: `NSTextViewDelegate` is an all-optional protocol; the field
    // answers the one method it implements with the declared signature.
    // The field is its field editor's delegate — AppKit sets the control
    // editing through it — so this is the hook the editor's context menu
    // consults.
    unsafe impl NSTextViewDelegate for TextField {
        // SAFETY: the signature `NSTextViewDelegate` declares; returning
        // the passed menu, customized or not, is the documented answer.
        // `method_id` because the implementation hands back a `Retained`.
        #[unsafe(method_id(textView:menu:forEvent:atIndex:))]
        fn text_view_menu_for_event_at_index(
            &self,
            view: &NSTextView,
            menu: &NSMenu,
            _event: &NSEvent,
            _char_index: NSUInteger,
        ) -> Option<Retained<NSMenu>> {
            guarded("TextField textView:menu:forEvent:atIndex:", || {
                // Cloned out of the ivars so the handler may mutate the field.
                let handler = self.ivars().edit_menu.borrow().clone();
                if let Some(handler) = handler {
                    handler(self, view, menu);
                }
                Some(menu.retain())
            })
        }
    }
);

impl TextField {
    /// An editable, selectable, enabled field drawing the platform's
    /// rounded bezel, with `line_limit`
    /// applied (`Some(1)` is the single-line mode, `None` unlimited).
    #[must_use]
    pub fn new(mtm: MainThreadMarker, line_limit: Option<usize>) -> Retained<Self> {
        let ivars = TextFieldIvars {
            change: RefCell::new(None),
            submit: RefCell::new(None),
            edit_menu: RefCell::new(None),
            undo_observers: RefCell::new(Vec::new()),
            history_transaction: Cell::new(false),
        };
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: `initWithFrame:` is the inherited designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        this.setBezeled(true);
        this.setBezelStyle(NSTextFieldBezelStyle::RoundedBezel);
        this.setEditable(true);
        this.setSelectable(true);
        this.set_line_limit(line_limit);
        // SAFETY: `delegate` is assign; self-delegation retains nothing.
        unsafe { this.setDelegate(Some(ProtocolObject::from_ref(&*this))) };
        this
    }

    /// Applies `line_limit`: a single-line field scrolls inside its one
    /// line and never wraps; an unlimited field wraps without a cap.
    pub fn set_line_limit(&self, line_limit: Option<usize>) {
        let single = line_limit == Some(1);
        self.setUsesSingleLineMode(single);
        #[expect(
            clippy::cast_possible_wrap,
            reason = "NSTextField.maximumNumberOfLines is NSInteger; a line limit is tiny"
        )]
        self.setMaximumNumberOfLines(line_limit.map_or(0, |limit| limit as isize));
        if let Some(cell) = self.cell() {
            cell.setWraps(!single);
            cell.setScrollable(single);
        }
    }

    /// The text the field currently holds — during editing, the field
    /// editor's in-flight string.
    #[must_use]
    pub fn string(&self) -> String {
        if let Some(editor) = self.currentEditor() {
            return editor.string().to_string();
        }
        self.stringValue().to_string()
    }

    /// Writes `text` into the field. Programmatic writes do not post
    /// `controlTextDidChange`, so an external update cannot echo back as an
    /// edit.
    pub fn set_attributed_string(&self, text: &NSAttributedString) {
        self.setAttributedStringValue(text);
    }

    /// The placeholder shown while the field is empty.
    pub fn set_placeholder(&self, placeholder: &NSAttributedString) {
        self.setPlaceholderAttributedString(Some(placeholder));
    }

    /// The declared content type — the `NSTextContentType` autofill and
    /// one-time-code completion key on. `NSTextField` conforms to
    /// `NSTextContent`, which carries the property.
    pub fn set_content_type(&self, content_type: crate::ContentType) {
        self.setContentType(Some(content_type.native()));
    }

    /// How the field aligns its text.
    pub fn set_text_alignment(&self, alignment: NSTextAlignment) {
        self.setAlignment(alignment);
    }

    /// Whether the field responds to input.
    pub fn set_enabled(&self, enabled: bool) {
        self.setEnabled(enabled);
    }

    /// The control's intrinsic height — the text share of a measure, as
    /// `NSTextField.intrinsicContentSize`.
    #[must_use]
    pub fn measured_height(&self) -> f64 {
        self.intrinsicContentSize().height
    }

    /// Fires `handler` on every user edit. A later call replaces the
    /// previous handler.
    pub fn on_change(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().change.replace(Some(Rc::new(handler)));
    }

    /// Fires `handler` when the user submits with Return; the key is
    /// consumed. Without a handler Return ends editing — the stock
    /// behavior a plain field keeps.
    pub fn on_submit(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().submit.replace(Some(Rc::new(handler)));
    }

    /// The field's field editor — the shared `NSTextView` `AppKit` edits it
    /// through — while it is first responder, `None` otherwise.
    #[must_use]
    pub fn field_editor(&self) -> Option<Retained<NSTextView>> {
        self.currentEditor()
            .and_then(|editor| editor.downcast::<NSTextView>().ok())
    }

    /// Brings up a field editor: `selectText:` makes the field first
    /// responder and selects its contents. Answers whether an editor is
    /// live afterwards — false for a field that cannot take focus (no
    /// window, disabled).
    pub fn begin_editing(&self) -> bool {
        if self.field_editor().is_none() {
            // SAFETY: `selectText:` is `NSControl`'s editing request;
            // `nil` needs no object.
            unsafe { self.selectText(None) };
        }
        self.field_editor().is_some()
    }

    /// The selected UTF-16 range of the live edit — the field editor's
    /// `selectedRange`. `None` while the field is not being edited.
    #[must_use]
    pub fn selected_range(&self) -> Option<NSRange> {
        self.field_editor().map(|editor| editor.selectedRange())
    }

    /// Selects `range` (UTF-16 units) in the live edit.
    ///
    /// # Panics
    ///
    /// If the field is not being edited — call [`Self::begin_editing`]
    /// first when the selection must be written without a live session.
    pub fn set_selected_range(&self, range: NSRange) {
        let editor = self
            .field_editor()
            .expect("set_selected_range on a field that is not being edited");
        editor.setSelectedRange(range);
    }

    /// Replaces `range` (UTF-16 units) of the live edit with `text`
    /// through the single `insertText:replacementRange:` input entry —
    /// the native path applies validation, undo registration, the
    /// control's edit notification and line-limit normalization, and the
    /// caret lands after the inserted text.
    ///
    /// The single `insertText:replacementRange:` input entry runs the whole
    /// native edit: validation, the field's line policy, undo registration
    /// and the change notification. A manual `shouldChange` consultation on
    /// top registers a second change the undo replay corrupts, so there is
    /// exactly one call.
    ///
    /// # Panics
    ///
    /// If the field is not being edited — call [`Self::begin_editing`]
    /// first.
    pub fn replace_range(&self, range: NSRange, text: &NSString) {
        let editor = self
            .field_editor()
            .expect("replace_range on a field that is not being edited");
        // SAFETY: the input-system entry; `text` is a live `NSString`, the
        // `AnyObject` the client accepts.
        unsafe {
            editor.insertText_replacementRange(AsRef::<AnyObject>::as_ref(text), range);
        }
    }

    /// Runs `handler` each time the field's editor asks for its context
    /// menu — `textView:menu:forEvent:atIndex:` — with the standard edit
    /// menu, so the handler can append its own rows. The returned menu
    /// keeps the platform's items.
    pub fn install_edit_menu_handler(
        &self,
        handler: impl Fn(&Self, &NSTextView, &NSMenu) + 'static,
    ) {
        self.ivars().edit_menu.replace(Some(Rc::new(handler)));
    }
}

/// The field's intrinsic size as a kit [`Size`], for measures that report
/// the control's share of a layout pass.
#[must_use]
pub fn intrinsic_size(field: &TextField) -> Size {
    field.intrinsicContentSize().into()
}

/// The editing handlers a [`SecureField`] fires — the same slots as
/// [`TextFieldIvars`], typed at the secure field.
pub struct SecureFieldIvars {
    change: RefCell<Option<SecureHandler>>,
    submit: RefCell<Option<SecureHandler>>,
}

impl fmt::Debug for SecureFieldIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecureFieldIvars")
            .field("has_change", &self.change.borrow().is_some())
            .field("has_submit", &self.submit.borrow().is_some())
            .finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSSecureTextField` inherits `NSTextField`'s designated
    // initializer `initWithFrame:`, which `SecureField::new` calls, and the
    // class does not implement `Drop`.
    #[unsafe(super(NSSecureTextField))]
    #[name = "CocoaUiSecureField"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SecureFieldIvars]
    #[derive(Debug)]
    /// A masked `NSSecureTextField` that is its own editing delegate — the
    /// secure twin of [`TextField`], firing the same change and submit
    /// handlers.
    pub struct SecureField;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSSecureTextField`
    // subclass.
    unsafe impl NSObjectProtocol for SecureField {}

    // SAFETY: same contract as `TextField` — the methods carry the
    // signatures `NSControlTextEditingDelegate` declares and fire only for
    // user edits.
    unsafe impl NSControlTextEditingDelegate for SecureField {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, _notification: &NSNotification) {
            guarded("SecureField controlTextDidChange:", || {
                let handler = self.ivars().change.borrow().clone();
                if let Some(handler) = handler {
                    handler(self);
                }
            });
        }

        // SAFETY: `command` is the selector AppKit hands the delegate; it is
        // only compared, never performed, so any selector value is valid.
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn control_text_view_do_command_by_selector(
            &self,
            _control: &NSTextField,
            _text_view: &NSTextView,
            command: Sel,
        ) -> bool {
            guarded("SecureField doCommandBySelector:", || {
                if command != sel!(insertNewline:) {
                    return false;
                }
                let handler = self.ivars().submit.borrow().clone();
                handler.is_some_and(|handler| {
                    handler(self);
                    true
                })
            })
        }
    }

    // SAFETY: every `NSTextFieldDelegate` method sits on the super-protocol
    // this class already implements; nothing extra is required.
    unsafe impl NSTextFieldDelegate for SecureField {}
);

impl SecureField {
    /// An editable, selectable, enabled secure field drawing the rounded
    /// bezel — the masked twin of [`TextField::new`].
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let ivars = SecureFieldIvars {
            change: RefCell::new(None),
            submit: RefCell::new(None),
        };
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: `initWithFrame:` is the inherited designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        this.setBezeled(true);
        this.setBezelStyle(NSTextFieldBezelStyle::RoundedBezel);
        this.setEditable(true);
        this.setSelectable(true);
        // SAFETY: `delegate` is assign; self-delegation retains nothing.
        unsafe { this.setDelegate(Some(ProtocolObject::from_ref(&*this))) };
        this
    }

    /// The text the field currently holds — during editing, the field
    /// editor's in-flight string.
    #[must_use]
    pub fn string(&self) -> String {
        if let Some(editor) = self.currentEditor() {
            return editor.string().to_string();
        }
        self.stringValue().to_string()
    }

    /// Writes `text` into the field. Programmatic writes do not post
    /// `controlTextDidChange`, so an external update cannot echo back as an
    /// edit.
    pub fn set_string(&self, text: &str) {
        self.setStringValue(&NSString::from_str(text));
    }

    /// The placeholder shown while the field is empty.
    pub fn set_placeholder(&self, placeholder: &NSAttributedString) {
        self.setPlaceholderAttributedString(Some(placeholder));
    }

    /// How the field aligns its text.
    pub fn set_text_alignment(&self, alignment: NSTextAlignment) {
        self.setAlignment(alignment);
    }

    /// The font the field draws with.
    pub fn set_font(&self, font: &objc2_app_kit::NSFont) {
        self.setFont(Some(font));
    }

    /// Whether the field responds to input.
    pub fn set_enabled(&self, enabled: bool) {
        self.setEnabled(enabled);
    }

    /// The control's intrinsic height — the text share of a measure.
    #[must_use]
    pub fn measured_height(&self) -> f64 {
        self.intrinsicContentSize().height
    }

    /// Fires `handler` on every user edit. A later call replaces the
    /// previous handler.
    pub fn on_change(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().change.replace(Some(Rc::new(handler)));
    }

    /// Fires `handler` when the user submits with Return; the key is
    /// consumed. Without a handler Return ends editing — the stock
    /// behavior a plain field keeps.
    pub fn on_submit(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().submit.replace(Some(Rc::new(handler)));
    }
}
