//! This module provides mechanisms for extracting values from the Environment.
//!
//! It defines the `Extractor` trait for types that can be extracted from an
//! Environment, along with implementations for common types.
//! The `Use<T>` wrapper provides a convenient way to extract specific types
//! from the environment, while [`State<T>`] supports cloneable state values
//! injected into the environment for action handlers.

use core::any::{TypeId, type_name};
use core::ops::{Deref, DerefMut};

use crate::Environment;
use alloc::{collections::BTreeMap, format};
use anyhow::Error;

/// A trait for extracting values from an Environment.
///
/// Types implementing this trait can be extracted from an Environment instance.
/// This is useful for dependency injection and accessing shared resources.
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
    fn extract_from_action(env: &Environment, state: &mut ExtractionState) -> Result<Self, Error> {
        let _ = state;
        Self::extract(env)
    }
}

/// Wrapper struct for values that need to be used from the Environment.
///
/// This wrapper enables extracting values by type from an Environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Use<T: 'static>(pub T);

/// Wrapper for cloneable state values injected into the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State<T: 'static>(pub T);

/// Per-action extraction state used to disambiguate repeated extractor types.
#[derive(Debug, Clone, Default)]
pub struct ExtractionState {
    positions: BTreeMap<TypeId, usize>,
}

impl ExtractionState {
    /// Returns the next extraction index for the requested type.
    #[must_use]
    pub fn next<T: 'static>(&mut self) -> usize {
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

impl<T> Deref for Use<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Use<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
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
        match T::extract_from_action(env, state) {
            Ok(value) => Ok(Some(value)),
            Err(_) => {
                *state = snapshot;
                Ok(None)
            }
        }
    }
}

impl<T: 'static + Clone> Extractor for Use<T> {
    /// Extracts a value of type T from the Environment.
    ///
    /// # Errors
    /// Returns an error if the requested type is not present in the Environment.
    fn extract(env: &Environment) -> Result<Self, Error> {
        env.get::<T>().map_or_else(
            || {
                Err(Error::msg(format!(
                    "Environment value `{}` not found",
                    type_name::<T>()
                )))
            },
            |value| Ok(Self(value.clone())),
        )
    }
}

impl<T: 'static + Clone> Extractor for State<T> {
    fn extract(env: &Environment) -> Result<Self, Error> {
        env.get::<Self>().map_or_else(
            || {
                Err(Error::msg(format!(
                    "Environment state `{}` not found",
                    type_name::<T>()
                )))
            },
            |value| Ok(value.clone()),
        )
    }

    fn extract_from_action(env: &Environment, state: &mut ExtractionState) -> Result<Self, Error> {
        let position = state.next::<Self>();
        env.get_nth::<Self>(position).map_or_else(
            || {
                Err(Error::msg(format!(
                    "Environment state `{}` not found at position {}",
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
