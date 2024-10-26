#![no_std]
#![allow(non_camel_case_types)]
#![warn(missing_debug_implementations)]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;
#[macro_use]
mod macros;
pub mod animation;
pub mod color;
pub mod component;
pub mod filter;
pub mod view;
pub mod widget;
use animation::Animation;
pub use view::{View, ViewExt};
use waterui_core::env::use_env;
#[doc(inline)]
pub use waterui_core::{
    env::{self, Environment},
    AnyView,
};

#[doc(inline)]
pub use waterui_reactive::{compute, Binding, Compute, ComputeExt, Computed};

pub mod layout;
pub mod utils;
pub use main_executor::{future::block_on, task, Task};
pub use waterui_str::Str;

pub fn use_state<T: 'static, V: View>(f: impl 'static + Fn(&T) -> V) -> impl View {
    use_env(move |env| f(env.get::<T>()))
}

pub trait AnimatedCompute: Compute {
    fn animated(self) -> impl Compute<Output = Self::Output>;
}

impl<C: Compute + 'static> AnimatedCompute for C {
    fn animated(self) -> impl Compute<Output = Self::Output> {
        self.with(Animation::Default)
    }
}
