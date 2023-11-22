mod text;

pub use text::{text, Text};
mod button;
pub use button::{button, Button};
mod gesture;
pub use gesture::TapGesture;
pub mod stack;
pub use stack::Stack;
mod foreach;
pub use foreach::ForEach;

use crate::{reactive::Ref, view::BoxView, View};

mod calendar;

pub struct ReactiveView {
    reactive: Ref<()>,
    pub view: BoxView,
}

native_implement!(ReactiveView);

impl ReactiveView {
    pub fn new<T>(watch: Ref<T>, view: impl View) -> Self {
        let reactive = Ref::new(());
        watch.subcribe(reactive.clone());
        Self {
            reactive,
            view: Box::new(view),
        }
    }
}
