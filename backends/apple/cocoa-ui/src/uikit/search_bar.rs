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
use objc2_foundation::NSObjectProtocol;
use objc2_ui_kit::{UIBarPositioningDelegate, UISearchBar, UISearchBarDelegate};

/// The search bar's state.
/// Called with the bar's current text after an edit.
type ChangeHandler = Rc<dyn Fn(String)>;

/// The search bar's state.
pub struct SearchBarIvars {
    /// Called with the bar's current text after an edit.
    change: RefCell<Option<ChangeHandler>>,
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
    /// A `UISearchBar` reporting its text edits.
    pub struct SearchBar;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UISearchBar`.
    unsafe impl NSObjectProtocol for SearchBar {}

    // SAFETY: `searchBar:textDidChange:` carries `UISearchBarDelegate`'s
    // signature.
    unsafe impl UIBarPositioningDelegate for SearchBar {}

    // SAFETY: `searchBar:textDidChange:` carries `UISearchBarDelegate`'s
    // signature.
    unsafe impl UISearchBarDelegate for SearchBar {
        // SAFETY: see the module safety note.
        #[unsafe(method(searchBar:textDidChange:))]
        fn search_bar_text_did_change(
            &self,
            _search_bar: &UISearchBar,
            search_text: &objc2_foundation::NSString,
        ) {
            let handler = self.ivars().change.borrow().clone();
            if let Some(handler) = handler {
                handler(search_text.to_string());
            }
        }
    }
);

impl SearchBar {
    /// An empty search bar.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SearchBarIvars {
            change: RefCell::new(None),
        });
        // SAFETY: `initWithFrame:` is `UISearchBar`'s designated initializer.
        let this: Retained<Self> =
            unsafe { msg_send![super(this), initWithFrame: objc2_core_foundation::CGRect::ZERO] };
        // SAFETY: the bar conforms to `UISearchBarDelegate`; the delegate is
        // an assign reference.
        this.setDelegate(Some(ProtocolObject::from_ref(&*this)));
        this
    }

    /// Sets the placeholder shown when the bar is empty.
    pub fn set_placeholder(&self, placeholder: &str) {
        self.setPlaceholder(Some(&objc2_foundation::NSString::from_str(placeholder)));
    }

    /// The bar's current text.
    #[must_use]
    pub fn text(&self) -> String {
        UISearchBar::text(self)
            .map(|text| text.to_string())
            .unwrap_or_default()
    }

    /// Sets the bar's text without firing `on_change`.
    pub fn set_text(&self, text: &str) {
        self.setText(Some(&objc2_foundation::NSString::from_str(text)));
    }

    /// Runs `handler` with the bar's text after each edit.
    pub fn set_change_handler(&self, handler: impl Fn(String) + 'static) {
        self.ivars().change.replace(Some(Rc::new(handler)));
    }
}
