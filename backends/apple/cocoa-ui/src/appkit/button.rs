//! The `AppKit` button: an `NSButton` whose chrome is a bezel style and
//! whose label is the owner's own laid-out content.
//!
//! Link-style buttons get the pointing-hand cursor through cursor rects
//! (`resetCursorRects`), and press feedback through `mouseDown:`/`mouseUp:`
//! forwarded to an optional handler — `AppKit` draws no pressed state for a
//! transparent borderless button, so the owner dims its content itself.
//!
//! # Safety
//!
//! The `unsafe` here defines an `NSButton` subclass, forwards to `NSButton`'s
//! own implementation of each method it overrides, and calls `objc2`
//! bindings marked unsafe because `AppKit` is main-thread only — which the
//! `MainThreadOnly` thread kind and [`MainThreadMarker`] constructor
//! guarantee.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSBezelStyle, NSButton, NSButtonCell, NSColor, NSControl, NSCursor, NSEvent,
};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSAttributedString, NSObjectProtocol, NSString};

use crate::callback::guarded;
use crate::geometry::Size;

type PressHandler = Rc<dyn Fn(&Button, bool)>;

/// The interactive state a [`Button`] tracks: link cursor and press feedback.
#[derive(Default)]
pub struct ButtonIvars {
    /// Whether hovering shows the pointing-hand cursor (link style).
    link_cursor: Cell<bool>,
    /// Whether the button is currently pressed, for press-state callbacks.
    pressed: Cell<bool>,
    /// Whether the owner declared the button chrome-less — the link and
    /// borderless styles — so chrome presenting it keeps it bare too.
    borderless: Cell<bool>,
    /// Called with `true` on `mouseDown:` and `false` on `mouseUp:`.
    press_handler: RefCell<Option<PressHandler>>,
}

impl fmt::Debug for ButtonIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ButtonIvars")
            .field("link_cursor", &self.link_cursor.get())
            .field("pressed", &self.pressed.get())
            .field("borderless", &self.borderless.get())
            .field("press_handler", &self.press_handler.borrow().is_some())
            .finish()
    }
}

define_class!(
    // SAFETY: `NSButton`'s designated initializer is `initWithFrame:`, which
    // `Button::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSButton))]
    #[name = "CocoaUiButton"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ButtonIvars]
    #[derive(Debug)]
    /// An `NSButton` whose label content is laid out by its owner.
    pub struct Button;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSButton` subclass.
    unsafe impl NSObjectProtocol for Button {}

    impl Button {
        // SAFETY: see the module safety note.
        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            guarded("Button resetCursorRects", || {
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), resetCursorRects] };
                if self.ivars().link_cursor.get() {
                    self.addCursorRect_cursor(self.bounds(), &NSCursor::pointingHandCursor());
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            guarded("Button mouseDown:", || {
                self.ivars().pressed.set(true);
                let handler = self.ivars().press_handler.borrow().clone();
                if let Some(handler) = handler {
                    handler(self, true);
                }
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), mouseDown: event] };
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            guarded("Button mouseUp:", || {
                if self.ivars().pressed.replace(false) {
                    let handler = self.ivars().press_handler.borrow().clone();
                    if let Some(handler) = handler {
                        handler(self, false);
                    }
                }
                // SAFETY: see the module safety note.
                let _: () = unsafe { msg_send![super(self), mouseUp: event] };
            });
        }
    }
);

impl Button {
    /// An empty-titled button laid out by its owner.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ButtonIvars::default());
        // SAFETY: `initWithFrame:` is `NSButton`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: CGRect::ZERO] };
        this.setTitle(&NSString::new());
        this
    }

    /// The control, for wiring `ActionTarget`.
    #[must_use]
    pub fn control(&self) -> &NSControl {
        self
    }

    /// Draws or removes the button's bezel.
    pub fn set_bordered(&self, bordered: bool) {
        self.setBordered(bordered);
    }

    /// The bezel style drawn when the button is bordered.
    pub fn set_bezel_style(&self, style: NSBezelStyle) {
        self.setBezelStyle(style);
    }

    /// Makes the button fully transparent — the chrome of link-style and
    /// borderless buttons, whose own content is the only visible part.
    pub fn set_transparent(&self, transparent: bool) {
        self.setTransparent(transparent);
    }

    /// Records that the button's owner declared it chrome-less — the link
    /// and borderless styles. Chrome that presents the button as a toolbar
    /// item keeps it bare, reading [`Button::is_borderless`].
    pub fn set_borderless(&self, borderless: bool) {
        self.ivars().borderless.set(borderless);
    }

    /// Whether the button was declared chrome-less — [`Button::set_borderless`].
    #[must_use]
    pub fn is_borderless(&self) -> bool {
        self.ivars().borderless.get()
    }

    /// The color a prominent bezel is filled with; `None` restores the
    /// default.
    pub fn set_bezel_color(&self, color: Option<&NSColor>) {
        self.setBezelColor(color);
    }

    /// Enables or disables user interaction.
    pub fn set_enabled(&self, enabled: bool) {
        self.setEnabled(enabled);
    }

    /// Whether the button currently accepts interaction.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.isEnabled()
    }

    /// The plain-text title. A button whose owner lays out the label never
    /// sets one, but it is available for completeness.
    pub fn set_title(&self, title: &str) {
        self.setTitle(&NSString::from_str(title));
    }

    /// The styled title, as a single attributed run.
    pub fn set_attributed_title(&self, title: &NSAttributedString) {
        self.setAttributedTitle(title);
    }

    /// Shows the named SF Symbol as an icon-only button's content and
    /// clears any title — `accessibility_description` names the glyph for
    /// screen readers. Returns `false` when the platform catalog has no
    /// symbol of that name.
    pub fn set_symbol(&self, name: &str, accessibility_description: &str) -> bool {
        let Some(image) =
            crate::appkit::image::system_symbol(name, Some(accessibility_description))
        else {
            return false;
        };
        self.setImage(Some(&image));
        self.setTitle(&NSString::new());
        true
    }

    /// How far the bezel keeps content from the button's edge, read from the
    /// button cell's drawing rect for a reference-size button: the owning
    /// layout supplies this padding itself, like `NSButton`'s own layout.
    ///
    /// Meaningful only while the button is bordered.
    #[must_use]
    pub fn content_padding(&self) -> (f64, f64) {
        let probe = CGRect::new(
            objc2_core_foundation::CGPoint::ZERO,
            objc2_core_foundation::CGSize::new(200.0, 24.0),
        );
        let Some(cell) = self.cell() else {
            return (0.0, 0.0);
        };
        // SAFETY: `downcast` on an `NSButton`'s cell, which is always an
        // `NSButtonCell`.
        let Ok(cell) = cell.downcast::<NSButtonCell>() else {
            return (0.0, 0.0);
        };
        let drawing = cell.drawingRectForBounds(probe);
        (drawing.origin.x, drawing.origin.y)
    }

    /// Marks the button's accessibility role as a push button — needed
    /// while a transparent bezel makes it otherwise unrecognizable.
    pub fn mark_accessible_as_button(&self) {
        self.setAccessibilityRole(Some(&NSString::from_str("AXButton")));
    }

    /// Shows the pointing-hand cursor over the button — the link style's
    /// affordance — and asks the window to rebuild cursor rects.
    pub fn set_link_cursor(&self, enabled: bool) {
        self.ivars().link_cursor.set(enabled);
        if let Some(window) = self.window() {
            window.invalidateCursorRectsForView(self);
        }
    }

    /// The button's intrinsic size — what a measure pass reports.
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        let size = self.intrinsicContentSize();
        Size::new(size.width, size.height)
    }

    /// Names the button to a screen reader; `None` leaves it unnamed.
    pub fn set_accessibility_label(&self, label: Option<&str>) {
        self.setAccessibilityLabel(label.map(NSString::from_str).as_deref());
    }

    /// Calls `handler` when the press state changes: `true` on mouse down,
    /// `false` on release. `AppKit` draws no pressed state for a transparent
    /// borderless button; the owner uses this to dim its content.
    pub fn set_press_handler(&self, handler: impl Fn(&Self, bool) + 'static) {
        self.ivars().press_handler.replace(Some(Rc::new(handler)));
    }
}

// Referenced from `mod.rs` docs: the class registers as `CocoaUiButton`.
