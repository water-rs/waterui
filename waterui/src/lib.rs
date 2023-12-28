#[macro_use]
mod macros;
pub mod attributed_string;
pub use attributed_string::AttributedString;
pub mod component;
pub mod view;
pub use view::{BoxView, View};
pub mod ffi;
pub mod modifier;
mod task;
pub use task::task;
pub mod env;
pub use env::Environment;
pub mod utils;
//pub mod window;
//pub use window::Window;
mod async_view;
pub mod layout;
pub use waterui_reactive::{
    binding::Binding,
    reactive::{IntoReactive, Reactive},
};
