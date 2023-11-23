#[macro_use]
mod macros;
pub mod attributed_string;
pub mod binding;
pub use binding::Binding;
pub mod component;
pub mod ffi;
mod html;
pub mod view;
pub use view::View;
pub mod utils;
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
