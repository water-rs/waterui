//! The callbacks behind the `UIKeyCommand`s a menu arms, and the
//! [`KeyCommandRegistry`] a fired command resolves them through — the
//! application menu bar's registry lives on the `UIApplicationDelegate`, a
//! view-owned menus arm their own.
//!
//! A `UIKeyCommand` carries no block: `UIKit` sends its action to a target —
//! the application delegate for the menu bar, the owning view for a
//! view's menu. `UIKit` also takes immutable copies of the menus it
//! receives, so the command a chord or a pick sends is a copy of the one a
//! builder made, and only what `-copy` preserves can lead it back to its
//! callback — its `propertyList`, which `UIKit` documents as the way to tell
//! commands apart. Each armed command's `propertyList` is an `NSUUID` token
//! in its owner's [`KeyCommandRegistry`], so a command that reaches the wrong
//! responder resolves nothing, and the callback stays registered for as long
//! as the [`KeyCommands`] scope of the menu's owner does.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::sel;
use objc2_foundation::{NSArray, NSObjectProtocol, NSString, NSUUID};
use objc2_ui_kit::{UICommand, UIKeyCommand};

/// The callback of every armed key command, by the id its `propertyList`
/// carries, owned by the menu bar's delegate or a view.
#[derive(Default)]
pub(super) struct KeyCommandRegistry {
    handlers: RefCell<HashMap<String, Rc<dyn Fn()>>>,
}

impl KeyCommandRegistry {
    /// The responder's merged key-command set: every command the
    /// superclass vends plus this registry's live commands. An inherited
    /// command carrying this registry's private selector survives only
    /// while still owned — stale copies of a retired scope are dropped.
    /// `UIKit` may copy a command between queries, so a candidate already
    /// present by `isEqual:` is not vended twice.
    pub(super) fn commands(
        &self,
        candidates: &[Retained<UIKeyCommand>],
        inherited: Option<Retained<NSArray<UIKeyCommand>>>,
        own_action: Sel,
    ) -> Option<Retained<NSArray<UIKeyCommand>>> {
        let mut merged: Vec<Retained<UIKeyCommand>> = Vec::new();
        if let Some(inherited) = inherited {
            for command in inherited {
                // SAFETY: `UIKeyCommand.action`'s declared getter.
                let action = unsafe { command.action() };
                if action != Some(own_action) || self.owns(&command) {
                    merged.push(command);
                }
            }
        }
        for command in candidates {
            if self.owns(command)
                && !merged
                    .iter()
                    .any(|existing| existing.isEqual(Some(command.as_ref())))
            {
                merged.push(command.clone());
            }
        }
        (!merged.is_empty()).then(|| NSArray::from_retained_slice(&merged))
    }

    /// Registers `handler` under a fresh `NSUUID` token — every field's own
    /// registry arms disjoint commands, so a command addressed to the wrong
    /// responder can never land.
    fn arm(&self, handler: Rc<dyn Fn()>) -> Retained<NSString> {
        let token = NSUUID::new().UUIDString();
        self.handlers
            .borrow_mut()
            .insert(token.to_string(), handler);
        token
    }

    /// The token `command`'s `propertyList` carries — `None` for a
    /// command no menu builder armed.
    fn token(command: &UICommand) -> Option<String> {
        command
            .propertyList()
            .as_deref()
            .and_then(|list| list.downcast_ref::<NSString>())
            .map(ToString::to_string)
    }

    /// `command` was armed in this registry — `false` for foreign or
    /// retired tokens, the ownership `canPerformAction` checks.
    pub(super) fn owns(&self, command: &UICommand) -> bool {
        Self::token(command).is_some_and(|token| self.handlers.borrow().contains_key(&token))
    }

    /// Any command is armed — the capability a nil-sender
    /// `canPerformAction` answers.
    pub(super) fn any_armed(&self) -> bool {
        !self.handlers.borrow().is_empty()
    }

    /// Runs the callback of `command`, a key command a builder armed — or
    /// a copy `UIKit` made of one.
    ///
    /// # Panics
    ///
    /// If `command` does not carry an id this registry armed, or the scope
    /// that armed it has been dropped.
    pub(super) fn fire(&self, command: &UICommand) {
        let token = Self::token(command).unwrap_or_else(|| {
            panic!("a menu-command action was sent by a command no menu builder armed: {command:?}")
        });
        let handler = self
            .handlers
            .borrow()
            .get(&token)
            .cloned()
            .unwrap_or_else(|| panic!("a key command fired after its owner retired it"));
        handler();
    }

    /// How many commands are armed — the native-test seam for scope
    /// retirement.
    #[cfg(feature = "native-test")]
    pub(super) fn armed(&self) -> usize {
        self.handlers.borrow().len()
    }
}

/// The key commands one menu owner has armed.
///
/// The owner is the menu bar, a context menu or a `Menu` trigger. Building a
/// menu arms each shortcut command in the scope it is given, and dropping the
/// scope retires them, so an owner keeps the scope beside the menu it built
/// and replaces both together.
///
/// A scope is dropped only once `UIKit` can no longer deliver its commands.
/// `UIKit` replaces an open menu's rows inside `setMenu:`, and closes a menu
/// whose presenting view leaves the window inside `removeFromSuperview`, its
/// rows taking no more input; so an owner hands `UIKit` the replacing menu
/// before it drops the previous scope, and a leaf's view leaves its window
/// before the leaf's state drops. A command that fires after its scope
/// retired means an owner broke that order, and the registry panics on it.
///
/// Mac Catalyst presents a button's menu as an `AppKit` `NSMenu` popup, which
/// meets neither behaviour: `setMenu:` leaves the popup's rows as they were
/// and `removeFromSuperview` leaves it tracking. A menu trigger there ends
/// the popup's tracking itself — when its menu is replaced while open, when
/// the button leaves its window, and when its owner is dropped — so the popup
/// closes before the scope retires and none of its rows can be picked
/// afterwards.
pub struct KeyCommands {
    registry: Rc<KeyCommandRegistry>,
    action: Sel,
    ids: RefCell<Vec<String>>,
}

impl KeyCommands {
    /// An empty scope in the running application's registry.
    ///
    /// # Panics
    ///
    /// If the application was not started through [`run`](super::run).
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self::in_registry(super::application::key_command_registry(mtm))
    }

    /// An empty scope in `registry`, routed through the application
    /// delegate's `cocoaUiMenuCommandFired:`.
    pub(super) fn in_registry(registry: Rc<KeyCommandRegistry>) -> Self {
        Self::in_registry_with_action(registry, sel!(cocoaUiMenuCommandFired:))
    }

    /// An empty scope in `registry`, armed commands sending `action`.
    pub(super) const fn in_registry_with_action(
        registry: Rc<KeyCommandRegistry>,
        action: Sel,
    ) -> Self {
        Self {
            registry,
            action,
            ids: RefCell::new(Vec::new()),
        }
    }

    /// The selector the scope's armed `UIKeyCommand`s send.
    pub(super) const fn action(&self) -> Sel {
        self.action
    }

    /// Registers `handler` for this scope's lifetime and returns the
    /// `propertyList` its key command carries.
    pub(super) fn arm(&self, handler: Rc<dyn Fn()>) -> Retained<NSString> {
        let token = self.registry.arm(handler);
        self.ids.borrow_mut().push(token.to_string());
        token
    }
}

impl Drop for KeyCommands {
    fn drop(&mut self) {
        let mut handlers = self.registry.handlers.borrow_mut();
        for id in self.ids.get_mut().drain(..) {
            handlers.remove(&id);
        }
    }
}

impl fmt::Debug for KeyCommands {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyCommands")
            .field("armed", &self.ids.borrow().len())
            .finish_non_exhaustive()
    }
}
