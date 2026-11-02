//! `NSAlert`: the window-modal alert sheet (water-rs/waterui#1210).
//!
//! A [`SheetAlert`] wraps one `NSAlert`: buttons are added most-to-least
//! prominent — the first button added is the rightmost and bound to Return
//! — and [`SheetAlert::begin_sheet`] runs the alert as a sheet on its host
//! window until a button response ends it. [`SheetAlert::dismiss`] ends a
//! live sheet when the presentation closes without a button (the app wrote
//! the binding back to `false`).

use std::cell::RefCell;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSButton, NSModalResponse, NSWindow};
use objc2_foundation::NSString;

/// A window-modal `NSAlert` presented as a sheet.
pub struct SheetAlert {
    alert: Retained<NSAlert>,
}

impl std::fmt::Debug for SheetAlert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SheetAlert")
            .field("alert", &self.alert)
            .finish()
    }
}

impl SheetAlert {
    /// An alert titled `title`, with `message` as its informative text.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, title: &str, message: Option<&str>) -> Self {
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(title));
        if let Some(message) = message {
            alert.setInformativeText(&NSString::from_str(message));
        }
        Self { alert }
    }

    /// Replaces the alert's message text — a reactive title landing while
    /// the sheet is up.
    pub fn set_title(&self, title: &str) {
        self.alert.setMessageText(&NSString::from_str(title));
    }

    /// Replaces the alert's informative text, or clears it with `None` —
    /// `setInformativeText:` takes no nil, so an empty string is the clear.
    pub fn set_message(&self, message: Option<&str>) {
        self.alert
            .setInformativeText(&NSString::from_str(message.unwrap_or("")));
    }

    /// Appends a button in most-to-least-prominent order — its position in
    /// add order is what `NSAlert` subtracts from `NSAlertFirstButtonReturn`
    /// to report its response. The caller marks the role on the returned
    /// button (Escape `keyEquivalent` for Cancel, `hasDestructiveAction`
    /// for Destructive).
    #[must_use]
    pub fn add_button(&self, title: &str) -> Retained<NSButton> {
        self.alert.addButtonWithTitle(&NSString::from_str(title))
    }

    /// Presents the alert as a sheet on `window`; `response` runs once with
    /// the zero-based index of the button that ended it.
    pub fn begin_sheet(&self, window: &NSWindow, response: impl FnMut(usize) + 'static) {
        let response = RefCell::new(response);
        let handler = RcBlock::new(move |result: NSModalResponse| {
            (response.borrow_mut())((result - NSAlertFirstButtonReturn).cast_unsigned());
        });
        self.alert
            .beginSheetModalForWindow_completionHandler(window, Some(&handler));
    }

    /// Ends the sheet without a button response — an app-side dismissal.
    /// No-op when the alert is not currently a sheet.
    pub fn dismiss(&self) {
        let alert_window = self.alert.window();
        if let Some(parent) = alert_window.sheetParent() {
            parent.endSheet(&alert_window);
        }
    }
}
