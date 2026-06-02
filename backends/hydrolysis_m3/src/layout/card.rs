use waterui::color::Color;
use waterui::widget::{CardStyle, CardStyleTokens, CardTheme};

use crate::theme::colors::MaterialColorScheme;

const CARD_CONTENT_PADDING: f32 = 16.0;
const CARD_CONTENT_SPACING: f32 = 4.0;
const CARD_CORNER_RADIUS: f32 = 12.0;
const CARD_CLIP_RADIUS: f32 = 0.08;
const CARD_OUTLINE_WIDTH: f32 = 1.0;

fn tokens(
    container_color: Color,
    outline_color: Color,
    outline_width: f32,
    shadow_color: Color,
    shadow_radius: f32,
    shadow_offset_y: f32,
) -> CardStyleTokens {
    CardStyleTokens {
        container_color,
        outline_color,
        outline_width,
        corner_radius: CARD_CORNER_RADIUS,
        clip_radius: CARD_CLIP_RADIUS,
        shadow_color,
        shadow_radius,
        shadow_offset_y,
    }
}

pub fn theme(colors: &MaterialColorScheme) -> CardTheme {
    let transparent_outline = colors.outline_variant.view_color().with_opacity(0.0);
    CardTheme {
        default_style: CardStyle::Filled,
        elevated: tokens(
            colors.surface_container_low.view_color(),
            transparent_outline.clone(),
            0.0,
            colors.shadow.view_color().with_opacity(0.18),
            1.0,
            1.0,
        ),
        filled: tokens(
            colors.surface_container_highest.view_color(),
            transparent_outline,
            0.0,
            colors.shadow.view_color().with_opacity(0.0),
            0.0,
            0.0,
        ),
        outlined: tokens(
            colors.surface.view_color(),
            colors.outline_variant.view_color(),
            CARD_OUTLINE_WIDTH,
            colors.shadow.view_color().with_opacity(0.0),
            0.0,
            0.0,
        ),
        content_padding: CARD_CONTENT_PADDING,
        content_spacing: CARD_CONTENT_SPACING,
    }
}

#[cfg(test)]
mod tests {
    use super::{CARD_CONTENT_PADDING, CARD_CORNER_RADIUS, theme};
    use crate::MaterialTheme;
    use waterui::widget::CardStyle;

    #[test]
    fn card_theme_matches_material_web_v0_192_tokens() {
        let theme = theme(&MaterialTheme::new().colors());

        assert_eq!(theme.default_style, CardStyle::Filled);
        assert_eq!(theme.content_padding, CARD_CONTENT_PADDING);
        assert_eq!(theme.filled.corner_radius, CARD_CORNER_RADIUS);
        assert_eq!(theme.outlined.outline_width, 1.0);
        assert_eq!(theme.elevated.shadow_radius, 1.0);
        assert_eq!(theme.filled.shadow_radius, 0.0);
    }
}
