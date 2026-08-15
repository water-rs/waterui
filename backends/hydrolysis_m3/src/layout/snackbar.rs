use core::time::Duration;

use waterui::animation::Animation;
use waterui::prelude::EdgeInsets;
use waterui::snackbar::SnackbarTheme;

use crate::color::{InverseOnSurface, InversePrimary, InverseSurface, Shadow};
use crate::theme::{colors::MaterialColorScheme, typography};

const SNACKBAR_HORIZONTAL_PADDING: f32 = 16.0;
const SNACKBAR_VERTICAL_PADDING: f32 = 12.0;
const SNACKBAR_VIEWPORT_PADDING: f32 = 16.0;
const SNACKBAR_CONTENT_SPACING: f32 = 12.0;
const SNACKBAR_MIN_WIDTH: f32 = 288.0;
const SNACKBAR_MAX_WIDTH: f32 = 568.0;
const SNACKBAR_ACTION_TRAILING_PADDING: f32 = 8.0;
const SNACKBAR_SINGLE_LINE_HEIGHT: f32 = 48.0;
/// `IconButtonTokens.IconSize` — the size Compose draws `Icons.Filled.Close` at
/// in the snackbar's `dismissAction` slot.
const SNACKBAR_CLOSE_ICON_SIZE: f32 = 24.0;
/// `IconButtonTokens.StateLayerSize` — the size Compose's `IconButton` measures.
const SNACKBAR_CLOSE_STATE_LAYER_SIZE: f32 = 40.0;
const SNACKBAR_CORNER_RADIUS: f32 = 4.0;
const SNACKBAR_CLIP_RADIUS: f32 = 0.08;
const SNACKBAR_MOTION_OFFSET_Y: f32 = 20.0;

pub fn theme(_colors: &MaterialColorScheme) -> SnackbarTheme {
    SnackbarTheme {
        container_color: InverseSurface.into(),
        supporting_text_color: InverseOnSurface.into(),
        action_label_color: InversePrimary.into(),
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
        close_icon_size: SNACKBAR_CLOSE_ICON_SIZE,
        close_state_layer_size: SNACKBAR_CLOSE_STATE_LAYER_SIZE,
        single_line_min_height: SNACKBAR_SINGLE_LINE_HEIGHT,
        corner_radius: SNACKBAR_CORNER_RADIUS,
        clip_radius: SNACKBAR_CLIP_RADIUS,
        shadow_color: Shadow.with_opacity(0.19).into(),
        shadow_radius: 5.0,
        shadow_offset_y: 1.25,
        ambient_shadow_color: Shadow.with_opacity(0.039).into(),
        ambient_shadow_radius: 1.5,
        ambient_shadow_offset_y: 0.3333,
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
    fn snackbar_theme_matches_mdui_2_1_5_tokens() {
        let theme = theme(&MaterialTheme::new().colors());

        assert_eq!(theme.content_padding.leading(), SNACKBAR_HORIZONTAL_PADDING);
        assert_eq!(
            theme.content_padding.trailing(),
            SNACKBAR_HORIZONTAL_PADDING
        );
        assert_eq!(theme.single_line_min_height, SNACKBAR_SINGLE_LINE_HEIGHT);
        assert_eq!(theme.corner_radius, SNACKBAR_CORNER_RADIUS);
        assert_eq!(theme.shadow_radius, 5.0);
        assert_eq!(theme.shadow_offset_y, 1.25);
        assert_eq!(theme.ambient_shadow_radius, 1.5);
        assert_eq!(theme.ambient_shadow_offset_y, 0.3333);
        assert_eq!(
            theme.enter_animation,
            Animation::bezier(Duration::from_millis(250), 0.0, 0.0, 0.0, 1.0)
        );
    }
}
