//! Metadata components for attaching arbitrary data to views.
//!
//! This module provides two types of metadata components:
//! - `Metadata<T>`: Strict metadata that must be handled by renderers
//! - `IgnorableMetadata<T>`: Optional metadata that can be safely ignored
//!
//! Metadata can be used to attach arbitrary data to views that will be processed
//! by renderers, such as accessibility attributes, transition effects, or custom
//! rendering instructions.

use crate::{AnyView, Environment, View};

/// Represents a view that carries additional metadata of type `T`.
///
/// This struct allows attaching arbitrary data to a view component. The metadata
/// is expected to be handled by a renderer, and will panic if not properly caught.
#[derive(Debug)]
#[must_use]
pub struct Metadata<T: 'static> {
    /// The view content wrapped by this metadata.
    pub content: AnyView,
    /// The metadata value associated with the content.
    pub value: T,
}

impl<T> Metadata<T> {
    /// Creates a new `Metadata` instance with the specified content and value.
    ///
    /// # Arguments
    ///
    /// * `content` - The view to be wrapped with metadata.
    /// * `value` - The metadata value to associate with the content.
    pub fn new(content: impl View, value: T) -> Self {
        Self {
            content: AnyView::new(content),
            value,
        }
    }
}

impl<T: 'static> View for Metadata<T> {
    fn body(self, _env: &Environment) -> impl View {
        panic!(
            "The metadata `{}`is not caught by your renderer. If the metadata is not essential, use `IgnorableMetadata<T>`.",
            core::any::type_name::<Self>()
        );
    }
}

/// A metadata wrapper that can be safely ignored by renderers if not handled explicitly.
///
/// Unlike `Metadata<T>`, this type won't panic if not caught by a renderer.
#[derive(Debug)]
pub struct IgnorableMetadata<T> {
    /// The view content wrapped by this ignorable metadata.
    pub content: AnyView,
    /// The metadata value associated with the content.
    pub value: T,
}

impl<T: 'static> View for IgnorableMetadata<T> {
    fn body(self, _env: &Environment) -> impl View {}
}
