use crate::theme::colors::MaterialColorScheme;
use crate::{Brush, DividerMetrics, DrawContext};
use vello::kurbo::Rect;

pub const fn metrics() -> DividerMetrics {
    DividerMetrics::new(crate::dimensions::DIVIDER_THICKNESS)
}

pub fn draw(colors: &MaterialColorScheme, draw: &mut dyn DrawContext, bounds: Rect) {
    draw.fill_rect(bounds, &Brush::from(colors.outline_variant.peniko()));
}

#[cfg(test)]
mod tests {
    use super::metrics;
    use crate::dimensions::DIVIDER_THICKNESS;

    #[test]
    fn divider_uses_material_tokens() {
        let metrics = metrics();

        assert_eq!(metrics.thickness, DIVIDER_THICKNESS);
        assert_eq!(DIVIDER_THICKNESS, 1.0);
    }
}
