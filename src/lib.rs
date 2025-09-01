#![doc = include_str!("../README.md")]

extern crate alloc;

#[macro_use]
mod macros;
pub mod background;
pub mod component;
pub mod filter;
/// Task management utilities and async support.
pub mod task;
pub mod view;
/// Widget components for building complex UI elements.
pub mod widget;
#[doc(inline)]
pub use view::View;
#[doc(inline)]
pub use view::ViewExt;
#[doc(inline)]
pub use waterui_core::{
    AnyView,
    env::{self, Environment},
    impl_extractor, raw_view,
};

#[doc(inline)]
pub use nami::{Binding, Computed, Signal, signal};
mod ext;
pub use ext::SignalExt;
pub use task::task;
#[doc(inline)]
pub use waterui_core as core;
#[doc(inline)]
pub use waterui_layout as layout;
pub use waterui_str::Str;
