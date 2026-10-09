//! `UIAlertController`: the alert presentation (water-rs/waterui#1210).
//!
//! An [`AlertController`] wraps one `UIAlertController` in `.alert` style:
//! actions carry `UIAlertActionStyle` — the platform's own Default /
//! Cancel / Destructive arrangement — and [`AlertController::present`]
//! presents it from the topmost controller of the host's window so a second
//! presented alert chains instead of being dropped.

use std::cell::RefCell;

use block2::RcBlock;
use core::ptr::NonNull;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_foundation::NSString;
use objc2_ui_kit::{
    UIAlertAction, UIAlertActionStyle, UIAlertController, UIAlertControllerStyle, UIView,
};

use crate::uikit::window_of;

/// A `.alert`-style `UIAlertController`.
pub struct AlertController {
    controller: Retained<UIAlertController>,
    mtm: MainThreadMarker,
}

impl std::fmt::Debug for AlertController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlertController")
            .field("controller", &self.controller)
            .field("mtm", &self.mtm)
            .finish()
    }
}

impl AlertController {
    /// An alert titled `title`, with `message` as its body.
    pub fn new(mtm: MainThreadMarker, title: &str, message: Option<&str>) -> Self {
        let title = NSString::from_str(title);
        let message = message.map(NSString::from_str);
        let controller = UIAlertController::alertControllerWithTitle_message_preferredStyle(
            Some(&title),
            message.as_deref(),
            UIAlertControllerStyle::Alert,
            mtm,
        );
        Self { controller, mtm }
    }

    /// Replaces the alert's title — a reactive title landing while the
    /// alert is up.
    pub fn set_title(&self, title: &str) {
        self.controller.setTitle(Some(&NSString::from_str(title)));
    }

    /// Replaces the alert's message, or clears it with `None`.
    pub fn set_message(&self, message: Option<&str>) {
        self.controller
            .setMessage(message.map(NSString::from_str).as_deref());
    }

    /// Appends an action of `style` whose tap runs `run`, and returns it so
    /// the caller can pin it as the preferred action.
    pub fn add_action(
        &self,
        title: &str,
        style: UIAlertActionStyle,
        run: impl FnMut() + 'static,
    ) -> Retained<UIAlertAction> {
        let run = RefCell::new(run);
        let handler = RcBlock::new(move |_action: NonNull<UIAlertAction>| {
            (run.borrow_mut())();
        });
        let action = UIAlertAction::actionWithTitle_style_handler(
            Some(&NSString::from_str(title)),
            style,
            Some(&handler),
            self.mtm,
        );
        self.controller.addAction(&action);
        action
    }

    /// Pins `action` — an action already added — as the preferred action:
    /// `UIKit` emboldens it, the visual primary.
    pub fn set_preferred(&self, action: &UIAlertAction) {
        self.controller.setPreferredAction(Some(action));
    }

    /// Presents the alert from the topmost controller of `host`'s window —
    /// an alert presented over a live alert chains above it instead of
    /// being dropped. Returns `false` when `host` is not in a window with a
    /// root view controller.
    #[must_use]
    pub fn present(&self, host: &UIView) -> bool {
        let _ = self.mtm;
        let Some(window) = window_of(host) else {
            return false;
        };
        let Some(root) = window.rootViewController() else {
            return false;
        };
        let mut presenter = root;
        while let Some(next) = presenter.presentedViewController() {
            presenter = next;
        }
        presenter.presentViewController_animated_completion(&self.controller, true, None);
        true
    }

    /// Dismisses the controller — the app-side close path; a user's button
    /// tap dismisses it itself.
    pub fn dismiss(&self) {
        self.controller
            .dismissViewControllerAnimated_completion(true, None);
    }
}
