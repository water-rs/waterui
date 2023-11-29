#[macro_use]
mod macros;
pub mod attributed_string;
pub use waterui_core::binding::{self, Binding};
pub mod component;
pub mod view;
pub use view::{BoxView, View, ViewExt};
pub mod ffi;
pub mod renderer;
pub mod utils;
mod vdom;
pub mod window;
pub use window::Window;
pub trait Event: 'static {
    fn call_event(&self);
}
impl<F> Event for F
where
    F: 'static + Fn(),
{
    fn call_event(&self) {
        (self)()
    }
}

pub type BoxEvent = Box<dyn Event>;
pub use waterui_derive::widget;
