use waterui_graphics::color::ResolvedColor;

pub mod vello_backend;

pub use hydrolysis_m3::{Brush, DrawContext, MaterialTheme, WidgetTheme};

pub(crate) fn brush_from_resolved_color(value: ResolvedColor) -> Brush {
    let srgb = value.to_srgb_with_headroom();
    Brush::Solid(vello::peniko::Color::new([
        srgb.red,
        srgb.green,
        srgb.blue,
        value.opacity,
    ]))
}
