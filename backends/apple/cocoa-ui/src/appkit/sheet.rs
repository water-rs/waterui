//! `NSWindow` sheets presented modally on a parent window (water-rs/waterui#659).
//!
//! A [`SheetWindow`] owns a dedicated `NSWindow` subclass presented via
//! `NSWindow::beginSheet:completionHandler:`. `AppKit` enforces window-modal
//! inertness on the parent window automatically — blocking pointer, keyboard,
//! and `VoiceOver` access to the underlying content and restoring focus when the
//! sheet ends. Overriding `cancelOperation:` routes Escape / Command-Period to
//! the caller's dismiss callback.
//!
//! # Safety
//!
//! The `unsafe` here defines a minimal `NSWindow` subclass overriding
//! `cancelOperation:`, disables release-on-close so Rust's `Retained` keeps
//! sole ownership, and invokes `AppKit`'s main-thread sheet APIs.

use std::cell::RefCell;
use std::fmt;
use std::ptr;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSBackingStoreType, NSModalResponse, NSView, NSWindow, NSWindowStyleMask};
use objc2_core_foundation::CGRect;
use objc2_foundation::NSObjectProtocol;

use crate::callback::guarded;
use crate::geometry::{Rect, Size};
use crate::view;

type CancelCallback = Box<dyn FnMut()>;

struct SheetWindowIvars {
    on_cancel: RefCell<Option<CancelCallback>>,
}

impl fmt::Debug for SheetWindowIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SheetWindowIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSWindow` expects subclasses to initialize through
    // `initWithContentRect:styleMask:backing:defer:`, which `SheetWindow::new`
    // calls, and this class does not implement `Drop`.
    #[unsafe(super(NSWindow))]
    #[name = "CocoaUiSheetWindow"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SheetWindowIvars]
    #[derive(Debug)]
    struct CocoaUiSheetWindow;

    // SAFETY: `NSObjectProtocol` adds no invariants on an `NSWindow` subclass.
    unsafe impl NSObjectProtocol for CocoaUiSheetWindow {}

    impl CocoaUiSheetWindow {
        // SAFETY: `cancelOperation:` is `NSResponder`'s standard Escape /
        // Command-Period action on the main thread.
        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, _sender: Option<&AnyObject>) {
            guarded("SheetWindow cancelOperation:", || {
                let handler = self.ivars().on_cancel.borrow_mut().take();
                if let Some(mut handler) = handler {
                    handler();
                    let mut slot = self.ivars().on_cancel.borrow_mut();
                    if slot.is_none() {
                        *slot = Some(handler);
                    }
                }
            });
        }
    }
);

/// An `AppKit` window presented as a modal sheet on a parent `NSWindow`.
#[derive(Clone)]
pub struct SheetWindow {
    window: Retained<CocoaUiSheetWindow>,
}

impl fmt::Debug for SheetWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SheetWindow")
            .field("window", &self.window)
            .finish()
    }
}

impl SheetWindow {
    /// Creates a new sheet window sized to `size`.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, size: Size) -> Self {
        let allocated = mtm
            .alloc::<CocoaUiSheetWindow>()
            .set_ivars(SheetWindowIvars {
                on_cancel: RefCell::new(None),
            });
        let content_rect: CGRect =
            Rect::new(0.0, 0.0, size.width.max(1.0), size.height.max(1.0)).into();
        let style = NSWindowStyleMask::Titled;
        let backing = NSBackingStoreType::Buffered;
        // SAFETY: `initWithContentRect:styleMask:backing:defer:` is `NSWindow`'s
        // designated initializer on the main thread.
        let window: Retained<CocoaUiSheetWindow> = unsafe {
            msg_send![
                super(allocated),
                initWithContentRect: content_rect,
                styleMask: style,
                backing: backing,
                defer: false
            ]
        };
        // SAFETY: Rust's `Retained` owns the window lifetime; disabling
        // `releasedWhenClosed` prevents AppKit from over-releasing on close.
        unsafe { window.setReleasedWhenClosed(false) };
        Self { window }
    }

    /// Installs `view` as the sheet window's content view.
    pub fn set_content(&self, view: &NSView) {
        let this: &NSWindow = &self.window;
        if this
            .contentView()
            .is_some_and(|current| ptr::eq(&raw const *current, view))
        {
            return;
        }
        view::set_autoresizing_flexible_size(view);
        this.setContentView(Some(view));
    }

    /// Updates the sheet window's content size.
    pub fn set_content_size(&self, size: Size) {
        let this: &NSWindow = &self.window;
        this.setContentSize(Size::new(size.width.max(1.0), size.height.max(1.0)).into());
    }

    /// Installs the callback invoked when the user presses Escape or Command-Period
    /// while the sheet is active.
    pub fn set_on_cancel(&self, handler: impl FnMut() + 'static) {
        *self.window.ivars().on_cancel.borrow_mut() = Some(Box::new(handler));
    }

    /// Clears any installed cancel callback.
    pub fn clear_on_cancel(&self) {
        *self.window.ivars().on_cancel.borrow_mut() = None;
    }

    /// Presents this window as a modal sheet on `parent`; `on_end` runs once
    /// after `AppKit` ends the sheet.
    pub fn begin_sheet(&self, parent: &NSWindow, on_end: impl FnOnce() + 'static) {
        let on_end = RefCell::new(Some(on_end));
        let handler = RcBlock::new(move |_response: NSModalResponse| {
            guarded("SheetWindow completionHandler", || {
                if let Some(on_end) = on_end.borrow_mut().take() {
                    on_end();
                }
            });
        });
        let sheet: &NSWindow = &self.window;
        parent.beginSheet_completionHandler(sheet, Some(&handler));
    }

    /// Ends the modal sheet if it is currently attached to a parent window.
    pub fn dismiss(&self) {
        let sheet: &NSWindow = &self.window;
        if let Some(parent) = sheet.sheetParent() {
            parent.endSheet(sheet);
        }
    }

    /// Returns the underlying `NSWindow`.
    #[must_use]
    pub fn window(&self) -> &NSWindow {
        &self.window
    }
}
