//! Contextual menus: an [`NSMenu`] popped at the pointer, optionally
//! carrying an accessory as a custom-view row at its top.
//!
//! [`ContextMenu`] builds a menu from [`MenuTreeNode`]s and opens it under
//! the pointer for a `rightMouseDown` event; its `open`/`close` handlers
//! run on `menuWillOpen`/`menuDidClose`. [`ContextMenu::set_accessory`]
//! inserts a custom [`NSMenuItem`] view inside the menu.
//!
//! # Safety
//!
//! The `unsafe` here defines the delegate class `AppKit` calls and creates
//! the accessory item; `NSMenu` delegates are weak, so the [`ContextMenu`]
//! value owns the delegate for as long as the menu can be tracked.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSEvent, NSMenu, NSMenuDelegate, NSMenuItem, NSView};
use objc2_foundation::{NSObjectProtocol, NSString};

use crate::appkit::host_view::HostView;
use crate::appkit::menu::Menu;
use crate::callback::guarded;
use crate::geometry::{Rect, Size};
use crate::menu::MenuTreeNode;

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

    /// Inserts `accessory` as a custom-view row at the top of the menu,
    /// sized to `ideal_size`.
    ///
    /// `AppKit` delivers mouse events to a menu item's view, so interactive
    /// children (buttons) work; the view gets no keyboard input and the
    /// row's size is fixed for the tracking session, so `ideal_size` is
    /// read once at insertion. The menu retains the item, which retains
    /// the container view, for the menu's life.
    pub fn set_accessory(&self, accessory: &NSView, size: Size) {
        let mtm = self.menu.menu().mtm();
        let container = HostView::new(mtm, Rect::ZERO);
        container.set_layout_handler({
            let accessory: Retained<NSView> = Retained::from(accessory);
            move |view| accessory.setFrame(view.bounds())
        });
        container.setFrame(Rect::new(0.0, 0.0, size.width, size.height).into());
        crate::view::add_subview(&container, accessory);
        // SAFETY: an empty title, no action and no key equivalent make the
        // item a plain view row.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::new(),
                None,
                &NSString::new(),
            )
        };
        item.setView(Some(&container));
        item.setEnabled(true);
        self.menu.menu().insertItem_atIndex(&item, 0);
    }
}
