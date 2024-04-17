mod computed;

mod binding;
#[cfg(feature = "async")]
mod bridge;
#[cfg(feature = "async")]
pub use bridge::Bridge;

mod components;
mod ty;
pub(crate) use ty::*;
pub use ty::{AppClosure, Closure};
mod app;

#[doc(inline)]
pub use components::AnyView;

type Int = isize;
