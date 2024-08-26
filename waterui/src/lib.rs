#![no_std]
#![allow(non_camel_case_types)]
extern crate alloc;
#[macro_use]
mod macros;
pub mod animation;
pub mod component;
pub mod view;
pub mod widget;
pub use view::{View, ViewExt};
#[doc(inline)]
pub use waterui_core::{
    env::{self, Environment},
    AnyView,
};

#[doc(inline)]
pub use waterui_reactive::{Binding, Compute, ComputeExt, Computed};

pub mod layout;
pub mod utils;
pub use async_gcd::{future::block_on, task, Task};
