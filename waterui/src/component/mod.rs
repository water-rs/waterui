mod text;

pub use text::Text;
mod button;
pub use button::Button;
mod stack;
pub use stack::{HStack, VStack};
mod image;
pub use image::Image;
mod text_field;
pub use text_field::TextField;

use crate::view::IntoViews;

pub fn vstack(contents: impl IntoViews) -> VStack {
    VStack::new(contents)
}

pub fn hstack(contents: impl IntoViews) -> HStack {
    HStack::new(contents)
}
