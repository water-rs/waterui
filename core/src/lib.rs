//! # WaterUI Core
//!
//! `waterui_core` provides the essential building blocks for developing cross-platform reactive UIs.
//! This foundation layer establishes a unified architecture that works consistently across desktop,
//! mobile, web, and embedded environments.
//!
//! ## Architecture Overview
//!
//! The system is structured around these key concepts:
//!
//! ### Declarative View System
//!
//! The [`View`] trait forms the foundation of the UI component model:
//!
//! ```rust
//! pub trait View: 'static {
//!     fn body(self, env: &Environment) -> impl View;
//! }
//! ```
//!
//! This recursive definition enables composition of complex interfaces from simple
//! building blocks. Each view receives contextual information and transforms into
//! its visual representation.
//!
//! ### Context Propagation
//!
//! The [`Environment`] provides a type-based dependency injection system:
//!
//! ```rust
//! let env = Environment::new()
//!     .with(Theme::Dark)
//!     .install(LocalizationPlugin::new("en_US"));
//! ```
//!
//! This propagates configuration and resources through the view hierarchy without
//! explicit parameter passing.
//!
//! ### Type Erasure
//!
//! [`AnyView`] enables heterogeneous collections by preserving behavior while
//! erasing concrete types, facilitating dynamic composition patterns.
//!
//! ## Component Architecture
//!
//! The framework provides several component categories:
//!
//! - **Platform Components**: Native UI elements with platform-optimized rendering
//! - **Reactive Components**: Views that automatically update when data changes
//! - **Metadata Components**: Elements that carry additional rendering instructions
//! - **Composite Components**: Higher-order components built from primitive elements
//!
//! ## Reactive Data Flow
//!
//! State management integrates seamlessly with the view system:
//!
//! ```rust
//! use waterui_reactive::{Binding, binding};
//! use waterui::components::Dynamic;
//!
//! // Create a reactive state container
//! let counter = binding(0);
//!
//! // Create a view that responds to state changes
//! let view = Dynamic::watch(counter, |count| {
//!     text(format!("Current value: {}", count))
//! });
//! ```
//!
//! The UI automatically updates when state changes, with efficient rendering that only
//! updates affected components.
//!
//! ## FFI System
//!
//! The framework includes a comprehensive interoperability layer:
//!
//! - **Memory Safety**: Safe Rust wrappers around C-compatible types
//! - **Zero-Copy Design**: Efficient data sharing between language boundaries
//! - **Callback Architecture**: Cross-language event handling
//! - **Resource Management**: Automatic cleanup of cross-language resources
//!
//! ## Extensibility
//!
//! The plugin interface enables framework extensions without modifying core code:
//!
//! ```rust
//! pub trait Plugin: Sized + 'static {
//!     fn install(self, env: &mut Environment);
//!     fn uninstall(self, env: &mut Environment);
//! }
//! ```
//!
//! This enables modular functionality like theming, localization, and platform-specific features.

#![no_std]
#![warn(missing_docs)]
#![feature(never_type)]
extern crate alloc;

#[macro_use]
mod macros;

pub mod components;
pub use components::anyview::AnyView;
pub mod env;
pub mod view;

pub use env::Environment;
pub use view::View;
pub mod extract;
pub mod handler;
pub mod plugin;
pub use anyhow::Error;
pub mod animation;
pub mod color;
pub use color::Color;
pub mod shape;
pub use waterui_reactive as reactive;
pub use waterui_reactive::{Binding, Compute, ComputeExt, Computed, binding, constant};
pub use waterui_str::Str;
pub mod ffi;
pub mod id;
