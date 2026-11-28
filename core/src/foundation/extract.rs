//! This module provides mechanisms for extracting values from the Environment.
//!
//! It defines the `Extractor` trait for types that can be extracted from an
//! Environment, along with implementations for common types.
//! [`State<T>`] supports cloneable state values injected into the environment
//! with `.state(&value)` or `App::state(&value)`, while `#[state]` implements
//! [`Extractor`] directly on owned `Clone` types over the same channel.

use core::any::{TypeId, type_name};
use core::fmt;
use core::ops::{Deref, DerefMut};

use crate::Environment;
use alloc::{collections::BTreeMap, format, rc::Rc};
use anyhow::Error;

/// A trait for extracting values from an Environment.
///
/// Types implementing this trait can be extracted from an Environment instance.
/// This is useful for dependency injection and accessing shared resources.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be extracted from a WaterUI environment",
    label = "expected a type implementing `Extractor`",
    note = "Handler and `use_env` parameters must be extractors: `#[state]` makes an owned `Clone` type extractable from `.state(&value)` injections, `State<T>` wraps an injected value of a foreign type, `Option<E>` tolerates a missing one, and tuples combine extractors."
)]
pub trait Extractor: 'static + Sized {
    /// Attempts to extract an instance of `Self` from the given environment.
    ///
    /// # Errors
    /// Returns an error if extraction fails, for example if the required value is not present in the environment.
    fn extract(env: &Environment) -> Result<Self, Error>;

    /// Attempts to extract an instance of `Self` from the given environment for
    /// an action handler invocation.
    ///
    /// This variant can track per-type extraction order when a single action
    /// asks for multiple instances of the same extractor, such as repeated
    /// [`State<T>`] parameters.
    ///
    /// # Errors
    ///
    /// Returns an error if extraction fails for the current action invocation.
    fn extract_from_action(env: &Environment, state: &mut ExtractionState) -> Result<Self, Error> {
        let _ = state;
        Self::extract(env)
    }
}

/// Wrapper for cloneable state values injected into the environment.
///
/// `.state(&value)` installs `State(value)` so handlers can recover it. For a
/// `Clone` type the app owns, `#[state]` implements [`Extractor`] on the type
/// itself and the wrapper disappears from handler signatures; `State<T>`
/// remains the extractor for values of foreign types such as `Binding<T>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State<T: 'static>(pub T);

/// Environment handle installed by dismissible presentations (`Sheet`,
/// `AnchoredOverlay`, and pushed `NavigationStack` pages) so handlers inside
/// can close the innermost presentation containing them.
///
/// ```rust
/// use waterui_core::{Environment, extract::{Dismiss, State}};
///
/// let closed = std::rc::Rc::new(std::cell::Cell::new(false));
/// let flag = std::rc::Rc::clone(&closed);
/// let env = Environment::new().extending(State(Dismiss::new(move || flag.set(true))));
/// let dismiss = env.extract::<Dismiss>().expect("dismiss handle installed");
/// dismiss.dismiss();
/// assert!(closed.get());
/// ```
#[waterui_macros::state]
#[derive(Clone)]
pub struct Dismiss {
    action: Rc<dyn Fn()>,
}

impl fmt::Debug for Dismiss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dismiss").finish_non_exhaustive()
    }
}

impl Dismiss {
    /// Creates a dismiss handle that invokes `action` when [`Self::dismiss`]
    /// is called.
    #[must_use]
    pub fn new(action: impl Fn() + 'static) -> Self {
        Self {
            action: Rc::new(action),
        }
    }

    /// Dismisses the innermost presentation that published this handle.
    pub fn dismiss(&self) {
        (self.action)();
    }
}

/// Per-action extraction state used to disambiguate repeated extractor types.
#[derive(Debug, Clone, Default)]
pub struct ExtractionState {
    positions: BTreeMap<TypeId, usize>,
}

impl ExtractionState {
    /// Returns the next extraction index for the requested type.
    #[must_use]
    pub fn take_next<T: 'static>(&mut self) -> usize {
        let position = self.positions.entry(TypeId::of::<T>()).or_insert(0);
        let current = *position;
        *position += 1;
        current
    }
}

impl Extractor for Environment {
    /// Extracts the Environment itself by creating a clone.
    fn extract(env: &Environment) -> Result<Self, Error> {
        Ok(env.clone())
    }
}

impl<T> Deref for State<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for State<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: Extractor> Extractor for Option<T> {
    /// Converts a regular extraction into an optional extraction.
    ///
    /// This implementation allows for graceful handling of extraction failures
    /// by converting the error case into a `None` value.
    fn extract(env: &Environment) -> Result<Self, Error> {
        Ok(<T as Extractor>::extract(env).ok())
    }

    fn extract_from_action(env: &Environment, state: &mut ExtractionState) -> Result<Self, Error> {
        let snapshot = state.clone();
        T::extract_from_action(env, state).map_or_else(
            |_| {
                *state = snapshot;
                Ok(None)
            },
            |value| Ok(Some(value)),
        )
    }
}

impl<T: 'static + Clone> Extractor for State<T> {
    fn extract(env: &Environment) -> Result<Self, Error> {
        env.get::<Self>().map_or_else(
            || {
                Err(Error::msg(format!(
                    "Environment state `{}` not found; install the value with \
                     `.state(&value)` in the handler's modifier chain, on an ancestor, \
                     or with `App::state` for an application-scoped handler",
                    type_name::<T>()
                )))
            },
            |value| Ok(value.clone()),
        )
    }

    fn extract_from_action(env: &Environment, state: &mut ExtractionState) -> Result<Self, Error> {
        let position = state.take_next::<Self>();
        env.get_nth::<Self>(position).map_or_else(
            || {
                Err(Error::msg(format!(
                    "Environment state `{}` not found at position {}; install \
                     the value with `.state(&value)` in the handler's modifier \
                     chain, on an ancestor, or with `App::state` for an \
                     application-scoped handler",
                    type_name::<T>(),
                    position
                )))
            },
            |value| Ok(value.clone()),
        )
    }
}

// Tuple extractors for combining multiple extractions
macro_rules! impl_tuple_extractor {
    ($($T:ident),+) => {
        impl<$($T: Extractor),+> Extractor for ($($T,)+) {
            fn extract(env: &Environment) -> Result<Self, Error> {
                Ok(($($T::extract(env)?,)+))
            }

            fn extract_from_action(
                env: &Environment,
                state: &mut ExtractionState,
            ) -> Result<Self, Error> {
                Ok(($($T::extract_from_action(env, state)?,)+))
            }
        }
    };
}

impl_tuple_extractor!(A, B);
impl_tuple_extractor!(A, B, C);
impl_tuple_extractor!(A, B, C, D);
impl_tuple_extractor!(A, B, C, D, E);
impl_tuple_extractor!(A, B, C, D, E, F);
impl_tuple_extractor!(A, B, C, D, E, F, G);
impl_tuple_extractor!(A, B, C, D, E, F, G, H);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Environment;
    use waterui_macros::state;

    /// The real `#[state]` expansion — inside `waterui-core` the macro resolves
    /// `crate::extract::…` through `proc_macro_crate`'s `Itself` branch.
    #[state]
    #[derive(Clone)]
    struct Editor {
        title: &'static str,
    }

    #[test]
    fn state_injected_owned_type_extracts_bare() {
        let env = Environment::new().extending(State(Editor { title: "draft" }));

        let editor = env.extract::<Editor>().expect("injected state extracts");
        assert_eq!(editor.title, "draft");
    }

    #[test]
    fn same_type_states_bind_positionally_and_share_positions() {
        // `.state(&first).state(&second)` layers the outer call's value below
        // the inner one's, so the nearest `State<Editor>` is `first`.
        let env = Environment::new()
            .extending(State(Editor { title: "second" }))
            .extending(State(Editor { title: "first" }));

        let mut state = ExtractionState::default();
        let bare = Editor::extract_from_action(&env, &mut state).expect("first parameter");
        // A `State<Editor>` parameter after a bare `Editor` draws the next
        // position of the same channel.
        let wrapped = <State<Editor> as Extractor>::extract_from_action(&env, &mut state)
            .expect("second parameter");

        assert_eq!(bare.title, "first");
        assert_eq!(wrapped.0.title, "second");
    }

    #[test]
    fn plain_environment_entry_does_not_satisfy_state_extractor() {
        let mut env = Environment::new();
        env.insert(Editor { title: "plain" });

        assert!(
            env.extract::<Editor>().is_err(),
            "plain Environment::insert must not satisfy #[state] / State<T> extraction"
        );
        assert!(
            env.extract::<State<Editor>>().is_err(),
            "plain Environment::insert must not satisfy State<T> extraction"
        );
    }

    #[test]
    fn dismiss_extracts_innermost_presentation_and_is_optional() {
        use core::cell::Cell;

        let empty = Environment::new();
        assert!(empty.extract::<Dismiss>().is_err());
        assert!(
            empty
                .extract::<Option<Dismiss>>()
                .expect("Option<Dismiss> extraction succeeds when absent")
                .is_none()
        );

        let outer_calls = Rc::new(Cell::new(0_u32));
        let inner_calls = Rc::new(Cell::new(0_u32));
        let outer = Dismiss::new({
            let outer_calls = Rc::clone(&outer_calls);
            move || outer_calls.set(outer_calls.get() + 1)
        });
        let inner = Dismiss::new({
            let inner_calls = Rc::clone(&inner_calls);
            move || inner_calls.set(inner_calls.get() + 1)
        });

        let outer_env = empty.extending(State(outer));
        let inner_env = outer_env.extending(State(inner));

        let mut state = ExtractionState::default();
        let extracted = Dismiss::extract_from_action(&inner_env, &mut state)
            .expect("innermost Dismiss extracts");
        extracted.dismiss();
        assert_eq!(inner_calls.get(), 1);
        assert_eq!(outer_calls.get(), 0);

        let outer_extracted = outer_env
            .extract::<Dismiss>()
            .expect("outer Dismiss extracts outside inner presentation");
        outer_extracted.dismiss();
        assert_eq!(outer_calls.get(), 1);
    }
}
