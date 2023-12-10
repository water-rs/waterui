mod text;

pub use text::Text;
mod button;
pub use button::Button;
mod gesture;
pub use gesture::TapGesture;
mod stack;
pub use stack::{HStack, VStack};
mod image;
pub use image::RawImage;

pub fn vstack(contents: impl crate::view::IntoViews) -> VStack {
    VStack::new(contents)
}

pub fn hstack(views: impl crate::view::IntoViews) -> HStack {
    HStack::new(views)
}
