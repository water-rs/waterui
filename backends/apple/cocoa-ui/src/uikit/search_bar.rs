//! The `UIKit` search bar: a `UISearchBar` reporting text edits.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UISearchBar` subclass that is its own
//! delegate and calls `objc2`/`UIKit` bindings marked unsafe because `UIKit`
//! control APIs are main-thread only, which the `MainThreadOnly` thread
//! kind and [`MainThreadMarker`] constructor guarantee.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSObjectProtocol, NSString};
use objc2_ui_kit::{UIBarPositioningDelegate, UISearchBar, UISearchBarDelegate};

use crate::callback::guarded;

/// Called with the bar's current text after an edit.
type ChangeHandler = Rc<dyn Fn(String)>;

/// Called on submit or editing focus changes.
type ActionHandler = Rc<dyn Fn()>;

/// The search bar's state.
pub struct SearchBarIvars {
    /// Called with the bar's current text after an edit.
    change: RefCell<Option<ChangeHandler>>,
    /// Called when the keyboard Search button is tapped.
    submit: RefCell<Option<ActionHandler>>,
    /// Called when editing begins.
    begin_editing: RefCell<Option<ActionHandler>>,
    /// Called when editing ends.
    end_editing: RefCell<Option<ActionHandler>>,
}

impl fmt::Debug for SearchBarIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchBarIvars").finish()
    }
}

define_class!(
    // SAFETY: `UISearchBar`'s designated initializer is `initWithFrame:`,
    // which `SearchBar::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(UISearchBar))]
    #[name = "CocoaUiSearchBar"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SearchBarIvars]
    #[derive(Debug)]
    /// A `UISearchBar` reporting text edits, submissions, and focus changes.
    pub struct SearchBar;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UISearchBar`.
    unsafe impl NSObjectProtocol for SearchBar {}

    // SAFETY: `searchBar:textDidChange:` carries `UISearchBarDelegate`'s
    // signature.
    unsafe impl UIBarPositioningDelegate for SearchBar {}

    // SAFETY: `UISearchBarDelegate` methods carry `UISearchBarDelegate`'s signatures.
    unsafe impl UISearchBarDelegate for SearchBar {
        // SAFETY: see the module safety note.
        #[unsafe(method(searchBar:textDidChange:))]
        fn search_bar_text_did_change(&self, _search_bar: &UISearchBar, search_text: &NSString) {
            guarded("SearchBar searchBar:textDidChange:", || {
                let handler = self.ivars().change.borrow().clone();
                if let Some(handler) = handler {
                    handler(search_text.to_string());
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(searchBarTextDidBeginEditing:))]
        fn search_bar_text_did_begin_editing(&self, _search_bar: &UISearchBar) {
            guarded("SearchBar searchBarTextDidBeginEditing:", || {
                let handler = self.ivars().begin_editing.borrow().clone();
                if let Some(handler) = handler {
                    handler();
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(searchBarTextDidEndEditing:))]
        fn search_bar_text_did_end_editing(&self, _search_bar: &UISearchBar) {
            guarded("SearchBar searchBarTextDidEndEditing:", || {
                let handler = self.ivars().end_editing.borrow().clone();
                if let Some(handler) = handler {
                    handler();
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(searchBarSearchButtonClicked:))]
        fn search_bar_search_button_clicked(&self, _search_bar: &UISearchBar) {
            guarded("SearchBar searchBarSearchButtonClicked:", || {
                let handler = self.ivars().submit.borrow().clone();
                if let Some(handler) = handler {
                    handler();
                }
            });
        }
    }
);

impl SearchBar {
    /// An empty search bar.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SearchBarIvars {
            change: RefCell::new(None),
            submit: RefCell::new(None),
            begin_editing: RefCell::new(None),
            end_editing: RefCell::new(None),
        });
        // SAFETY: `initWithFrame:` is `UISearchBar`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        // SAFETY: the bar conforms to `UISearchBarDelegate`; the delegate is
        // an assign reference.
        this.setDelegate(Some(ProtocolObject::from_ref(&*this)));
        this
    }

    /// Sets the placeholder shown when the bar is empty.
    pub fn set_placeholder(&self, placeholder: &str) {
        self.setPlaceholder(Some(&NSString::from_str(placeholder)));
    }

    /// The bar's current text.
    #[must_use]
    pub fn text(&self) -> String {
        UISearchBar::text(self).map_or_default(|text| text.to_string())
    }

    /// Sets the bar's text without firing `on_change`.
    pub fn set_text(&self, text: &str) {
        self.setText(Some(&NSString::from_str(text)));
    }

    /// Runs `handler` with the bar's text after each edit.
    pub fn set_change_handler(&self, handler: impl Fn(String) + 'static) {
        self.ivars().change.replace(Some(Rc::new(handler)));
    }

    /// Runs `handler` when the user submits the query via the keyboard Search button.
    pub fn set_submit_handler(&self, handler: impl Fn() + 'static) {
        self.ivars().submit.replace(Some(Rc::new(handler)));
    }

    /// Runs `handler` when the search bar begins editing.
    pub fn set_begin_editing_handler(&self, handler: impl Fn() + 'static) {
        self.ivars().begin_editing.replace(Some(Rc::new(handler)));
    }

    /// Runs `handler` when the search bar ends editing.
    pub fn set_end_editing_handler(&self, handler: impl Fn() + 'static) {
        self.ivars().end_editing.replace(Some(Rc::new(handler)));
    }

    /// Focuses or dismisses the search bar's editing session.
    pub fn set_active(&self, active: bool) {
        if active {
            if !self.isFirstResponder() {
                self.becomeFirstResponder();
            }
        } else if self.isFirstResponder() {
            self.resignFirstResponder();
        }
    }
}
