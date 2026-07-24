use waterui::color::Color;
use waterui::widget::{CardStyle, CardStyleTokens, CardTheme};

use crate::color::{OutlineVariant, Shadow, Surface, SurfaceContainerHighest, SurfaceContainerLow};
use crate::theme::colors::MaterialColorScheme;

const CARD_CONTENT_PADDING: f32 = 16.0;
const CARD_CONTENT_SPACING: f32 = 4.0;
const CARD_CORNER_RADIUS: f32 = 12.0;
const CARD_CLIP_RADIUS: f32 = 0.08;
const CARD_OUTLINE_WIDTH: f32 = 1.0;

struct CardShadowTokens {
    color: Color,
    radius: f32,
    offset_y: f32,
}

const fn shadow(color: Color, radius: f32, offset_y: f32) -> CardShadowTokens {
    CardShadowTokens {
        color,
        radius,
        offset_y,
    }
}

fn tokens(
    container_color: Color,
    outline_color: Color,
    outline_width: f32,
    shadow: CardShadowTokens,
    ambient_shadow: CardShadowTokens,
) -> CardStyleTokens {
    CardStyleTokens {
        container_color,
        outline_color,
        outline_width,
        corner_radius: CARD_CORNER_RADIUS,
        clip_radius: CARD_CLIP_RADIUS,
        shadow_color: shadow.color,
        shadow_radius: shadow.radius,
        shadow_offset_y: shadow.offset_y,
        ambient_shadow_color: ambient_shadow.color,
        ambient_shadow_radius: ambient_shadow.radius,
        ambient_shadow_offset_y: ambient_shadow.offset_y,
    }
}

pub fn theme(_colors: &MaterialColorScheme) -> CardTheme {
    let transparent_outline: Color = OutlineVariant.with_opacity(0.0).into();
    CardTheme {
        default_style: CardStyle::Filled,
        elevated: tokens(
            SurfaceContainerLow.into(),
            transparent_outline.clone(),
            0.0,
            shadow(Shadow.with_opacity(0.19).into(), 1.5, 0.5),
            shadow(Shadow.with_opacity(0.039).into(), 1.0, 0.0),
        ),
        filled: tokens(
            SurfaceContainerHighest.into(),
            transparent_outline,
            0.0,
            shadow(Shadow.with_opacity(0.0).into(), 0.0, 0.0),
            shadow(Shadow.with_opacity(0.0).into(), 0.0, 0.0),
        ),
        outlined: tokens(
            Surface.into(),
            OutlineVariant.into(),
            CARD_OUTLINE_WIDTH,
            shadow(Shadow.with_opacity(0.0).into(), 0.0, 0.0),
            shadow(Shadow.with_opacity(0.0).into(), 0.0, 0.0),
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
    fn card_theme_matches_mdui_2_1_5_tokens() {
        let theme = theme(&MaterialTheme::new().colors());

        assert_eq!(theme.default_style, CardStyle::Filled);
        assert_eq!(theme.content_padding, CARD_CONTENT_PADDING);
        assert_eq!(theme.filled.corner_radius, CARD_CORNER_RADIUS);
        assert_eq!(theme.outlined.outline_width, 1.0);
        assert_eq!(theme.elevated.shadow_radius, 1.5);
        assert_eq!(theme.elevated.ambient_shadow_radius, 1.0);
        assert_eq!(theme.elevated.ambient_shadow_offset_y, 0.0);
        assert_eq!(theme.filled.shadow_radius, 0.0);
    }
}
