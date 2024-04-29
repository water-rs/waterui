#![no_std]
extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

#[macro_use]
mod macros;
mod anyview;
mod dynamic_view;
pub use anyview::AnyView;
pub use dynamic_view::{DynamicView, DynamicViewHandle};
#[cfg(feature = "async")]
pub mod async_view;
pub mod env;
pub mod view;
pub use env::Environment;
pub use view::View;
pub mod error;
pub use error::Error;
