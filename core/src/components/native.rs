//! This module provides platform-specific native views that can wrap platform-native UI components.

use core::any::type_name;

use crate::{Environment, View};

/// A wrapper for platform-specific native UI components.
///
/// `Native<T>` allows embedding platform-specific UI elements within the view hierarchy.
/// The generic parameter `T` represents the platform-specific component type.
///
/// # Panics
///
/// Attempting to render a `Native<T>` view directly will panic. This type is intended
/// to be handled by platform-specific rendering backends.
#[derive(Debug)]
pub struct Native<T>(pub T);

impl<T: 'static> View for Native<T> {
    fn body(self, _env: &Environment) -> impl View {
        panic!("Native view ({})", type_name::<T>())
    }
}
