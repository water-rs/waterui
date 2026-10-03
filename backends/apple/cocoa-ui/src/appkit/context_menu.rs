//! Contextual menus: an [`NSMenu`] popped at the pointer plus the
//! borderless accessory panel that may float above it.
//!
//! [`ContextMenu`] builds a menu from [`MenuTreeNode`]s and opens it under
//! the pointer for a `rightMouseDown` event; its `open`/`close` handlers
//! run on `menuWillOpen`/`menuDidClose`. [`AccessoryPanel`] is the
//! non-activating floating panel a menu presents above the source view.
//!
//! # Safety
//!
//! The `unsafe` here defines the delegate class `AppKit` calls and creates
//! the panel; `NSMenu` delegates are weak, so the [`ContextMenu`] value owns
//! the delegate for as long as the menu can be tracked.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSEvent, NSMenu, NSMenuDelegate, NSPanel, NSPopUpMenuWindowLevel, NSScreen, NSWindowStyleMask,
};
use objc2_foundation::{NSObjectProtocol, NSRect};

use crate::appkit::host_view::HostView;
use crate::appkit::menu::Menu;
use crate::callback::guarded;
use crate::geometry::{Rect, Size, anchored_screen_frame};
use crate::menu::MenuTreeNode;
use objc2_app_kit::{NSBackingStoreType, NSColor, NSView};

/// The air around an accessory, between it and the preview or the screen
/// edge.
const GAP: f64 = 8.0;
const EDGE_MARGIN: f64 = 8.0;

/// The screen frame of an accessory anchored to `source`'s frame.
fn accessory_screen_frame(source: &NSView, size: Size) -> Rect {
    let Some(window) = source.window() else {
        return Rect::ZERO;
    };
    let preview: Rect = window
        .convertRectToScreen(source.convertRect_toView(source.bounds(), None))
        .into();
    let screen_bounds: Rect = window
        .screen()
        .or_else(|| NSScreen::mainScreen(MainThreadMarker::from(source)))
        .map_or(NSRect::ZERO, |screen| screen.visibleFrame())
        .into();
    anchored_screen_frame(preview, size, screen_bounds, GAP, EDGE_MARGIN)
}

/// The closures a [`ContextMenuDelegate`] runs.
pub struct ContextMenuDelegateIvars {
    on_open: RefCell<Rc<dyn Fn()>>,
    on_close: RefCell<Rc<dyn Fn()>>,
}

impl fmt::Debug for ContextMenuDelegateIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContextMenuDelegateIvars").finish()
    }
}

define_class!(
    // SAFETY: `NSObject` asks a subclass to initialize through `init`, which
    // `ContextMenuDelegate::new` does, and the class does not implement
    // `Drop`.
    #[unsafe(super(objc2_foundation::NSObject))]
    #[name = "CocoaUiContextMenuDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ContextMenuDelegateIvars]
    #[derive(Debug)]
    /// The `NSMenuDelegate` of a [`ContextMenu`]: open and close hooks.
    struct ContextMenuDelegate;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for ContextMenuDelegate {}

    // SAFETY: the two optional methods `AppKit` calls through the delegate
    // protocol are implemented with their declared signatures, on the main
    // thread.
    unsafe impl NSMenuDelegate for ContextMenuDelegate {
        // SAFETY: see the module safety note.
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, _menu: &NSMenu) {
            guarded("ContextMenuDelegate menuWillOpen", || {
                let open = self.ivars().on_open.borrow().clone();
                open();
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(menuDidClose:))]
        fn menu_did_close(&self, _menu: &NSMenu) {
            guarded("ContextMenuDelegate menuDidClose", || {
                let close = self.ivars().on_close.borrow().clone();
                close();
            });
        }
    }
);

impl ContextMenuDelegate {
    fn new(mtm: MainThreadMarker, on_open: Rc<dyn Fn()>, on_close: Rc<dyn Fn()>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ContextMenuDelegateIvars {
            on_open: RefCell::new(on_open),
            on_close: RefCell::new(on_close),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// An `NSMenu` popped under the pointer, with open/close hooks.
///
/// `AppKit` holds the delegate weakly; keep this value for as long as the
/// menu can track.
#[derive(Debug)]
pub struct ContextMenu {
    menu: Menu,
    _delegate: Retained<ContextMenuDelegate>,
}

impl ContextMenu {
    /// A context menu of `nodes`. `on_open` runs when the menu begins
    /// tracking, `on_close` when it finishes.
    #[must_use]
    pub fn new(
        mtm: MainThreadMarker,
        nodes: &[MenuTreeNode],
        on_open: impl Fn() + 'static,
        on_close: impl Fn() + 'static,
    ) -> Self {
        let menu = Menu::new(mtm, "");
        menu.set_nodes(nodes);
        let delegate = ContextMenuDelegate::new(mtm, Rc::new(on_open), Rc::new(on_close));
        menu.menu()
            .setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        Self {
            menu,
            _delegate: delegate,
        }
    }

    /// Pops the menu up for `event` under `view`. Returns when tracking
    /// ends.
    pub fn pop_up(&self, view: &NSView, event: &NSEvent) {
        NSMenu::popUpContextMenu_withEvent_forView(self.menu.menu(), event, view);
    }

    /// Cancels the menu's tracking loop, if one is running.
    pub fn cancel(&self) {
        self.menu.menu().cancelTracking();
    }
}

/// The borderless, non-activating panel a context menu presents above its
/// source view: floating at the menu's level, clear-backed, shadowed.
///
/// The accessory inside is re-measured on every layout pass; when its ideal
/// size changes the panel re-anchors itself to the source view.
#[derive(Debug)]
pub struct AccessoryPanel {
    panel: Retained<NSPanel>,
    _container: Retained<HostView>,
}

impl AccessoryPanel {
    /// A panel holding `accessory`, anchored to `source` when presented.
    ///
    /// `ideal_size` measures the accessory when it is laid out.
    #[must_use]
    pub fn new(
        mtm: MainThreadMarker,
        source: &NSView,
        accessory: &NSView,
        ideal_size: impl Fn() -> Size + 'static,
    ) -> Self {
        // SAFETY: the style mask describes a borderless non-activating
        // panel.
        let panel = {
            NSPanel::initWithContentRect_styleMask_backing_defer(
                NSPanel::alloc(mtm),
                NSRect::ZERO,
                NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        panel.setFloatingPanel(true);
        panel.setLevel(NSPopUpMenuWindowLevel);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        // SAFETY: the panel is owned by this value and released when it
        // drops, not when `orderOut` runs.
        unsafe { panel.setReleasedWhenClosed(false) };

        let container = HostView::new(mtm, Rect::ZERO);
        let measured = Cell::new(Size::ZERO);
        let source: Retained<NSView> = Retained::from(source);
        let ideal_size = Rc::new(ideal_size);
        container.set_layout_handler({
            let panel = panel.clone();
            let source = source.clone();
            let accessory: Retained<NSView> = Retained::from(accessory);
            let ideal_size = ideal_size.clone();
            move |view| {
                accessory.setFrame(view.bounds());
                let size = ideal_size();
                if size != measured.get() {
                    measured.set(size);
                    panel.setFrame_display(accessory_screen_frame(&source, size).into(), true);
                }
            }
        });
        crate::view::add_subview(&container, accessory);
        panel.setContentView(Some(&container));
        panel.setFrame_display(accessory_screen_frame(&source, ideal_size()).into(), false);
        Self {
            panel,
            _container: container,
        }
    }

    /// Brings the panel to the front.
    pub fn order_front(&self) {
        self.panel.orderFront(None);
    }

    /// Takes the panel off screen.
    pub fn order_out(&self) {
        self.panel.orderOut(None);
    }
}
