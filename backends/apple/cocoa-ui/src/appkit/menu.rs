//! Menus built from standard actions, plus the pieces a pull-down trigger
//! needs: per-item handlers, item presentation options, and
//! [`MenuButton`].
//!
//! # Safety
//!
//! The `unsafe` here creates menu items whose action is sent to the first
//! responder: the item has no target, so `AppKit` walks the responder chain
//! for an object that implements the action and disables the item when none
//! does. No action can reach an object that does not understand it.
//!
//! `with_action` instead installs a private target object on the item that
//! invokes a Rust closure; the item's weak target reference is kept alive
//! by the [`MenuItem`] itself. The attributed-title attribute used by
//! `with_destructive` is a static the platform exports, applied over the
//! item's whole title range, and `setTarget:`/`setAction:` are the
//! documented target/action setters.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use objc2::Message;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSColor, NSControlStateValueOff, NSControlStateValueOn, NSForegroundColorAttributeName,
    NSImage, NSMenu, NSMenuItem, NSPopUpButton, NSView,
};
use objc2_foundation::{
    NSMutableAttributedString, NSObject, NSObjectProtocol, NSRange, NSRect, NSString,
};

use crate::callback::guarded;
use crate::menu::{Command, KeyModifiers, MenuTreeNode};

/// A menu: a list of items, shown as the menu bar or as a submenu.
///
/// A `Menu` is a handle: clones refer to the same menu.
#[derive(Debug, Clone)]
pub struct Menu {
    menu: Retained<NSMenu>,
}

impl Menu {
    /// An empty menu titled `title`.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, title: &str) -> Self {
        Self {
            menu: NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title)),
        }
    }

    /// Appends `item`, which belongs to this menu from then on.
    pub fn add_item(&self, item: MenuItem) {
        let MenuItem { item, .. } = item;
        self.menu.addItem(&item);
    }

    /// Appends a separator line.
    pub fn add_separator(&self) {
        self.menu
            .addItem(&NSMenuItem::separatorItem(self.menu.mtm()));
    }

    pub(super) fn native(&self) -> &NSMenu {
        &self.menu
    }
}

/// One entry of a [`Menu`].
///
/// `title` is the label shown for the item; `action`, when given, is the
/// standard action message it sends up the responder chain when chosen;
/// `key_equivalent` is its keyboard-equivalent character.
#[derive(Debug)]
pub struct MenuItem {
    item: Retained<NSMenuItem>,
    /// Keeps the `with_action` closure target alive: `NSMenuItem` does not
    /// retain its target.
    action_target: Option<Retained<MenuItemTarget>>,
}

impl MenuItem {
    /// An item titled `title` that performs `action` when chosen, or does
    /// nothing when `action` is `None`.
    ///
    /// `key_equivalent` is the key that chooses the item with ⌘ held (a
    /// lowercase letter, or `""` for none); [`MenuItem::with_key_modifiers`]
    /// changes which modifiers it takes.
    #[must_use]
    pub fn new(
        mtm: MainThreadMarker,
        title: &str,
        action: Option<MenuAction>,
        key_equivalent: &str,
    ) -> Self {
        // SAFETY: see the module safety note.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                action.map(MenuAction::selector),
                &NSString::from_str(key_equivalent),
            )
        };
        Self {
            item,
            action_target: None,
        }
    }

    /// The same item, chosen by its key equivalent with exactly `modifiers`
    /// held instead of ⌘ alone.
    #[must_use]
    pub fn with_key_modifiers(self, modifiers: KeyModifiers) -> Self {
        self.item.setKeyEquivalentModifierMask(modifiers.native());
        self
    }

    /// The same item, opening `submenu` instead of performing an action.
    #[must_use]
    pub fn with_submenu(self, submenu: &Menu) -> Self {
        self.item.setSubmenu(Some(&submenu.menu));
        self
    }

    /// The same item, invoking `handler` when chosen instead of sending a
    /// standard action up the responder chain.
    #[must_use]
    pub fn with_action(mut self, handler: impl Fn() + 'static) -> Self {
        let target = MenuItemTarget::new(self.item.mtm(), Rc::new(handler));
        // SAFETY: `setTarget:`/`setAction:` are the documented way to point
        // an item at a per-item receiver; `menuItemFired:` is defined by the
        // target. The item holds the target weakly, so `self.action_target`
        // keeps it alive.
        unsafe {
            self.item.setTarget(Some(target.as_super()));
            self.item.setAction(Some(sel!(menuItemFired:)));
            // `setRepresentedObject:` retains the target for the item's
            // lifetime, so the handler is still valid once `add_item`
            // drops this `MenuItem`.
            self.item.setRepresentedObject(Some(
                std::ptr::from_ref::<MenuItemTarget>(target.as_ref())
                    .cast::<AnyObject>()
                    .as_ref()
                    .unwrap_unchecked(),
            ));
        }
        self.action_target = Some(target);
        self
    }

    /// The same item, greyed out and unselectable when `enabled` is false.
    #[must_use]
    pub fn with_enabled(self, enabled: bool) -> Self {
        self.item.setEnabled(enabled);
        self
    }

    /// The same item, drawn with a checkmark when `selected` is true.
    #[must_use]
    pub fn with_selected(self, selected: bool) -> Self {
        self.item.setState(if selected {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        self
    }

    /// The same item, subtitled `subtitle` beneath its title.
    ///
    /// Apply before [`MenuItem::with_destructive`]: an attributed title
    /// replaces the title drawing entirely.
    #[must_use]
    pub fn with_subtitle(self, subtitle: &str) -> Self {
        self.item.setSubtitle(Some(&NSString::from_str(subtitle)));
        self
    }

    /// The same item, drawn in the system's destructive red: an attributed
    /// title in `systemRed` replaces the plain title.
    #[must_use]
    pub fn with_destructive(self) -> Self {
        let mtm = self.item.mtm();
        let title = NSMutableAttributedString::initWithString(
            mtm.alloc::<NSMutableAttributedString>(),
            &self.item.title(),
        );
        let range = NSRange::new(0, title.length());
        let color = NSColor::systemRedColor();
        // SAFETY: `title` is a live mutable attributed string; the key is a
        // static the platform exports and an `NSColor` is the documented
        // value type for it.
        unsafe {
            title.addAttribute_value_range(NSForegroundColorAttributeName, color.as_ref(), range);
        }
        self.item.setAttributedTitle(Some(&title.into_super()));
        self
    }

    /// The same item, shown with the system symbol image `symbol_name`
    /// (an `SF Symbols` name); no image when the name is unknown.
    #[must_use]
    pub fn with_icon(self, symbol_name: &str) -> Self {
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(symbol_name),
            None,
        );
        self.item.setImage(image.as_deref());
        self
    }
}

/// A pull-down button that opens its menu: the trigger view a `Menu`
/// attaches to.
///
/// A `MenuButton` is a handle: clones refer to the same control.
#[derive(Debug, Clone)]
pub struct MenuButton {
    button: Retained<NSPopUpButton>,
}

impl MenuButton {
    /// A pull-down button showing an empty menu.
    ///
    /// Item enabling is manual: `autoenablesItems` would re-validate items
    /// whose `enabled` state callers manage themselves.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        let button =
            NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), NSRect::ZERO, true);
        button.setAutoenablesItems(false);
        Self { button }
    }

    /// Replaces the menu the button opens.
    pub fn set_menu(&self, menu: &Menu) {
        self.button.setMenu(Some(&menu.menu));
    }

    /// The button, for adding to a view hierarchy and framing.
    #[must_use]
    pub fn view(&self) -> &NSView {
        &self.button
    }
}

impl AsRef<NSView> for MenuButton {
    fn as_ref(&self) -> &NSView {
        &self.button
    }
}

/// A standard command a menu item sends to whichever object currently
/// handles it: the focused text view for editing commands, the key window for
/// window commands, the application for the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuAction {
    /// Shows the standard About panel.
    About,
    /// Hides the application.
    Hide,
    /// Hides every other application.
    HideOthers,
    /// Shows every hidden application.
    ShowAll,
    /// Quits the application.
    Quit,
    /// Undoes the last change.
    Undo,
    /// Redoes the last undone change.
    Redo,
    /// Cuts the selection to the pasteboard.
    Cut,
    /// Copies the selection to the pasteboard.
    Copy,
    /// Pastes the pasteboard's contents.
    Paste,
    /// Deletes the selection.
    Delete,
    /// Selects everything.
    SelectAll,
    /// Minimizes the key window into the Dock.
    Minimize,
    /// Toggles the key window between its standard and its user size.
    Zoom,
    /// Brings every window of the application to the front.
    BringAllToFront,
}

impl MenuAction {
    fn selector(self) -> Sel {
        match self {
            Self::About => sel!(orderFrontStandardAboutPanel:),
            Self::Hide => sel!(hide:),
            Self::HideOthers => sel!(hideOtherApplications:),
            Self::ShowAll => sel!(unhideAllApplications:),
            Self::Quit => sel!(terminate:),
            Self::Undo => sel!(undo:),
            Self::Redo => sel!(redo:),
            Self::Cut => sel!(cut:),
            Self::Copy => sel!(copy:),
            Self::Paste => sel!(paste:),
            Self::Delete => sel!(delete:),
            Self::SelectAll => sel!(selectAll:),
            Self::Minimize => sel!(miniaturize:),
            Self::Zoom => sel!(zoom:),
            Self::BringAllToFront => sel!(arrangeInFront:),
        }
    }
}

#[cfg(test)]
mod tests {
    use objc2_app_kit::NSEventModifierFlags;

    use super::KeyModifiers;

    #[test]
    fn modifiers_map_to_their_event_flags() {
        assert_eq!(
            (KeyModifiers::COMMAND | KeyModifiers::OPTION).native(),
            NSEventModifierFlags::Command | NSEventModifierFlags::Option
        );
        assert_eq!(
            (KeyModifiers::COMMAND | KeyModifiers::SHIFT).native(),
            NSEventModifierFlags::Command | NSEventModifierFlags::Shift
        );
        assert_eq!(
            KeyModifiers::CONTROL.native(),
            NSEventModifierFlags::Control
        );
        assert_eq!(
            KeyModifiers::empty().native(),
            NSEventModifierFlags::empty()
        );
    }
}

/// An `NSMenuItem` target that runs a Rust callback.
pub struct MenuItemTargetIvars {
    /// Called when the item is chosen.
    action: RefCell<Option<Rc<dyn Fn()>>>,
}

impl fmt::Debug for MenuItemTargetIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MenuItemTargetIvars").finish()
    }
}

define_class!(
    // SAFETY: `NSObject`'s designated initializer is `init`, which
    // `MenuItemTarget::new` calls, and the class does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiMenuItemTarget"]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuItemTargetIvars]
    #[derive(Debug)]
    /// The target `AppKit` calls when a callback menu item is chosen.
    pub struct MenuItemTarget;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject`.
    unsafe impl NSObjectProtocol for MenuItemTarget {}

    impl MenuItemTarget {
        // SAFETY: `menuItemFired:` is this class's own target action.
        #[unsafe(method(menuItemFired:))]
        fn menu_item_fired(&self, _sender: &NSMenuItem) {
            guarded("MenuItemTarget menuItemFired:", || {
                let handler = self.ivars().action.borrow().clone();
                if let Some(handler) = handler {
                    handler();
                }
            });
        }
    }
);

impl MenuItemTarget {
    fn new(mtm: MainThreadMarker, handler: Rc<dyn Fn()>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(MenuItemTargetIvars {
            action: RefCell::new(Some(handler)),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

impl MenuItem {
    /// An item from a [`Command`]. `action` runs when it is chosen.
    #[must_use]
    pub fn command(mtm: MainThreadMarker, command: &Command, action: Rc<dyn Fn()>) -> Self {
        // SAFETY: see the module safety note.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(&command.label),
                Some(sel!(menuItemFired:)),
                &NSString::from_str(&command.key_equivalent),
            )
        };
        let target = MenuItemTarget::new(mtm, action);
        // SAFETY: `setTarget:` accepts any object; the item holds it weakly,
        // so the target is retained as the item's `representedObject` as well.
        unsafe {
            item.setTarget(Some(
                std::ptr::from_ref::<MenuItemTarget>(target.as_ref())
                    .cast::<AnyObject>()
                    .as_ref()
                    .unwrap_unchecked(),
            ));
        };
        // SAFETY: `setRepresentedObject:` retains its argument.
        unsafe {
            item.setRepresentedObject(Some(
                std::ptr::from_ref::<MenuItemTarget>(target.as_ref())
                    .cast::<AnyObject>()
                    .as_ref()
                    .unwrap_unchecked(),
            ));
        }
        Self::apply_command(&item, command)
    }

    /// An item titled `label` that opens `submenu`.
    #[must_use]
    pub fn submenu(mtm: MainThreadMarker, command: &Command, submenu: &NSMenu) -> Self {
        // SAFETY: see the module safety note.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(&command.label),
                None,
                &NSString::new(),
            )
        };
        item.setSubmenu(Some(submenu));
        Self::apply_command(&item, command)
    }

    /// Applies a [`Command`]'s presentation to the item: enabled state,
    /// checkmark, key modifiers, symbol image, subtitle and the destructive
    /// style — `AppKit` draws a destructive command with the system-red
    /// title.
    fn apply_command(item: &NSMenuItem, command: &Command) -> Self {
        item.setEnabled(command.enabled);
        item.setState(if command.selected {
            objc2_app_kit::NSControlStateValueOn
        } else {
            objc2_app_kit::NSControlStateValueOff
        });
        item.setKeyEquivalentModifierMask(command.modifiers.native());
        if let Some(symbol) = &command.symbol
            && let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(symbol),
                None,
            )
        {
            item.setImage(Some(&image));
        }
        if let Some(subtitle) = &command.subtitle {
            item.setSubtitle(Some(&NSString::from_str(subtitle)));
        }
        if command.destructive {
            let title = NSMutableAttributedString::initWithString(
                item.mtm().alloc::<NSMutableAttributedString>(),
                &NSString::from_str(&command.label),
            );
            // SAFETY: `addAttribute:value:range:` takes any attribute-value
            // pair on a live attributed string.
            unsafe {
                title.addAttribute_value_range(
                    objc2_app_kit::NSForegroundColorAttributeName,
                    &NSColor::systemRedColor(),
                    NSRange::new(0, title.length()),
                );
            }
            item.setAttributedTitle(Some(&title));
        }
        Self {
            item: item.retain(),
            action_target: None,
        }
    }
}

impl Menu {
    /// Replaces the menu's contents with `nodes`.
    pub fn set_nodes(&self, nodes: &[MenuTreeNode]) {
        self.menu.removeAllItems();
        self.append_nodes(nodes);
    }

    /// Appends `nodes` after the menu's existing contents — the menu bar
    /// grows this way, its standard menus already in place.
    pub fn append_nodes(&self, nodes: &[MenuTreeNode]) {
        let mtm = self.menu.mtm();
        for node in nodes {
            match node {
                MenuTreeNode::Divider => self.add_separator(),
                MenuTreeNode::Command(command, action) => {
                    self.add_item(MenuItem::command(mtm, command, action.clone()));
                }
                MenuTreeNode::Submenu(command, children) => {
                    let submenu = Self::new(mtm, &command.label);
                    submenu.set_nodes(children);
                    self.add_item(MenuItem::submenu(mtm, command, &submenu.menu));
                }
            }
        }
    }

    /// The raw `NSMenu`.
    #[must_use]
    pub fn menu(&self) -> &NSMenu {
        &self.menu
    }
}
