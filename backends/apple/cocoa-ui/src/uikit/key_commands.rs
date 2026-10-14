//! The callbacks behind the `UIKeyCommand`s a menu arms, and the
//! application-wide registry a fired command resolves them through.
//!
//! A `UIKeyCommand` carries no block: `UIKit` sends its action untargeted up
//! the responder chain, which ends at the application delegate. `UIKit` also
//! takes immutable copies of the menus it receives, so the command a chord or
//! a pick sends is a copy of the one a builder made, and only what `-copy`
//! preserves can lead it back to its callback — its `propertyList`, which
//! `UIKit` documents as the way to tell commands apart. Each armed command's
//! `propertyList` is its id in the [`KeyCommandRegistry`] the application
//! delegate owns, and the callback stays registered for as long as the
//! [`KeyCommands`] scope of the menu's owner does.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_foundation::NSNumber;
use objc2_ui_kit::UICommand;

/// The callback of every armed key command, by the id its `propertyList`
/// carries. The application delegate owns the one registry.
#[derive(Default)]
pub(super) struct KeyCommandRegistry {
    handlers: RefCell<HashMap<u64, Rc<dyn Fn()>>>,
    next: Cell<u64>,
}

impl KeyCommandRegistry {
    /// Registers `handler` under a fresh id.
    fn arm(&self, handler: Rc<dyn Fn()>) -> u64 {
        let id = self.next.get();
        self.next.set(id + 1);
        self.handlers.borrow_mut().insert(id, handler);
        id
    }

    /// Runs the callback of `command`, a key command a builder armed — or
    /// a copy `UIKit` made of one.
    ///
    /// # Panics
    ///
    /// If `command` does not carry an id this registry armed, or the scope
    /// that armed it has been dropped.
    pub(super) fn fire(&self, command: &UICommand) {
        let id = command
            .propertyList()
            .as_deref()
            .and_then(|list| list.downcast_ref::<NSNumber>())
            .map_or_else(
                || {
                    panic!(
                        "`cocoaUiMenuCommandFired:` was sent by a command no menu builder armed: {command:?}"
                    )
                },
                NSNumber::as_u64,
            );
        let handler =
            self.handlers.borrow().get(&id).cloned().unwrap_or_else(|| {
                panic!("key command {id} fired after its menu's owner retired it")
            });
        handler();
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
pub struct KeyCommands {
    registry: Rc<KeyCommandRegistry>,
    ids: RefCell<Vec<u64>>,
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

    /// An empty scope in `registry`.
    pub(super) const fn in_registry(registry: Rc<KeyCommandRegistry>) -> Self {
        Self {
            registry,
            ids: RefCell::new(Vec::new()),
        }
    }

    /// Registers `handler` for this scope's lifetime and returns the
    /// `propertyList` its key command carries.
    pub(super) fn arm(&self, handler: Rc<dyn Fn()>) -> Retained<NSNumber> {
        let id = self.registry.arm(handler);
        self.ids.borrow_mut().push(id);
        NSNumber::new_u64(id)
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
