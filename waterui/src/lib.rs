#![no_std]
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

pub mod ffi;
pub mod layout;
pub mod utils;
