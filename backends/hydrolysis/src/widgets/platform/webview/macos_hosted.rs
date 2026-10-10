//! `AppKit` input port. Cherenkov, not this port, places the hosted view.

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
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    rc::Rc,
};

pub(super) struct PortState {
    web: Retained<WKWebView>,
    window: RefCell<Option<Weak<NSWindow>>>,
    focused: Binding<bool>,
    focus_requested: Cell<bool>,
    occlusion: Rc<RefCell<Vec<kurbo::Rect>>>,
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
        fn flipped(&self) -> bool { true }

        #[unsafe(method_id(hitTest:))]
        fn hit(&self, point: NSPoint) -> Option<Retained<NSView>> {
            self.hit_test(point)
        }

        #[unsafe(method(viewWillMoveToWindow:))]
        fn moving(&self, window: Option<&NSWindow>) {
            self.stop_observing();
            // SAFETY: forward AppKit's lifecycle notification to NSView.
            unsafe { let _: () = msg_send![super(self), viewWillMoveToWindow: window]; }
        }

        #[unsafe(method(viewDidMoveToWindow))]
        fn moved(&self) {
            // SAFETY: NSView lifecycle on the main thread.
            unsafe { let _: () = msg_send![super(self), viewDidMoveToWindow]; }
            if let Some(window) = self.window() {
                self.ivars().window.replace(Some(Weak::new(&window)));
                // SAFETY: this live port implements the observer selector and removes
                // itself before leaving the window. The window is held weakly.
                unsafe {
                    window.addObserver_forKeyPath_options_context(
                        self, &NSString::from_str("firstResponder"),
                        NSKeyValueObservingOptions::Initial, std::ptr::null_mut(),
                    );
                }
                if self.ivars().focus_requested.replace(false) {
                    self.request_focus();
                }
            }
        }

        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn focus_changed(&self, _key: Option<&NSString>, _object: Option<&AnyObject>,
            _change: Option<&NSDictionary>, _context: *mut c_void) {
            let focused = self.window().and_then(|window| window.firstResponder())
                .and_then(|responder| responder.downcast::<NSView>().ok())
                .is_some_and(|view| view.isDescendantOf(&self.ivars().web));
            if self.ivars().focused.snapshot() != focused { self.ivars().focused.set(focused); }
        }
    }
);

impl WebPort {
    fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
        let window = self.window()?;
        let root = window.contentView()?;
        // SAFETY: AppKit calls hitTest on the main thread while the view is attached.
        let parent = unsafe { self.superview() };
        let local = self.convertPoint_fromView(point, parent.as_deref());
        let mut position = self.convertPoint_toView(local, Some(&root));
        if !root.isFlipped() {
            position.y = root.bounds().size.height - position.y;
        }
        if self
            .ivars()
            .occlusion
            .borrow()
            .iter()
            .any(|rect| rect.contains((position.x, position.y)))
        {
            return None;
        }
        // SAFETY: the superclass receives AppKit's original superview-space point.
        unsafe { msg_send![super(self), hitTest: point] }
    }

    pub(super) fn new(
        web: &WKWebView,
        focused: Binding<bool>,
        occlusion: Rc<RefCell<Vec<kurbo::Rect>>>,
    ) -> Retained<Self> {
        let mtm = MainThreadMarker::new().expect("WKWebView hosting requires the main thread");
        let this = Self::alloc(mtm).set_ivars(PortState {
            web: web.retain(),
            window: RefCell::new(None),
            focused,
            occlusion,
            focus_requested: Cell::new(false),
        });
        // SAFETY: initialize the allocated NSView subclass on its required thread.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        web.setFrame(this.bounds());
        web.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        this.addSubview(web);
        this
    }

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

    pub(super) fn stop_observing(&self) {
        if let Some(window) = self
            .ivars()
            .window
            .borrow_mut()
            .take()
            .and_then(|weak| weak.load())
        {
            // SAFETY: matches the single registration made in moved().
            unsafe {
                window.removeObserver_forKeyPath_context(
                    self,
                    &NSString::from_str("firstResponder"),
                    std::ptr::null_mut(),
                );
            }
        }
        self.ivars().focused.set(false);
    }
}
