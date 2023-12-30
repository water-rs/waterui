#[macro_use]
mod macros;
pub mod attributed_string;
pub use attributed_string::AttributedString;
pub mod component;
pub mod view;
pub use view::{View, ViewExt};
pub mod env;
pub mod ffi;
pub mod modifier;
pub use env::Environment;
pub mod app;
pub use app::App;
pub mod utils;
//pub mod window;
//pub use window::Window;
mod async_view;
pub mod layout;
pub use waterui_reactive::{
    binding::Binding,
    reactive::{IntoReactive, Reactive},
};

#[doc(hidden)]
pub use futures_lite::future::block_on as __block_on;
pub use waterui_derive::water_main;
