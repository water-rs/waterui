use waterui::graphics::color::Srgb;
use waterui::prelude::*;

pub fn control_shell<V: View>(content: V) -> impl View {
    vstack((content,))
        .spacing(12.0)
        .padding_with(16.0)
        .background(Srgb::BLACK)
}
