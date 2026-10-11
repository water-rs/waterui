//! Borderless child panels floating above a parent window's content.
//!
//! A [`Panel`] is the `AppKit` surface for content that must hover over the
//! window — overlays anchored to a view, tooltips, popovers — without
//! activating or shadowing it. It is created hidden; [`Panel::attach`] makes
//! it a child of the anchor's window and [`Panel::detach`] pulls it back.
//!
//! # Safety
//!
//! The `unsafe` here initializes an `NSPanel`, disables its release-on-close
//! so the `Retained` this wrapper owns is not freed under it, and orders the
//! panel in and out of a parent window — all main-thread `AppKit` calls.

use std::ptr;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSPanel, NSView, NSWindow, NSWindowOrderingMode, NSWindowStyleMask,
};
use objc2_foundation::NSString;

use crate::geometry::Rect;

/// A borderless, non-activating panel presented as a child window.
pub struct Panel {
    panel: Retained<NSPanel>,
}

impl std::fmt::Debug for Panel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Panel").field("panel", &self.panel).finish()
    }
}

impl Panel {
    /// A transparent, shadowless panel that never takes activation.
    ///
    /// The panel draws into a buffer created immediately and stays alive
    /// across `orderOut:` calls — dropping this value is what ends it.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            Rect::ZERO.into(),
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        // SAFETY: see the module safety note.
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(false);
        Self { panel }
    }

    /// Makes the panel a child of `parent`, ordered above its content.
    /// Does nothing when the panel already has a parent window.
    pub fn attach(&self, parent: &NSWindow) {
        if self.panel.parentWindow().is_none() {
            // SAFETY: see the module safety note.
            unsafe { parent.addChildWindow_ordered(&self.panel, NSWindowOrderingMode::Above) };
        }
    }

    /// Removes the panel from its parent, orders it out and drops its
    /// content view.
    pub fn detach(&self) {
        if let Some(parent) = self.panel.parentWindow() {
            parent.removeChildWindow(&self.panel);
        }
        self.panel.orderOut(None);
        self.panel.setContentView(None);
    }

    /// Installs `view` as the panel's content view, resizing with the
    /// panel. No-op when `view` already is the content.
    pub fn set_content(&self, view: &NSView) {
        let this: &NSWindow = &self.panel;
        if this
            .contentView()
            .is_some_and(|current| ptr::eq(&raw const *current, view))
        {
            return;
        }
        crate::view::set_autoresizing_flexible_size(view);
        this.setContentView(Some(view));
    }

    /// Whether `window` is this panel — the comparison an outside-event
    /// check makes against the window an event was delivered to.
    #[must_use]
    pub fn is_window(&self, window: Option<&NSWindow>) -> bool {
        let this: &NSWindow = &self.panel;
        window.is_some_and(|window| ptr::eq(window, this))
    }

    /// Moves and resizes the panel to `frame`, in screen coordinates.
    pub fn set_frame(&self, frame: Rect) {
        self.panel.setFrame_display(frame.into(), true);
    }
}

/// The window `view` currently lives in, if any.
#[must_use]
pub fn window_of(view: &NSView) -> Option<Retained<NSWindow>> {
    // SAFETY: a main-thread read of the view hierarchy; `window` is the
    // accessor `AppKit` declares for it.
    view.window()
}

/// The bounds of `window`'s content view — the rectangle the window's
/// bottom-left coordinate space covers.
#[must_use]
pub fn content_bounds(window: &NSWindow) -> Rect {
    window.contentView().map_or(Rect::ZERO, |content| {
        let view: &NSView = &content;
        view.bounds().into()
    })
}

/// `rect`, expressed in `window`'s coordinate space, in screen coordinates.
#[must_use]
pub fn convert_to_screen(window: &NSWindow, rect: Rect) -> Rect {
    window.convertRectToScreen(rect.into()).into()
}

/// The name posted when `window`'s frame changes.
#[must_use]
pub fn did_resize_notification() -> crate::notification::NotificationName {
    // SAFETY: reads a string constant `AppKit` owns for the process's
    // lifetime.
    let name: &'static NSString = unsafe { objc2_app_kit::NSWindowDidResizeNotification };
    crate::notification::NotificationName::framework(name)
}

/// The name posted when a frame-notifying `NSView`'s frame changes; see
/// [`crate::view::set_posts_frame_changed`].
#[must_use]
pub fn view_frame_did_change_notification() -> crate::notification::NotificationName {
    // SAFETY: reads a string constant `AppKit` owns for the process's
    // lifetime.
    let name: &'static NSString = unsafe { objc2_app_kit::NSViewFrameDidChangeNotification };
    crate::notification::NotificationName::framework(name)
}
