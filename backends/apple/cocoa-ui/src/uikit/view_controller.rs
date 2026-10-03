//! The view controller at the root of a window.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UIViewController` subclass, initializes it,
//! and forwards to `UIViewController`'s own implementation of the method it
//! extends. Every override has the signature `UIViewController` declares, and
//! `UIKit` calls them on the main thread.

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSBundle, NSObjectProtocol, NSString};
use objc2_ui_kit::{UIView, UIViewController};

use super::host_view::{HostView, window_root};
use crate::callback::guarded;

/// The state a [`ViewController`] keeps.
#[derive(Debug)]
pub struct ViewControllerIvars {
    view: Retained<HostView>,
}

define_class!(
    // SAFETY: `UIViewController` asks a subclass to initialize through its
    // designated initializer, which `ViewController::new` does, and the class
    // does not implement `Drop`.
    #[unsafe(super(UIViewController))]
    #[name = "CocoaUiViewController"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ViewControllerIvars]
    #[derive(Debug)]
    /// A view controller hosting one [`HostView`] that always fills the
    /// window.
    ///
    /// The host view reports the window's safe-area insets, so content laid
    /// out in it can keep clear of bars and screen corners while backgrounds
    /// extend under them, and it still delivers touches to subviews placed
    /// outside its bounds.
    pub struct ViewController;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIViewController`
    // subclass.
    unsafe impl NSObjectProtocol for ViewController {}

    impl ViewController {
        // SAFETY: see the module safety note.
        #[unsafe(method(loadView))]
        fn load_view_override(&self) {
            guarded("ViewController loadView", || {
                self.setView(Some(&self.ivars().view));
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(viewWillLayoutSubviews))]
        fn view_will_layout_subviews_override(&self) {
            guarded("ViewController viewWillLayoutSubviews", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), viewWillLayoutSubviews] };
                let view = &self.ivars().view;
                if let Some(window) = view.window() {
                    view.setFrame(window.bounds());
                }
            });
        }
    }
);

impl ViewController {
    /// A view controller with an empty host view.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ViewControllerIvars {
            view: window_root(mtm),
        });
        // SAFETY: `initWithNibName:bundle:` is `UIViewController`'s designated
        // initializer; no nib means the view comes from `loadView`.
        unsafe {
            msg_send![
                super(this),
                initWithNibName: None::<&NSString>,
                bundle: None::<&NSBundle>
            ]
        }
    }

    /// The view filling the window, into which content is placed.
    #[must_use]
    pub fn host_view(&self) -> &HostView {
        &self.ivars().view
    }
}

/// The controller `view` is the root view of, if any: a `UIViewController`
/// sits immediately after its own view on the responder chain.
#[must_use]
pub fn owning_controller(view: &UIView) -> Option<Retained<UIViewController>> {
    view.nextResponder()
        .and_then(|responder| responder.downcast::<UIViewController>().ok())
}

/// The first `UIViewController` on `view`'s responder chain — the controller
/// whose hierarchy contains `view`.
#[must_use]
pub fn enclosing_controller(view: &UIView) -> Option<Retained<UIViewController>> {
    let mut responder = view.nextResponder();
    while let Some(candidate) = responder {
        responder = match candidate.downcast::<UIViewController>() {
            Ok(controller) => return Some(controller),
            Err(responder) => responder.nextResponder(),
        };
    }
    None
}

/// `-[UIViewController addChildViewController:]`.
pub fn add_child(parent: &UIViewController, child: &UIViewController) {
    parent.addChildViewController(child);
}

/// `-[UIViewController didMoveToParentViewController:]` with the parent
/// `add_child` installed.
pub fn did_move_to_parent(child: &UIViewController) {
    let parent = child.parentViewController();
    child.didMoveToParentViewController(parent.as_deref());
}

/// `-[UIViewController willMoveToParentViewController:]` with a nil parent.
pub fn will_move_to_parent(child: &UIViewController) {
    child.willMoveToParentViewController(None);
}

/// `-[UIViewController removeFromParentViewController]`.
pub fn remove_from_parent(child: &UIViewController) {
    child.removeFromParentViewController();
}
