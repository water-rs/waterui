#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

mod binding;

pub use binding::Binding;
pub mod constant;
pub use constant::constant;
pub mod compute;
pub use compute::{Compute, ComputeExt, Computed};
pub mod cached;
pub mod flatten;
pub mod map;
pub mod watcher;
pub mod zip;

#[macro_export]
macro_rules! impl_constant {
    ($($ty:ty),*) => {
        $(
            impl $crate::compute::Compute for $ty {
                type Output=$ty;
                fn compute(&self) -> Self::Output{
                    self.clone()
                }
                fn add_watcher(&self, _watcher: $crate::watcher::Watcher<Self::Output>) -> $crate::watcher::WatcherGuard{
                    $crate::watcher::WatcherGuard::new(||{})
                }
            }
        )*
    };
}

impl_constant!(&'static str, bool, i32, f64);
