//! `UIKit` modal sheet presentation (`UISheetPresentationController`,
//! water-rs/waterui#659).
//!
//! A [`SheetController`] hosts a [`ViewController`] presented modally over the
//! topmost controller of the anchor's `UIWindow`, configured either with
//! `UISheetPresentationController` detents (`PageSheet`) or `FullScreen` modal
//! presentation. `UIKit` handles modal inertness (pointer, keyboard, `VoiceOver`),
//! detent dragging, interactive swipe-down dismissal, and focus restoration.
//!
//! # Safety
//!
//! The `unsafe` here defines a `UIAdaptivePresentationControllerDelegate`
//! subclass bridging `presentationControllerDidDismiss:` on the main thread.

use std::cell::RefCell;
use std::fmt;

use block2::RcBlock;
use core::ptr::NonNull;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol};
use objc2_ui_kit::{
    UIAdaptivePresentationControllerDelegate, UIModalPresentationStyle, UIPresentationController,
    UISheetPresentationControllerDelegate, UISheetPresentationControllerDetent,
    UISheetPresentationControllerDetentResolutionContext, UIView,
};

use super::host_view::HostView;
use super::view_controller::ViewController;
use super::window_of;
use crate::callback::guarded;

/// A detent configuration for [`SheetController`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SheetDetentSpec {
    /// System medium detent (`mediumDetent`).
    Medium,
    /// System large detent (`largeDetent`).
    Large,
    /// Custom fractional detent of `maximumDetentValue` in `0.0..=1.0`.
    Fraction(f64),
    /// Custom fixed-height detent in points.
    Height(f64),
    /// Full-screen modal presentation (`UIModalPresentationStyle::FullScreen`).
    FullScreen,
}

type DismissCallback = Box<dyn FnMut()>;

struct SheetDelegateIvars {
    on_user_dismiss: RefCell<Option<DismissCallback>>,
}

impl fmt::Debug for SheetDelegateIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SheetDelegateIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSObject` has no subclass invariants and this class does not
    // implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiSheetDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SheetDelegateIvars]
    #[derive(Debug)]
    struct CocoaUiSheetDelegate;

    // SAFETY: `NSObjectProtocol` asks nothing extra of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for CocoaUiSheetDelegate {}

    // SAFETY: `UIAdaptivePresentationControllerDelegate` delivers
    // `presentationControllerDidDismiss:` on the main thread when the user
    // interactively dismisses the sheet.
    unsafe impl UIAdaptivePresentationControllerDelegate for CocoaUiSheetDelegate {
        #[unsafe(method(presentationControllerDidDismiss:))]
        fn presentation_controller_did_dismiss(&self, _controller: &UIPresentationController) {
            guarded("SheetController presentationControllerDidDismiss:", || {
                let handler = self.ivars().on_user_dismiss.borrow_mut().take();
                if let Some(mut handler) = handler {
                    handler();
                    let mut slot = self.ivars().on_user_dismiss.borrow_mut();
                    if slot.is_none() {
                        *slot = Some(handler);
                    }
                }
            });
        }
    }

    // SAFETY: `UISheetPresentationControllerDelegate` extends
    // `UIAdaptivePresentationControllerDelegate` on the main thread.
    unsafe impl UISheetPresentationControllerDelegate for CocoaUiSheetDelegate {}
);

impl CocoaUiSheetDelegate {
    fn new(mtm: MainThreadMarker, on_user_dismiss: impl FnMut() + 'static) -> Retained<Self> {
        let allocated = mtm.alloc::<Self>().set_ivars(SheetDelegateIvars {
            on_user_dismiss: RefCell::new(Some(Box::new(on_user_dismiss))),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(allocated), init] }
    }
}

/// A `UIKit` modal sheet controller hosting a [`HostView`].
pub struct SheetController {
    controller: Retained<ViewController>,
    delegate: Retained<CocoaUiSheetDelegate>,
}

impl fmt::Debug for SheetController {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SheetController")
            .field("controller", &self.controller)
            .finish()
    }
}

impl SheetController {
    /// Creates a modal sheet controller hosting `host` with `detents` and
    /// `dismissible` interactive-dismiss behavior.
    #[must_use]
    pub fn new(
        mtm: MainThreadMarker,
        host: Retained<HostView>,
        detents: &[SheetDetentSpec],
        dismissible: bool,
        on_user_dismiss: impl FnMut() + 'static,
    ) -> Self {
        let controller = ViewController::new(mtm, host);
        let delegate = CocoaUiSheetDelegate::new(mtm, on_user_dismiss);

        let full_screen = detents == [SheetDetentSpec::FullScreen];
        if full_screen {
            controller.setModalPresentationStyle(UIModalPresentationStyle::FullScreen);
        } else {
            controller.setModalPresentationStyle(UIModalPresentationStyle::PageSheet);
            controller.setModalInPresentation(!dismissible);
            if let Some(sheet) = controller.sheetPresentationController() {
                let mut ui_detents: Vec<Retained<UISheetPresentationControllerDetent>> =
                    Vec::with_capacity(detents.len().max(1));
                for &detent in detents {
                    let resolved = match detent {
                        SheetDetentSpec::Medium => {
                            UISheetPresentationControllerDetent::mediumDetent(mtm)
                        }
                        SheetDetentSpec::Large | SheetDetentSpec::FullScreen => {
                            UISheetPresentationControllerDetent::largeDetent(mtm)
                        }
                        SheetDetentSpec::Fraction(fraction) => {
                            let clamped = fraction.clamp(0.0, 1.0);
                            let resolver = RcBlock::new(
                                move |context: NonNull<
                                    ProtocolObject<
                                        dyn UISheetPresentationControllerDetentResolutionContext,
                                    >,
                                >| {
                                    // SAFETY: `UIKit` passes a valid non-null resolution context
                                    // for the duration of the resolver callback.
                                    let max = unsafe { context.as_ref().maximumDetentValue() };
                                    max * clamped
                                },
                            );
                            UISheetPresentationControllerDetent::customDetentWithIdentifier_resolver(
                                None, &resolver, mtm,
                            )
                        }
                        SheetDetentSpec::Height(height) => {
                            let target = height.max(0.0);
                            let resolver = RcBlock::new(
                                move |context: NonNull<
                                    ProtocolObject<
                                        dyn UISheetPresentationControllerDetentResolutionContext,
                                    >,
                                >| {
                                    // SAFETY: `UIKit` passes a valid non-null resolution context
                                    // for the duration of the resolver callback.
                                    let max = unsafe { context.as_ref().maximumDetentValue() };
                                    target.min(max)
                                },
                            );
                            UISheetPresentationControllerDetent::customDetentWithIdentifier_resolver(
                                None, &resolver, mtm,
                            )
                        }
                    };
                    ui_detents.push(resolved);
                }
                if ui_detents.is_empty() {
                    ui_detents.push(UISheetPresentationControllerDetent::largeDetent(mtm));
                }
                let array = NSArray::from_retained_slice(&ui_detents);
                sheet.setDetents(&array);
                sheet.setPrefersGrabberVisible(dismissible);
                sheet.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            }
        }
        Self {
            controller,
            delegate,
        }
    }

    /// Presents the sheet from the topmost controller of `host`'s window.
    /// Returns `false` when `host` has no window or root view controller.
    #[must_use]
    pub fn present(&self, host: &UIView) -> bool {
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

    /// Programmatically dismisses the sheet and runs `finished` once gone.
    pub fn dismiss(&self, finished: impl FnOnce() + 'static) {
        let finished = RefCell::new(Some(finished));
        let completion = RcBlock::new(move || {
            guarded("SheetController dismiss completion", || {
                if let Some(finished) = finished.borrow_mut().take() {
                    finished();
                }
            });
        });
        self.controller
            .dismissViewControllerAnimated_completion(true, Some(&completion));
    }

    /// Clears the interactive-dismiss delegate callback.
    pub fn clear_callbacks(&self) {
        *self.delegate.ivars().on_user_dismiss.borrow_mut() = None;
    }
}
