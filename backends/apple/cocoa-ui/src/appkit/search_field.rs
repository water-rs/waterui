//! The `AppKit` search field: an `NSSearchField` reporting its edits.
//!
//! The field is its own `NSSearchFieldDelegate, NSTextFieldDelegate` (`controlTextDidChange`),
//! like [`crate::appkit::TextField`]; owners install a change handler and a
//! styled or plain placeholder rather than polling the value.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSSearchField` subclass, conforms it to
//! `NSSearchFieldDelegate` so editing notifications reach its handler, and
//! calls `objc2` bindings marked unsafe because `AppKit` control APIs are
//! main-thread only — which the `MainThreadOnly` thread kind and
//! [`MainThreadMarker`] constructor guarantee. `delegate` is an assign
//! reference, so the field pointing at itself creates no cycle.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSControlTextEditingDelegate, NSSearchField, NSSearchFieldDelegate, NSTextField,
    NSTextFieldDelegate, NSTextView,
};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSAttributedString, NSNotification, NSObjectProtocol, NSString};

use crate::callback::guarded;

type Handler = Rc<dyn Fn(&SearchField)>;

/// The editing handlers a [`SearchField`] fires — slots so handlers may
/// be installed after construction and replaced while the field is live.
pub struct SearchFieldIvars {
    change: RefCell<Option<Handler>>,
    submit: RefCell<Option<Handler>>,
    begin_editing: RefCell<Option<Handler>>,
    end_editing: RefCell<Option<Handler>>,
}

impl fmt::Debug for SearchFieldIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchFieldIvars")
            .field("has_change", &self.change.borrow().is_some())
            .field("has_submit", &self.submit.borrow().is_some())
            .field("has_begin_editing", &self.begin_editing.borrow().is_some())
            .field("has_end_editing", &self.end_editing.borrow().is_some())
            .finish()
    }
}

define_class!(
    // SAFETY: `NSSearchField`'s designated initializer is `initWithFrame:`,
    // which `SearchField::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSSearchField))]
    #[name = "CocoaUiSearchField"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SearchFieldIvars]
    #[derive(Debug)]
    /// An `NSSearchField` that is its own editing delegate.
    pub struct SearchField;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSSearchField` subclass.
    unsafe impl NSObjectProtocol for SearchField {}

    // SAFETY: the delegate methods carry the signatures
    // `NSControlTextEditingDelegate` declares; they fire only for user
    // edits/commands — programmatic writes through `setStringValue` do not
    // post `controlTextDidChange`, so an external update cannot echo back.
    unsafe impl NSControlTextEditingDelegate for SearchField {
        #[unsafe(method(controlTextDidBeginEditing:))]
        fn control_text_did_begin_editing(&self, _notification: &NSNotification) {
            guarded("SearchField controlTextDidBeginEditing:", || {
                let handler = self.ivars().begin_editing.borrow().clone();
                if let Some(handler) = handler {
                    handler(self);
                }
            });
        }

        #[unsafe(method(controlTextDidEndEditing:))]
        fn control_text_did_end_editing(&self, _notification: &NSNotification) {
            guarded("SearchField controlTextDidEndEditing:", || {
                let handler = self.ivars().end_editing.borrow().clone();
                if let Some(handler) = handler {
                    handler(self);
                }
            });
        }

        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, _notification: &NSNotification) {
            guarded("SearchField controlTextDidChange:", || {
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
            guarded("SearchField doCommandBySelector:", || {
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

    // SAFETY: `NSSearchFieldDelegate` adds nothing beyond the editing
    // delegate's methods.
    unsafe impl NSTextFieldDelegate for SearchField {}

    // SAFETY: `NSSearchFieldDelegate` adds nothing over
    // `NSTextFieldDelegate`.
    unsafe impl NSSearchFieldDelegate for SearchField {}
);

impl SearchField {
    /// An empty search field.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SearchFieldIvars {
            change: RefCell::new(None),
            submit: RefCell::new(None),
            begin_editing: RefCell::new(None),
            end_editing: RefCell::new(None),
        });
        // SAFETY: `initWithFrame:` is `NSSearchField`'s designated
        // initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        // SAFETY: the field conforms to `NSSearchFieldDelegate`; the
        // delegate is an assign reference.
        unsafe { this.setDelegate(Some(ProtocolObject::from_ref(&*this))) };
        this
    }

    /// The plain-text placeholder shown while the field is empty.
    pub fn set_placeholder(&self, placeholder: &str) {
        self.setPlaceholderString(Some(&NSString::from_str(placeholder)));
    }

    /// The styled placeholder shown while the field is empty.
    pub fn set_attributed_placeholder(&self, placeholder: &NSAttributedString) {
        self.setPlaceholderAttributedString(Some(placeholder));
    }

    /// The current text.
    #[must_use]
    pub fn text(&self) -> String {
        self.stringValue().to_string()
    }

    /// Replaces the field's text; does not fire the change handler.
    pub fn set_text(&self, text: &str) {
        self.setStringValue(&NSString::from_str(text));
    }

    /// Runs `handler` on every user edit.
    pub fn set_change_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().change.replace(Some(Rc::new(handler)));
    }

    /// Runs `handler` when the user submits with Return or Enter.
    pub fn set_submit_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().submit.replace(Some(Rc::new(handler)));
    }

    /// Runs `handler` when the search field begins an editing session.
    pub fn set_begin_editing_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().begin_editing.replace(Some(Rc::new(handler)));
    }

    /// Runs `handler` when the search field ends an editing session.
    pub fn set_end_editing_handler(&self, handler: impl Fn(&Self) + 'static) {
        self.ivars().end_editing.replace(Some(Rc::new(handler)));
    }

    /// Focuses or dismisses the search field's active editing session.
    pub fn set_active(&self, active: bool) {
        let Some(window) = self.window() else {
            return;
        };
        let is_editing = self.currentEditor().is_some();
        if active {
            if !is_editing {
                window.makeFirstResponder(Some(self));
            }
        } else if is_editing {
            // SAFETY: `self` is an `NSView` attached to `window`; ending editing for it is valid on the main thread.
            unsafe {
                window.endEditingFor(Some(self));
            }
            window.makeFirstResponder(None);
        }
    }
}
