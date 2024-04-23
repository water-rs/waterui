pub mod computed;

pub mod binding;
#[cfg(feature = "async")]
mod bridge;
mod error;
#[cfg(feature = "async")]
pub use bridge::Bridge;

mod components;
mod ty;
pub(crate) use ty::*;
pub use ty::{AppClosure, Closure};
mod app;

#[doc(inline)]
pub use components::AnyView;

pub use modifier::WithValue;

type Int = isize;
mod modifier;
