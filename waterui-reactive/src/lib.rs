#![no_std]
extern crate alloc;

pub mod binding;

use alloc::{boxed::Box, string::String, vec::Vec};
pub use binding::{binding, Binding};
pub mod constant;
use compute::ComputeResult;
pub use constant::constant;
pub mod compute;
pub use compute::{Compute, ComputeExt, Computed};
use watcher::WatcherGuard;
use waterui_str::Str;
pub mod flatten;
pub mod mailbox;
pub mod map;
pub mod watcher;
pub mod zip;

#[macro_export]
macro_rules! impl_constant {
    ($($ty:ty),*) => {
        $(
            impl $crate::compute::Compute for $ty {
                const CONSTANT: bool = true;
                type Output=$ty;
                fn compute(&self) -> Self::Output{
                    self.clone()
                }
                fn watch(&self, _watcher: impl Into<$crate::watcher::Watcher<Self::Output>>) -> $crate::watcher::WatcherGuard{
                    $crate::watcher::WatcherGuard::new(||{})
                }
            }
        )*
    };
}

impl_constant!(
    &'static str,
    bool,
    u8,
    u16,
    u32,
    u64,
    usize,
    i8,
    i16,
    i32,
    i64,
    f32,
    f64,
    String,
    Box<str>,
    Box<[u8]>,
    Str
);

impl<T: ComputeResult> Compute for Vec<T> {
    type Output = Self;
    fn compute(&self) -> Self::Output {
        self.clone()
    }
    fn watch(&self, _watcher: impl Into<watcher::Watcher<Self::Output>>) -> watcher::WatcherGuard {
        WatcherGuard::new(|| {})
    }
}
