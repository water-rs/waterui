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
    Environment,
    extract::State,
    handler::{
        BoxedEventAction, EventHandler, Handler, ReportExtraction as _, boxed_event_handler,
    },
    metadata::MetadataKey,
};

/// A key press delivered to an [`OnKeyPress`] handler.
///
/// Passed as the first argument to [`OnKeyPress::new`] / `ViewExt::on_key_press`.
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
    handler: BoxedEventAction<KeyPress, KeyHandling>,
}

impl MetadataKey for OnKeyPress {}

impl OnKeyPress {
    /// Creates a key handler that consumes the delivered [`KeyPress`] first,
    /// extracts any remaining arguments from the environment, and says whether
    /// it consumed the key.
    #[must_use]
    pub fn new<Args>(handler: impl EventHandler<KeyPress, Args, KeyHandling>) -> Self {
        Self {
            handler: boxed_event_handler(handler),
        }
    }

    /// Invokes the handler with an environment that carries [`State<KeyPress>`].
    ///
    /// A handler whose parameters cannot be extracted does not run: the
    /// failure is reported and the key is [`Ignored`](KeyHandling::Ignored),
    /// so it keeps bubbling.
    pub fn handle(&mut self, env: &Environment) -> KeyHandling {
        Handler::call(&mut |s: State<KeyPress>| s, env)
            .and_then(|State(press)| (self.handler)(press, env))
            .report_extraction()
            .unwrap_or(KeyHandling::Ignored)
    }
}

impl fmt::Debug for OnKeyPress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OnKeyPress").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nami::Signal;
    use waterui_core::Binding;

    #[test]
    fn on_key_press_delivers_typed_press_and_state() {
        let mut env = Environment::new();
        let handled_count = Binding::new(0_i32);
        let mut on_key = OnKeyPress::new(|press: KeyPress, State(count): State<Binding<i32>>| {
            if *press.key() == Key::Named(NamedKey::Escape) {
                *count.get_mut() += 1;
                KeyHandling::Handled
            } else {
                KeyHandling::Ignored
            }
        });
        env.insert(State(handled_count.clone()));
        let press_env = env.extending(State(KeyPress::new(
            Key::Named(NamedKey::Escape),
            Code::Escape,
            Modifiers::empty(),
            false,
        )));

        assert_eq!(on_key.handle(&press_env), KeyHandling::Handled);
        assert_eq!(handled_count.snapshot(), 1);
    }
}
