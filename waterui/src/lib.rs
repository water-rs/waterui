#![no_std]
#![allow(non_camel_case_types)]

extern crate alloc;

#[macro_use]
mod macros;
pub mod component;
pub mod view;
pub mod widget;

use alloc::borrow::Cow;
pub use view::ViewExt;
#[doc(inline)]
pub use waterui_core::*;
pub mod modifier;
#[doc(inline)]
pub use waterui_reactive::{Binding, Compute, ComputeExt, Computed};
mod app;
pub use app::App;

pub mod layout;
pub mod utils;

pub(crate) type CowStr = Cow<'static, str>;
