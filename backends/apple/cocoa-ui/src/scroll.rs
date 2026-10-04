//! The nearest enclosing scroll surface and observation of its viewport.
//!
//! A lazily materializing container asks [`enclosing_scroll_view`] which
//! scroll view clips it, [`scroll_viewport`] for the visible rectangle in its
//! own coordinates, and [`observe_scroll_viewport`] to be told when that
//! rectangle moves.
//!
//! # Safety
//!
//! The `unsafe` here walks the view hierarchy, converts rectangles between
//! sibling coordinate spaces, and registers a key-value observer — all
//! main-thread calls on objects the caller keeps alive. The `UIKit` observer is
//! a purpose-built `NSObject` subclass whose `observeValueForKeyPath`
//! implementation forwards to a Rust closure; the `AppKit` path rides
//! [`crate::notification`], whose observer token the observation unregisters
//! on drop.

use std::rc::Rc;

use objc2::rc::Retained;

use crate::PlatformView;
use crate::geometry::Rect;

#[cfg(target_os = "macos")]
use objc2_app_kit::NSScrollView as PlatformScrollView;
#[cfg(target_os = "ios")]
use objc2_ui_kit::UIScrollView as PlatformScrollView;

/// The platform's scroll view class: `NSScrollView` on macOS, `UIScrollView`
/// on iOS.
pub type ScrollView = PlatformScrollView;

/// The nearest [`ScrollView`] ancestor of `view`, walking superviews.
#[must_use]
#[cfg(target_os = "ios")]
pub fn enclosing_scroll_view(view: &PlatformView) -> Option<Retained<ScrollView>> {
    enclosing_scroll_views(view).into_iter().next()
}

/// The nearest [`ScrollView`] ancestor of `view`, walking superviews.
#[must_use]
#[cfg(target_os = "macos")]
pub fn enclosing_scroll_view(view: &PlatformView) -> Option<Retained<ScrollView>> {
    view.enclosingScrollView()
}

/// Every [`ScrollView`] ancestor of `view` walking superviews.
///
/// Ordered nearest-first. A nested scroll chain can clip a leaf through
/// any of its ancestors — a foreign outer scroll view emits none of the
/// emissions the nearest one does — so a visibility watch must observe
/// the whole chain, not just the nearest.
#[must_use]
#[cfg(target_os = "ios")]
pub fn enclosing_scroll_views(view: &PlatformView) -> Vec<Retained<ScrollView>> {
    let mut ancestors = Vec::new();
    let mut current = view.superview();
    while let Some(candidate) = current {
        let next = candidate.superview();
        if let Ok(scroll) = Retained::downcast::<ScrollView>(candidate) {
            ancestors.push(scroll);
        }
        current = next;
    }
    ancestors
}

/// Every [`ScrollView`] ancestor of `view` walking superviews.
///
/// Ordered nearest-first. `NSView.enclosingScrollView` only answers the
/// nearest, which cannot wake a leaf clipped away by a foreign outer
/// scroll, so the chain is walked like the `iOS` sibling.
///
/// # Safety
///
/// Same main-thread hierarchy walk as the callers: the views are alive
/// because the caller walks from a mounted descendant.
#[must_use]
#[cfg(target_os = "macos")]
pub fn enclosing_scroll_views(view: &PlatformView) -> Vec<Retained<ScrollView>> {
    let mut ancestors = Vec::new();
    // SAFETY: reading superview links of live views on the main thread.
    let mut current = unsafe { view.superview() };
    while let Some(candidate) = current {
        // SAFETY: same main-thread read of a live ancestor's link.
        let next = unsafe { candidate.superview() };
        if let Ok(scroll) = Retained::downcast::<ScrollView>(candidate) {
            ancestors.push(scroll);
        }
        current = next;
    }
    ancestors
}

/// The part of `view`'s coordinate space visible through `scroll`: the scroll
/// view's bounds converted into `view`'s coordinates.
#[must_use]
#[cfg(target_os = "ios")]
pub fn scroll_viewport(view: &PlatformView, scroll: &ScrollView) -> Rect {
    view.convertRect_fromView(scroll.bounds(), Some(scroll))
        .into()
}

/// The part of `view`'s coordinate space visible through `scroll`: the clip
/// view's bounds converted into `view`'s coordinates.
#[must_use]
#[cfg(target_os = "macos")]
pub fn scroll_viewport(view: &PlatformView, scroll: &ScrollView) -> Rect {
    let clip = scroll.contentView();
    view.convertRect_fromView(clip.bounds(), Some(&clip)).into()
}

/// Calls `handler` whenever `scroll`'s viewport moves — `contentOffset` on
/// `UIKit`, the clip view's bounds on `AppKit`.
///
/// Drop the returned observation to stop the calls. On `AppKit` the
/// clip view's `postsBoundsChangedNotifications` stays on afterwards:
/// the flag is `AppKit`'s documented opt-in for these notifications and
/// restoring it under an unknown set of sibling observers could silence
/// a live one — the residual cost is notification posts nobody reads,
/// not a lost wake.
#[must_use]
pub fn observe_scroll_viewport(
    scroll: &Retained<ScrollView>,
    handler: impl Fn() + 'static,
) -> ScrollObservation {
    ScrollObservation {
        _inner: imp::register(scroll, Rc::new(handler)),
    }
}

/// A live scroll-viewport observation; dropping it unregisters the observer.
#[derive(Debug)]
pub struct ScrollObservation {
    _inner: imp::Token,
}

#[cfg(target_os = "ios")]
mod imp {
    use super::ScrollView;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
    use objc2_foundation::{
        NSDictionary, NSKeyValueChangeKey, NSKeyValueObservingOptions, NSObject, NSObjectProtocol,
        NSString,
    };
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::rc::Rc;

    use crate::callback::guarded;

    /// The key path `UIKit` scroll views publish their offset under.
    fn content_offset() -> Retained<NSString> {
        NSString::from_str("contentOffset")
    }

    /// The state [`ScrollObserver`] carries: the closure a change calls.
    #[derive(Default)]
    pub struct ObserverIvars {
        handler: RefCell<Option<Rc<dyn Fn()>>>,
    }

    impl std::fmt::Debug for ObserverIvars {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("ObserverIvars")
                .field("handler", &self.handler.borrow().is_some())
                .finish()
        }
    }

    define_class!(
        // SAFETY: `NSObject` has no designated initializer requirement beyond
        // `init`, which `new` uses, and the class does not implement `Drop`.
        #[unsafe(super(NSObject))]
        #[name = "CocoaUiScrollObserver"]
        #[thread_kind = MainThreadOnly]
        #[ivars = ObserverIvars]
        #[derive(Debug)]
        /// A key-value observer that forwards `contentOffset` changes to a
        /// Rust closure.
        pub struct ScrollObserver;

        // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
        unsafe impl NSObjectProtocol for ScrollObserver {}

        impl ScrollObserver {
            // SAFETY: overrides `NSObject`'s `observeValueForKeyPath`, which
            // `UIKit` calls on the main thread with the values it was
            // registered for; all parameters are read-only for the call.
            #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
            unsafe fn observe_value(
                &self,
                _key_path: Option<&NSString>,
                _object: Option<&AnyObject>,
                _change: Option<&NSDictionary<NSKeyValueChangeKey, AnyObject>>,
                _context: *mut c_void,
            ) {
                guarded("scroll observation", || {
                    let handler = self.ivars().handler.borrow().clone();
                    if let Some(handler) = handler {
                        handler();
                    }
                });
            }
        }
    );

    impl ScrollObserver {
        /// An observer calling `handler` on every observed change.
        fn new(mtm: objc2::MainThreadMarker, handler: Rc<dyn Fn()>) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(ObserverIvars {
                handler: RefCell::new(Some(handler)),
            });
            // SAFETY: `init` is `NSObject`'s designated initializer.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// A registered key-value observation on a scroll view. The scroll
    /// view is held weakly: the observation is owned by a descendant,
    /// so a strong retain would loop the view hierarchy — and a scroll
    /// view gone before its token has no registration left to remove.
    pub struct Token {
        scroll: objc2::rc::Weak<ScrollView>,
        observer: Retained<ScrollObserver>,
    }

    impl std::fmt::Debug for Token {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Token").finish_non_exhaustive()
        }
    }

    impl Drop for Token {
        fn drop(&mut self) {
            use objc2_foundation::NSObjectNSKeyValueObserverRegistration;
            let Some(scroll) = self.scroll.load() else {
                return;
            };
            // SAFETY: pairs the `addObserver` below; the scroll view is
            // still alive, so the registration can be removed cleanly.
            unsafe {
                scroll.removeObserver_forKeyPath_context(
                    &self.observer,
                    &content_offset(),
                    std::ptr::null_mut(),
                );
            }
        }
    }

    /// Registers a `contentOffset` observer on `scroll`.
    pub(super) fn register(scroll: &Retained<ScrollView>, handler: Rc<dyn Fn()>) -> Token {
        use objc2_foundation::NSObjectNSKeyValueObserverRegistration;
        let mtm = objc2::MainThreadMarker::from(&**scroll);
        let observer = ScrollObserver::new(mtm, handler);
        // SAFETY: `observer` is a live `NSObject` subclass answering
        // `observeValueForKeyPath`, and the token removes the
        // registration on drop while the observed view is still alive.
        unsafe {
            scroll.addObserver_forKeyPath_options_context(
                &observer,
                &content_offset(),
                NSKeyValueObservingOptions::New,
                std::ptr::null_mut(),
            );
        }
        Token {
            scroll: objc2::rc::Weak::new(&**scroll),
            observer,
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::ScrollView;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::NSViewBoundsDidChangeNotification;
    use std::rc::Rc;

    use crate::notification::{NotificationName, NotificationObserver};

    /// A registered clip-view bounds observer on a scroll view.
    ///
    /// The clip view is not retained: the observation is owned by a
    /// descendant, so a strong retain would loop the view hierarchy.
    /// `postsBoundsChangedNotifications` is left on after drop — the
    /// flag is the documented opt-in and restoring it could silence a
    /// sibling observer still watching the same clip view.
    pub struct Token {
        /// The notification-center observer.
        _observer: NotificationObserver,
    }

    impl std::fmt::Debug for Token {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Token").finish_non_exhaustive()
        }
    }

    /// Observes `scroll`'s clip view's bounds changes.
    pub(super) fn register(scroll: &Retained<ScrollView>, handler: Rc<dyn Fn()>) -> Token {
        let clip = scroll.contentView();
        clip.setPostsBoundsChangedNotifications(true);
        // SAFETY: `&clip` as `&AnyObject` is an upcast of a live view; the
        // notification name is an AppKit constant.
        let name = unsafe { NSViewBoundsDidChangeNotification };
        let observer = crate::notification::observe_object(
            objc2::MainThreadMarker::from(&**scroll),
            &NotificationName::framework(name),
            AsRef::<AnyObject>::as_ref(&*clip),
            move || handler(),
        );
        Token {
            _observer: observer,
        }
    }
}
