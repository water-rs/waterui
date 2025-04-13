#![doc = include_str!("../README.md")]
extern crate alloc;

pub mod binding;
#[doc(inline)]
pub use binding::{Binding, binding};
pub mod constant;
#[doc(inline)]
pub use constant::constant;
pub mod compute;
#[doc(inline)]
pub use compute::{Compute, Computed};

pub mod collection;
mod ext;
pub mod mailbox;
pub mod map;
pub mod utils;
pub mod watcher;
pub mod zip;
#[doc(inline)]
pub use ext::ComputeExt;
#[macro_use]
pub mod ffi;

uniffi::setup_scaffolding!();
