use std::{collections::HashMap, ops::Deref};

#[macro_use]
mod macros;
pub mod attributed_string;
pub mod component;
pub mod ffi;
mod html;
pub mod reactive;
pub mod view;
use reactive::Ref;
pub use view::View;
use view::{downcast_view, BoxView, Renderer};
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
