use core::time::Duration;

use waterui::animation::Animation;
use waterui::prelude::EdgeInsets;
use waterui::snackbar::SnackbarTheme;

use crate::theme::{colors::MaterialColorScheme, typography};

const SNACKBAR_HORIZONTAL_PADDING: f32 = 16.0;
const SNACKBAR_VERTICAL_PADDING: f32 = 12.0;
const SNACKBAR_VIEWPORT_PADDING: f32 = 16.0;
const SNACKBAR_CONTENT_SPACING: f32 = 12.0;
const SNACKBAR_MIN_WIDTH: f32 = 288.0;
const SNACKBAR_MAX_WIDTH: f32 = 568.0;
const SNACKBAR_ACTION_TRAILING_PADDING: f32 = 8.0;
const SNACKBAR_SINGLE_LINE_HEIGHT: f32 = 48.0;
const SNACKBAR_CORNER_RADIUS: f32 = 4.0;
const SNACKBAR_CLIP_RADIUS: f32 = 0.08;
const SNACKBAR_MOTION_OFFSET_Y: f32 = 20.0;

pub(crate) fn theme(colors: &MaterialColorScheme) -> SnackbarTheme {
    SnackbarTheme {
        container_color: colors.inverse_surface.view_color(),
        supporting_text_color: colors.inverse_on_surface.view_color(),
        action_label_color: colors.inverse_primary.view_color(),
        supporting_text_font: typography::body_medium(),
        action_label_font: typography::label_large(),
        content_padding: EdgeInsets::symmetric(
            SNACKBAR_VERTICAL_PADDING,
            SNACKBAR_HORIZONTAL_PADDING,
        ),
        viewport_padding: EdgeInsets::all(SNACKBAR_VIEWPORT_PADDING),
        content_spacing: SNACKBAR_CONTENT_SPACING,
        min_width: SNACKBAR_MIN_WIDTH,
        max_width: SNACKBAR_MAX_WIDTH,
        action_trailing_padding: SNACKBAR_ACTION_TRAILING_PADDING,
        single_line_min_height: SNACKBAR_SINGLE_LINE_HEIGHT,
        corner_radius: SNACKBAR_CORNER_RADIUS,
        clip_radius: SNACKBAR_CLIP_RADIUS,
        shadow_color: colors.shadow.view_color().with_opacity(0.2),
        shadow_radius: 3.0,
        shadow_offset_y: 3.0,
        motion_offset_y: SNACKBAR_MOTION_OFFSET_Y,
        enter_animation: Animation::bezier(Duration::from_millis(250), 0.0, 0.0, 0.0, 1.0),
        exit_animation: Animation::bezier(Duration::from_millis(250), 0.0, 0.0, 0.0, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SNACKBAR_CORNER_RADIUS, SNACKBAR_HORIZONTAL_PADDING, SNACKBAR_SINGLE_LINE_HEIGHT, theme,
    };
    use crate::MaterialTheme;
    use core::time::Duration;
    use waterui::animation::Animation;

    #[test]
    fn snackbar_theme_matches_material_web_v0_192_tokens() {
        let theme = theme(&MaterialTheme::new().colors());

        assert_eq!(theme.content_padding.leading(), SNACKBAR_HORIZONTAL_PADDING);
        assert_eq!(
            theme.content_padding.trailing(),
            SNACKBAR_HORIZONTAL_PADDING
        );
        assert_eq!(theme.single_line_min_height, SNACKBAR_SINGLE_LINE_HEIGHT);
        assert_eq!(theme.corner_radius, SNACKBAR_CORNER_RADIUS);
        assert_eq!(
            theme.enter_animation,
            Animation::bezier(Duration::from_millis(250), 0.0, 0.0, 0.0, 1.0)
        );
    }
}
