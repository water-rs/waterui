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
pub mod channel;
pub mod debug;
#[macro_use]
pub mod macros;
//pub mod error;
mod ext;
//pub mod filter;
pub mod mailbox;
pub mod map;
//pub mod stream;
pub mod utils;
pub mod watcher;
pub mod zip;
#[doc(inline)]
pub use ext::ComputeExt;
#[macro_use]
pub mod ffi;

uniffi::setup_scaffolding!();

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

    impl_genetic_constant!(Vec<T>,BTreeMap<K,V>);

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
