//! The `AppKit` port a `WKWebView` is hosted through.
//!
//! Cherenkov places the port inside its own views (`HostedView`); the port
//! never sets its own geometry. It owns what only the view hierarchy can
//! decide: which clicks the page receives — none where Hydrolysis painted
//! interactive content above it — and whether the page holds the window's
//! first responder.

use nami::{Binding, Signal};
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView, NSWindow};
use objc2_foundation::{
    NSDictionary, NSKeyValueObservingOptions, NSObjectNSKeyValueObserverRegistration, NSPoint,
    NSRect, NSString,
};
use objc2_web_kit::WKWebView;
use std::{cell::Cell, cell::RefCell, ffi::c_void};

use crate::HostedOcclusion;

/// The window key path whose changes tell the port where focus went.
const FIRST_RESPONDER: &str = "firstResponder";

pub(super) struct PortState {
    web: Retained<WKWebView>,
    /// The window the port observes, held weakly: the window owns the port
    /// through its view hierarchy, not the other way round.
    window: RefCell<Option<Weak<NSWindow>>>,
    focused: Binding<bool>,
    /// A focus request made before the port reached a window, applied once
    /// it does.
    focus_requested: Cell<bool>,
    occlusion: HostedOcclusion,
}

define_class!(
    #[unsafe(super(NSView))]
    #[name = "WuiHydrolysisHostedWebPort"]
    #[thread_kind = MainThreadOnly]
    #[ivars = PortState]
    pub(super) struct WebPort;

    unsafe impl NSObjectProtocol for WebPort {}

    impl WebPort {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool {
            true
        }

        #[unsafe(method_id(hitTest:))]
        fn hit(&self, point: NSPoint) -> Option<Retained<NSView>> {
            self.hit_test(point)
        }

        #[unsafe(method(viewWillMoveToWindow:))]
        fn moving(&self, window: Option<&NSWindow>) {
            self.stop_observing();
            // SAFETY: forwards AppKit's own lifecycle message, with its
            // argument, to `NSView` on the main thread.
            unsafe {
                let _: () = msg_send![super(self), viewWillMoveToWindow: window];
            }
        }

        #[unsafe(method(viewDidMoveToWindow))]
        fn moved(&self) {
            // SAFETY: forwards AppKit's own lifecycle message to `NSView` on
            // the main thread.
            unsafe {
                let _: () = msg_send![super(self), viewDidMoveToWindow];
            }
            let Some(window) = self.window() else {
                return;
            };
            self.ivars().window.replace(Some(Weak::new(&window)));
            // SAFETY: the port implements the observer selector, and the one
            // registration is removed under the same context in
            // `stop_observing`, which `viewWillMoveToWindow:` runs before the
            // port leaves this window — so the window never messages a port
            // that has left it.
            unsafe {
                window.addObserver_forKeyPath_options_context(
                    self,
                    &NSString::from_str(FIRST_RESPONDER),
                    NSKeyValueObservingOptions::Initial,
                    self.observation_context(),
                );
            }
            if self.ivars().focus_requested.replace(false) {
                self.request_focus();
            }
        }

        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(
            &self,
            key_path: Option<&NSString>,
            object: Option<&AnyObject>,
            change: Option<&NSDictionary>,
            context: *mut c_void,
        ) {
            if context != self.observation_context() {
                // SAFETY: an observation `NSView` registered itself; KVO's
                // contract is to hand it to the superclass unchanged.
                unsafe {
                    let _: () = msg_send![
                        super(self),
                        observeValueForKeyPath: key_path,
                        ofObject: object,
                        change: change,
                        context: context
                    ];
                }
                return;
            }
            let focused = self
                .window()
                .and_then(|window| window.firstResponder())
                .and_then(|responder| responder.downcast::<NSView>().ok())
                .is_some_and(|view| view.isDescendantOf(&self.ivars().web));
            self.set_focused(focused);
        }
    }
);

impl WebPort {
    pub(super) fn new(
        web: &WKWebView,
        focused: Binding<bool>,
        occlusion: HostedOcclusion,
    ) -> Retained<Self> {
        let mtm = MainThreadMarker::new().expect("WKWebView hosting requires the main thread");
        let this = Self::alloc(mtm).set_ivars(PortState {
            web: web.retain(),
            window: RefCell::new(None),
            focused,
            focus_requested: Cell::new(false),
            occlusion,
        });
        // SAFETY: `initWithFrame:` is `NSView`'s designated initializer, sent
        // once to the freshly allocated instance on the main thread.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        web.setFrame(this.bounds());
        web.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        this.addSubview(web);
        this
    }

    /// The KVO context naming the port's own observation: the address of
    /// its ivars, unique to this instance for its whole lifetime.
    fn observation_context(&self) -> *mut c_void {
        std::ptr::from_ref(self.ivars()).cast_mut().cast()
    }

    fn set_focused(&self, focused: bool) {
        if self.ivars().focused.snapshot() != focused {
            self.ivars().focused.set(focused);
        }
    }

    /// Refuses the hit wherever Hydrolysis painted interactive content above
    /// the page, so the event falls through to the window's content view.
    fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
        let window = self.window()?;
        let root = window.contentView()?;
        // SAFETY: AppKit sends `hitTest:` on the main thread to a view in a
        // window, whose superview it is reading.
        let parent = unsafe { self.superview() };
        let local = self.convertPoint_fromView(point, parent.as_deref());
        let mut position = self.convertPoint_toView(local, Some(&root));
        if !root.isFlipped() {
            position.y = root.bounds().size.height - position.y;
        }
        if self
            .ivars()
            .occlusion
            .covers(kurbo::Point::new(position.x, position.y))
        {
            return None;
        }
        // SAFETY: `NSView`'s own hit test, given AppKit's original point in
        // the superview's space, on the main thread.
        unsafe { msg_send![super(self), hitTest: point] }
    }

    /// Makes the page the window's first responder, or does so once the port
    /// reaches a window.
    pub(super) fn request_focus(&self) {
        if let Some(window) = self.window() {
            assert!(
                window.makeFirstResponder(Some(&self.ivars().web)),
                "WKWebView refused first responder"
            );
        } else {
            self.ivars().focus_requested.set(true);
        }
    }

    /// Removes the port's first-responder observation, if it holds one, and
    /// reports the page unfocused.
    pub(super) fn stop_observing(&self) {
        if let Some(window) = self
            .ivars()
            .window
            .borrow_mut()
            .take()
            .and_then(|weak| weak.load())
        {
            // SAFETY: removes exactly the registration `viewDidMoveToWindow`
            // made on this window, under the same key path and context.
            unsafe {
                window.removeObserver_forKeyPath_context(
                    self,
                    &NSString::from_str(FIRST_RESPONDER),
                    self.observation_context(),
                );
            }
        }
        self.set_focused(false);
    }
}
