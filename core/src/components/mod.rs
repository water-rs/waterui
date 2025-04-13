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
