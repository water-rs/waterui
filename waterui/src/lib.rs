#![no_std]
#![allow(non_camel_case_types)]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;
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
pub use waterui_reactive::{compute, Binding, Compute, ComputeExt, Computed};

pub mod layout;
pub mod utils;
pub use main_executor::{future::block_on, task, Task};
pub use waterui_str::Str;
