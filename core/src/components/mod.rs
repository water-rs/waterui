//! Core Components
//!
//! This module provides the fundamental UI components used throughout the application.
//! It includes various view types and utilities for building user interfaces.
//!
//! ## Core View Types
//!
//! - `AnyView`: Type-erased view for heterogeneous collections
//! - `Native`: Platform-specific native UI elements
//! - `Dynamic`: Runtime-configurable views
//!
//! ## Metadata
//!
//! The module also provides facilities for attaching metadata to views.
pub mod anyview;
pub use anyview::AnyView;
pub mod dynamic;
mod label;
pub use dynamic::Dynamic;
pub mod metadata;
pub use metadata::{IgnorableMetadata, Metadata};

use crate::View;

/// A wrapper allows a view to carry an additional value without affecting its rendering.
#[derive(Debug, Clone)]
pub struct With<V, T> {
    view: V,
    #[allow(unused)]
    value: T,
}

impl<V: View, T: 'static> View for With<V, T> {
    fn body(self, _env: &crate::Environment) -> impl View {
        self.view
    }
}

impl<V, T> With<V, T> {
    /// Creates a new `With` instance that wraps a view and an additional value.
    pub const fn new(view: V, value: T) -> Self {
        Self { view, value }
    }
}
