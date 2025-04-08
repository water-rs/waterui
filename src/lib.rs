#![doc = include_str!("../README.md")]
#![no_std]
#![allow(non_camel_case_types)]
#![warn(missing_debug_implementations)]
#![warn(missing_docs)]

extern crate alloc;

#[macro_use]
mod macros;
pub mod background;
pub mod component;
pub mod filter;
pub mod task;
pub mod view;
pub mod widget;
#[doc(inline)]
pub use view::View;
#[doc(inline)]
pub use view::ViewExt;
pub mod ffi;
#[doc(inline)]
pub use waterui_core::{
    AnyView,
    env::{self, Environment},
    impl_extractor, raw_view,
};

#[doc(inline)]
pub use waterui_reactive::{Binding, Compute, Computed, compute};
mod ext;
pub use ext::ComputeExt;
pub use task::task;
#[doc(inline)]
pub use waterui_core as core;
#[doc(inline)]
pub use waterui_layout as layout;
pub use waterui_str::Str;
