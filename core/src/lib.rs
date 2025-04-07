//! # WaterUI Core
//!
//! `waterui_core` is the foundational framework for building portable and reactive user interfaces
//! across multiple platforms. It provides the architecture, abstractions, and utilities to create
//! composable UI components with a declarative API.
//!
//! ## Core Architecture
//!
//! The framework is built around a few key abstractions:
//!
//! ### View System
//!
//! The [`View`] trait is the cornerstone of WaterUI's component model:
//!
//! ```rust
//! pub trait View: 'static {
//!     fn body(self, env: &Environment) -> impl View;
//! }
//! ```
//!
//! Views are composable UI components that can be combined, nested, and transformed.
//! They receive an environment context and return new views, forming a tree structure.
//!
//! ### Environment
//!
//! The [`Environment`] type provides a type-based storage mechanism for passing contextual
//! data through the view hierarchy:
//!
//! ```rust
//! let env = Environment::new()
//!     .with(Theme::Dark)
//!     .with(Locale::English);
//! ```
//!
//! Values in the environment can be retrieved by their type, allowing for dependency
//! injection and context propagation without explicit parameters.
//!
//! ### Type-Erased Views
//!
//! The [`AnyView`] type erases the concrete type of a view while preserving its behavior,
//! enabling heterogeneous collections of views and dynamic dispatch where needed.
//!
//! ## Component System
//!
//! WaterUI offers various component types to build UIs:
//!
//! - **Native components**: Platform-specific UI elements
//! - **Dynamic components**: Views that can change at runtime
//! - **Metadata components**: Views with attached metadata for renderers
//!
//! ## Reactive Programming
//!
//! The framework integrates with `waterui_reactive` for state management:
//!
//! ```rust
//! use waterui_reactive::{Binding, binding};
//! use waterui::dynamic::watch;
//!
//! let counter = binding(0);
//! let view = watch(counter, |count| text(format!("Count: {}", count)));
//! ```
//!
//! Changes to state automatically propagate to the UI.
//!
//! ## Foreign Function Interface
//!
//! A comprehensive FFI layer enables binding to native platforms:
//!
//! - C-compatible types and conversions
//! - Memory-safe wrappers for Rust values
//! - Callback system for cross-language communication
//!
//! ## Extension Points
//!
//! The plugin system allows extending the framework's functionality:
//!
//! ```rust
//! pub trait Plugin: Sized + 'static {
//!     fn install(self, env: &mut Environment);
//!     fn uninstall(self, env: &mut Environment);
//! }
//! ```
//!
//! Plugins can be installed into the environment to provide additional capabilities.

#![no_std]
#![allow(non_snake_case)]
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
