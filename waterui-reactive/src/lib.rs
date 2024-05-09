#![no_std]
#![allow(incomplete_features)]

extern crate alloc;

pub mod binding;
use alloc::{borrow::Cow, string::String};
pub use binding::Binding;
pub mod compute;
pub use compute::{Compute, ComputeExt, Computed};
pub mod mpsc;
mod reactive;
pub use reactive::Reactive;
pub mod subscriber;
pub use subscriber::Subscriber;

#[macro_export]
macro_rules! impl_constant {
    ($($ty:ty),*) => {
        $(
            impl $crate::Compute for $ty {
                type Output = $ty;
                fn compute(&self) -> Self::Output {
                    self.clone()
                }
            }

            $crate::no_reactive!($ty);
        )*
    };
}

impl_constant!(i32, f64, bool, String, &'static str, Cow<'static, str>);

#[macro_export]
macro_rules! no_reactive {
    ($ty:ty) => {
        impl $crate::Reactive for $ty {
            fn register_subscriber(
                &self,
                _subscriber: $crate::subscriber::BoxSubscriber,
            ) -> Option<core::num::NonZeroUsize> {
                None
            }
            fn cancel_subscriber(&self, _id: core::num::NonZeroUsize) {}
            fn notify(&self) {}
        }
    };
}
