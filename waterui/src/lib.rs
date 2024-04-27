#![no_std]
#![allow(non_camel_case_types)]
extern crate alloc;

#[macro_use]
mod macros;
pub mod component;
pub mod view;
pub use view::{View, ViewExt};

pub mod modifier;
#[doc(inline)]
pub use waterui_reactive::*;
#[doc(inline)]
pub use waterui_view::*;
pub mod app;
pub use app::App;

#[cfg(feature = "bridge")]
pub mod bridge;
pub mod layout;
pub mod utils;
