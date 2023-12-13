mod text;

pub use text::Text;
mod button;
pub use button::Button;
mod gesture;
pub use gesture::TapGesture;
mod stack;
pub use stack::{HStack, VStack};
mod image;
mod menu;
pub use image::RawImage;
pub use menu::Action;
pub use menu::Menu;

use crate::view::IntoViews;

pub fn vstack(contents: impl IntoViews) -> VStack {
    VStack::new(contents)
}

pub fn hstack(contents: impl IntoViews) -> HStack {
    HStack::new(contents)
}
