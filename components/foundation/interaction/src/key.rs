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

use waterui_core::{
    Environment, Error,
    extract::{ExtractionState, Extractor, State},
    handler::{BoxedAction, Handler, ReportExtraction, boxed_action},
    metadata::MetadataKey,
};

/// A key press delivered to an [`OnKeyPress`] handler.
///
/// Backends install it into the handler's environment via [`State<KeyPress>`],
/// so a handler reads it directly as `press: KeyPress` or `State(press): State<KeyPress>`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct KeyPress {
    /// What the key means under the current layout and modifiers.
    key: Key,
    /// Where the key sits on the keyboard, independent of layout.
    code: Code,
    /// The modifiers held with the key.
    modifiers: Modifiers,
    /// Whether this press is an auto-repeat of a held key.
    repeat: bool,
}

impl KeyPress {
    /// What the key means under the current layout and modifiers.
    #[must_use]
    pub const fn key(&self) -> &Key {
        &self.key
    }

    /// Where the key sits on the keyboard, independent of layout.
    #[must_use]
    pub const fn code(&self) -> Code {
        self.code
    }

    /// The modifiers held with the key.
    #[must_use]
    pub const fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Whether this press is an auto-repeat of a held key.
    #[must_use]
    pub const fn repeat(&self) -> bool {
        self.repeat
    }
}

impl KeyPress {
    /// A press of `key` at `code` with `modifiers` held, an auto-repeat when
    /// `repeat`.
    #[must_use]
    pub const fn new(key: Key, code: Code, modifiers: Modifiers, repeat: bool) -> Self {
        Self {
            key,
            code,
            modifiers,
            repeat,
        }
    }
}

impl Extractor for KeyPress {
    fn extract(env: &Environment) -> Result<Self, Error> {
        State::<Self>::extract(env).map(|State(press)| press)
    }

    fn extract_from_action(env: &Environment, state: &mut ExtractionState) -> Result<Self, Error> {
        State::<Self>::extract_from_action(env, state).map(|State(press)| press)
    }
}

/// Whether a key handler consumed a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "a key handler must say whether it consumed the key"]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed answer: a handler either consumed the key or did not"
)]
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
    /// [`KeyPress`] among them, and says whether it consumed the key.
    #[must_use]
    pub fn new<Args>(handler: impl Handler<Args, KeyHandling>) -> Self {
        Self {
            handler: boxed_action(handler),
        }
    }

    /// Invokes the handler with an environment that carries the [`KeyPress`].
    ///
    /// A handler whose parameters cannot be extracted does not run: the
    /// failure is reported and the key is [`Ignored`](KeyHandling::Ignored),
    /// so it keeps bubbling.
    pub fn handle(&mut self, env: &Environment) -> KeyHandling {
        (self.handler)(env)
            .report_extraction()
            .unwrap_or(KeyHandling::Ignored)
    }
}

impl fmt::Debug for OnKeyPress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OnKeyPress").finish_non_exhaustive()
    }
}
