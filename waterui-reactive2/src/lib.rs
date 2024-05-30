#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

mod binding;
pub use binding::Binding;
mod constant;
pub use constant::constant;
mod compute;
pub use compute::{Compute, ComputeExt, Computed};
mod flatten;
mod map;
mod watcher;
mod zip;
