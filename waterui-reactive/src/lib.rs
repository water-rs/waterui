#![no_std]
extern crate alloc;

pub mod binding;
use alloc::borrow::Cow;
pub use binding::Binding;
pub mod compute;
pub use compute::{Compute, ComputeExt, Computed};

pub mod subscriber;
pub use subscriber::Subscriber;

pub type CowStr = Cow<'static, str>;
#[macro_export]
macro_rules! impl_constant {
    ($($ty:ty),*) => {
        $(
            impl $crate::Compute for $ty {
                type Output = $ty;

                fn compute(&self) -> Self::Output {
                    self.clone()
                }

                fn register_subscriber(&self, _subscriber: $crate::Subscriber) -> Option<core::num::NonZeroUsize> {
                    None
                }

                fn cancel_subscriber(&self, _id: core::num::NonZeroUsize) {}
                fn notify(&self) {}

            }
        )*
    };
}

#[cfg(feature = "url")]
impl_constant!(url::Url);
