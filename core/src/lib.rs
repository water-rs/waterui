//! # `WaterUI` Core
//!
//! `waterui_core` provides the essential building blocks for developing cross-platform reactive UIs.
//! This foundation layer establishes a unified architecture that works consistently across desktop,
//! mobile, web, and embedded environments.

#![cfg_attr(feature = "nightly", feature(never_type))]
//!
//! ## Architecture Overview
//!
//! The system is structured around these key concepts:
//!
//! ### Declarative View System
//!
//! The [`View`] trait forms the foundation of the UI component model:
//! ```rust,ignore
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
//! use waterui_core::Environment;
//!
//! let env = Environment::new();
//! // .with() and .install() methods would be used with actual theme and plugin types
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
//! use waterui_core::{components::Dynamic,binding,Binding};
//!
//! // Create a reactive state container
//! let counter: Binding<i32> = binding(0);
//!
//! // Create a view that responds to state changes using Dynamic
//! let view = Dynamic::watch(counter, |count: i32| {
//!      format!("Current value: {}", count)
//! });
//! ```
//!
//! The UI automatically updates when state changes, with efficient rendering that only
//! updates affected components.
//!
//! ## Extensibility
//!
//! The plugin interface enables framework extensions without modifying core code:
//!
//! ```rust,ignore
//!
//! pub trait Plugin: Sized + 'static {
//!     fn install(self, env: &mut Environment);
//!     fn uninstall(self, env: &mut Environment);
//! }
//! ```
//!
//! This enables modular functionality like theming, localization, and platform-specific features.

#![cfg_attr(not(feature = "std"), no_std)]
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
pub use animation::AnimationExt;
pub use nami as reactive;
pub use nami::{Binding, Computed, Signal, SignalExt, binding, constant};
pub use waterui_str::Str;
pub mod id;
