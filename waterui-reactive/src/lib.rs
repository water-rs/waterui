#![no_std]
extern crate alloc;

pub mod binding;

pub use binding::{binding, Binding};
pub mod constant;
pub use constant::constant;
pub mod compute;
pub use compute::{Compute, ComputeExt, Computed};

pub mod collection;
pub mod mailbox;
pub mod map;
pub mod watcher;
pub mod zip;
