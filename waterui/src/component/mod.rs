mod text;

pub use text::{text, Text};
mod button;
pub use button::{button, Button};
mod gesture;
pub use gesture::TapGesture;
pub mod stack;
pub use stack::Stack;

mod image;
pub use image::Image;
mod date_picker;
pub use date_picker::DatePicker;
mod async_view;
pub use async_view::AsyncView;
mod frame;
pub use frame::FrameView;

pub struct UnreachableView;
impl crate::view::Reactive for UnreachableView {}

impl crate::view::View for UnreachableView {
    fn view(&self) -> crate::BoxView {
        unreachable!()
    }
}
