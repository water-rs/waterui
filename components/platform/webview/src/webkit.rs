//! The `WebKit` (`WKWebView`) engine's own vocabulary, shared between the
//! backend realizations that drive it.
//!
//! `objc2-web-kit` types exist only where `WKWebView` does, so this module
//! compiles on macOS and iOS alone.

use std::cell::RefCell;

use objc2::rc::{Retained, Weak};
use objc2_web_kit::WKNavigation;

/// The `WKNavigation`s whose policy question is already answered.
///
/// `WKNavigationAction.mainFrameNavigation` names the navigation an action
/// belongs to, and it is exact where counting or URL comparison cannot be:
///
/// - the calls that issue a navigation — `loadRequest:`, `goBack`,
///   `goForward`, `reload` — return the same object the later action
///   reports, because a `WebKit` API object has one wrapper;
/// - a server redirect continues its navigation instead of starting one,
///   so the action `WebKit` asks about for the redirect carries the same
///   `WKNavigation` again;
/// - a subframe action reports `nil`, and a page action carries its own
///   navigation, so neither can be mistaken for a load the application
///   issued.
///
/// Recording the `WKNavigation` a load call returned — or the one an
/// action's `mainFrameNavigation` names when the policy allows it —
/// therefore answers every action the navigation produces: itself and each
/// of its redirects.
///
/// The references are weak. `WebKit` drops a navigation once it finishes
/// or fails, the entry dies with it, and dead entries are pruned on each
/// access — nothing is counted, and no URL is compared.
#[derive(Debug, Default)]
pub struct AnsweredNavigations {
    navigations: RefCell<Vec<Weak<WKNavigation>>>,
}

impl AnsweredNavigations {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records `navigation` as answered: a load the application issued, the
    /// in-place reroute of a targetless action, or a navigation the policy
    /// allowed.
    pub fn answer(&self, navigation: &WKNavigation) {
        let mut navigations = self.navigations.borrow_mut();
        navigations.retain(|weak| weak.load().is_some());
        navigations.push(Weak::new(navigation));
    }

    /// Whether `navigation` — an action's `mainFrameNavigation`, which is
    /// `None` for a subframe action — is one already answered.
    pub fn is_answered(&self, navigation: Option<&WKNavigation>) -> bool {
        let Some(navigation) = navigation else {
            return false;
        };
        let mut answered = false;
        self.navigations.borrow_mut().retain(|weak| {
            weak.load().is_some_and(|live| {
                answered |= std::ptr::eq(Retained::as_ptr(&live), navigation);
                true
            })
        });
        answered
    }
}
