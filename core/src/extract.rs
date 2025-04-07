//! This module provides mechanisms for extracting values from the Environment.
//!
//! It defines the `Extractor` trait for types that can be extracted from an
//! Environment, along with implementations for common types.
//! The `Use<T>` wrapper provides a convenient way to extract specific types
//! from the environment.

use core::any::type_name;

use crate::Environment;
use alloc::format;
use anyhow::Error;
/// A trait for extracting values from an Environment.
///
/// Types implementing this trait can be extracted from an Environment instance.
/// This is useful for dependency injection and accessing shared resources.
pub trait Extractor: 'static + Sized {
    /// Attempts to extract an instance of `Self` from the given environment.
    ///
    /// # Parameters
    /// * `env` - The environment to extract from
    ///
    /// # Returns
    /// * `Result<Self, Error>` - The extracted value or an error if extraction failed
    fn extract(env: &Environment) -> Result<Self, Error>;
}

/// Wrapper struct for values that need to be used from the Environment.
///
/// This wrapper enables extracting values by type from an Environment.
pub struct Use<T: 'static>(pub T);

impl Extractor for Environment {
    /// Extracts the Environment itself by creating a clone.
    fn extract(env: &Environment) -> Result<Self, Error> {
        Ok(env.clone())
    }
}

impl<T: Extractor> Extractor for Option<T> {
    /// Converts a regular extraction into an optional extraction.
    ///
    /// This implementation allows for graceful handling of extraction failures
    /// by converting the error case into a `None` value.
    fn extract(env: &Environment) -> Result<Self, Error> {
        Ok(Extractor::extract(env).ok())
    }
}

impl<T: 'static + Clone> Extractor for Use<T> {
    /// Extracts a value of type T from the Environment.
    ///
    /// # Errors
    /// Returns an error if the requested type is not present in the Environment.
    fn extract(env: &Environment) -> Result<Self, Error> {
        if let Some(value) = env.get::<T>() {
            Ok(Self(value.clone()))
        } else {
            Err(Error::msg(format!(
                "Environment value `{}` not found",
                type_name::<T>()
            )))
        }
    }
}
