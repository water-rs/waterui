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
    let mut current = view.superview();
    while let Some(candidate) = current {
        let next = candidate.superview();
        if let Ok(scroll) = Retained::downcast::<ScrollView>(candidate) {
            return Some(scroll);
        }
        current = next;
    }
    None
}

/// The nearest [`ScrollView`] ancestor of `view`, walking superviews.
#[must_use]
#[cfg(target_os = "macos")]
pub fn enclosing_scroll_view(view: &PlatformView) -> Option<Retained<ScrollView>> {
    view.enclosingScrollView()
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
/// Drop the returned observation to stop the calls; on `AppKit` dropping also
/// switches the clip view's `postsBoundsChangedNotifications` back off.
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

    /// A registered key-value observation on a scroll view.
    #[derive(Debug)]
    pub struct Token {
        scroll: Retained<ScrollView>,
        observer: Retained<ScrollObserver>,
    }

    impl Drop for Token {
        fn drop(&mut self) {
            use objc2_foundation::NSObjectNSKeyValueObserverRegistration;
            // SAFETY: pairs the `addObserver` below; the token retained both
            // parties for the registration's life.
            unsafe {
                self.scroll.removeObserver_forKeyPath_context(
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
        // `observeValueForKeyPath`, and the token removes the registration on
        // drop while it still retains both parties.
        unsafe {
            scroll.addObserver_forKeyPath_options_context(
                &observer,
                &content_offset(),
                NSKeyValueObservingOptions::New,
                std::ptr::null_mut(),
            );
        }
        Token {
            scroll: scroll.clone(),
            observer,
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::ScrollView;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSClipView, NSViewBoundsDidChangeNotification};
    use std::rc::Rc;

    use crate::notification::{NotificationName, NotificationObserver};

    /// A registered clip-view bounds observer on a scroll view.
    #[derive(Debug)]
    pub struct Token {
        /// The clip view whose notifications flag is cleared on drop.
        clip: Retained<NSClipView>,
        /// The notification-center observer.
        _observer: NotificationObserver,
    }

    impl Drop for Token {
        fn drop(&mut self) {
            self.clip.setPostsBoundsChangedNotifications(false);
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
            clip,
            _observer: observer,
        }
    }
}
