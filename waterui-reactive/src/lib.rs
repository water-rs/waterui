#![no_std]
extern crate alloc;

pub mod binding;
pub use binding::Binding;
pub mod compute;
pub use compute::{Compute, ComputeExt, Computed};
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

                fn register_subscriber(&self, _subscriber: $crate::Subscriber) -> usize {
                    0
                }

                fn cancel_subscriber(&self, _id: usize) {}

                fn computed(self) -> $crate::Computed<Self::Output> {
                    $crate::Computed::new(self.clone())
                }
            }
        )*
    };
}

#[cfg(feature = "url")]
impl_constant!(url::Url);
