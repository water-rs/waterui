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
/// Channel utilities for reactive communication.
pub mod channel;
/// Debug utilities for reactive computations.
pub mod debug;
#[macro_use]
/// Macros for reactive programming patterns.
pub mod macros;
/// Projection utilities for transforming reactive values.
pub mod project;
//pub mod ffi;
//pub mod error;
mod ext;
//pub mod filter;
/// Caching utilities for reactive computations.
pub mod cache;
/// Mailbox utilities for thread-safe reactive communication.
pub mod mailbox;
/// Mapping utilities for transforming reactive values.
pub mod map;
/// Stream utilities for reactive data flows.
pub mod stream;
/// Utility functions for reactive programming.
pub mod utils;
/// Watcher utilities for observing reactive changes.
pub mod watcher;
/// Zip utilities for combining reactive values.
pub mod zip;
#[doc(inline)]
pub use ext::ComputeExt;

/// Implements the `Compute` trait for constant types.
///
/// This macro generates `Compute` implementations for types that should be
/// treated as constant values in reactive computations. The implementation
/// simply returns a clone of the value and provides a no-op watcher.
///
/// # Usage
///
/// ```ignore
/// impl_constant!(String, i32, f64);
/// ```
#[macro_export]
macro_rules! impl_constant {



    ($($ty:ty),*) => {
         $(
            impl $crate::Compute for $ty {
                type Output = Self;

                fn compute(&self) -> Self::Output {
                    self.clone()
                }

                fn add_watcher(
                    &self,
                    _watcher: impl $crate::watcher::Watcher<Self::Output>,
                ) -> $crate::watcher::WatcherGuard {
                    $crate::watcher::WatcherGuard::new(|| {})
                }
            }
        )*
    };

}

/// Implements the `Compute` trait for generic constant types.
///
/// This macro generates `Compute` implementations for generic types that should be
/// treated as constant values in reactive computations. Similar to `impl_constant!`
/// but works with generic type parameters.
///
/// # Usage
///
/// ```ignore
/// impl_genetic_constant!(Vec<T>, Option<T>);
/// ```
#[macro_export]
macro_rules! impl_genetic_constant {

    ( $($ty:ident < $($param:ident),* >),* $(,)? ) => {
        $(
            impl<$($param: Clone + 'static),*> $crate::Compute for $ty<$($param),*> {
                type Output = Self;

                fn compute(&self) -> Self::Output {
                    self.clone()
                }

                fn add_watcher(
                    &self,
                    _watcher: impl $crate::watcher::Watcher<Self::Output>,
                ) -> $crate::watcher::WatcherGuard {
                    $crate::watcher::WatcherGuard::new(|| {})
                }
            }
        )*
    };




}

mod impl_constant {
    use alloc::borrow::Cow;
    use alloc::collections::BTreeMap;
    use core::time::Duration;
    use waterui_str::Str;

    use crate::Compute;
    use crate::watcher::WatcherGuard;
    impl_constant!(
        &'static str,
        u8,
        u16,
        u32,
        u64,
        i8,
        i16,
        i32,
        i64,
        f32,
        f64,
        bool,
        char,
        Duration,
        Str,
        String,
        Cow<'static, str>
    );

    impl_genetic_constant!(Vec<T>,BTreeMap<K,V>,Option<T>,Result<T,E>);

    impl<T> Compute for &'static [T] {
        type Output = &'static [T];
        fn compute(&self) -> Self::Output {
            self
        }
        fn add_watcher(
            &self,
            _watcher: impl crate::watcher::Watcher<Self::Output>,
        ) -> crate::watcher::WatcherGuard {
            WatcherGuard::new(|| {})
        }
    }
}
