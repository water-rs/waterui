//! Key presses a view can handle once its focused descendant leaves them
//! unconsumed.
//!
//! The keyboard goes to the focused view first — a text field, an embedded
//! surface. A key that view does not use, such as Escape or an arrow in a
//! single-line field, bubbles to its ancestors, nearest first, and stops at
//! the first [`OnKeyPress`] handler that returns [`KeyHandling::Handled`]. A
//! handler attached near the root therefore sees every key nothing below it
//! claimed, which is how an overlay owns Escape while its field keeps focus.
//!
//! Keys use the W3C UI Events vocabulary from [`keyboard_types`]: the
//! layout-aware [`Key`] for what the key means and the layout-independent
//! [`Code`] for where it sits.

use core::fmt;

pub use keyboard_types::{Code, Key, Modifiers, NamedKey};

use crate::{
    handler::{BoxedAction, Handler, boxed_action},
    metadata::MetadataKey,
};

/// A key press delivered to an [`OnKeyPress`] handler.
///
/// Backends place it into the handler's environment, so a handler reads it
/// with [`Use<KeyPress>`](crate::extract::Use).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPress {
    /// What the key means under the current layout and modifiers.
    pub key: Key,
    /// Where the key sits on the keyboard, independent of layout.
    pub code: Code,
    /// The modifiers held with the key.
    pub modifiers: Modifiers,
    /// Whether this press is an auto-repeat of a held key.
    pub repeat: bool,
}

/// Whether a key handler consumed a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "a key handler must say whether it consumed the key"]
pub enum KeyHandling {
    /// The key was consumed; it stops bubbling.
    Handled,
    /// The key was not used; it keeps bubbling to the next ancestor.
    Ignored,
}

/// A handler for key presses left unconsumed by the focused view and its
/// descendants.
pub struct OnKeyPress {
    handler: BoxedAction<KeyHandling>,
}

impl MetadataKey for OnKeyPress {}

impl OnKeyPress {
    /// Creates a key handler. It extracts its arguments from the environment,
    /// [`Use<KeyPress>`](crate::extract::Use) among them, and says whether it
    /// consumed the key.
    #[must_use]
    pub fn new<Args>(handler: impl Handler<Args, KeyHandling>) -> Self {
        Self {
            handler: boxed_action(handler),
        }
    }

    /// Invokes the handler with an environment that carries the [`KeyPress`].
    pub fn handle(&mut self, env: &crate::Environment) -> KeyHandling {
        (self.handler)(env)
    }
}

impl fmt::Debug for OnKeyPress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OnKeyPress").finish_non_exhaustive()
    }
}
