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
//! use waterui_core::{Binding, Signal, SignalExt};
//!
//! // Create a reactive state container
//! let counter = Binding::container(0);
//!
//! // Derive the exact value consumed by a signal-aware component.
//! let label = counter.map(|count| format!("Current value: {count}"));
//! assert_eq!(label.get(), "Current value: 0");
//! ```
//!
//! Signal-aware component inputs subscribe to derived values and update only the
//! affected native property. Structural subtree replacement is reserved for cases
//! where the view identity itself changes.
//!
//! ## Extensibility
//!
//! The plugin interface enables framework extensions without modifying core code:
//!
//! ```rust
//! use waterui_core::{plugin::Plugin, Environment};
//!
//! struct MyPlugin;
//! impl Plugin for MyPlugin {}
//!
//! let mut env = Environment::new();
//! MyPlugin.install(&mut env);
//! ```
//!
//! This enables modular functionality like theming, localization, and platform-specific features.

#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

#[macro_use]
mod macros;
mod animation_system;
mod components;
mod foundation;
mod state;
mod ui;

pub use animation::{Animatable, AnimationExt, AnimationTrack};
pub use animation_system::{animation, easing, vector_arithmetic};
pub use anyhow::Error;
pub use anyview::AnyView;
pub use components::*;
pub use easing::{EasingCurve, Interpolatable};
pub use env::Environment;
pub use extract::State;
pub use foundation::main_thread::MainThreadBound;
pub use foundation::signal::flatten_signal;
pub use foundation::{env, extract, handler, id, main_thread, plugin, resolve};
pub use nami as reactive;
pub use nami::signal::IntoSignal;
pub use nami::{Binding, Computed, Signal, SignalExt, binding, constant, impl_constant};
pub use state::IntoSignalF32;
pub use ui::{accessibility, event, gesture, interaction, layout, view, view_renderer, views};
pub use view::View;
pub use view_renderer::{CustomViewRenderer, RenderResult, RenderSize, ViewRenderer};
pub use waterui_str::Str;
