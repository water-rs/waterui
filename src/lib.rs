#![no_std]
#![allow(non_camel_case_types)]
#![warn(missing_debug_implementations)]
extern crate alloc;

#[macro_use]
mod macros;
pub mod background;
pub mod component;
pub mod filter;
pub mod task;
pub mod view;
pub mod widget;
pub use view::{View, ViewExt};
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
pub use waterui_core as core;
pub use waterui_layout as layout;
pub use waterui_str::Str;
pub use waterui_task::*;
