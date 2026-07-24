//! Material Design 3 snackbar convenience API.

use waterui::Str;
use waterui::snackbar::Snackbar;

/// A Material Design 3 snackbar configuration.
///
/// Rendering, queueing, dismissal, and motion are owned by `WaterUI`'s
/// `SnackbarManager`; this alias keeps the Material package API explicit while
/// preserving the existing runtime contract.
pub type MaterialSnackbar = Snackbar;

/// Creates a Material Design 3 snackbar.
#[must_use]
pub fn material_snackbar(message: impl Into<Str>) -> MaterialSnackbar {
    Snackbar::new(message)
}

#[cfg(test)]
mod tests {
    use crate::MaterialTheme;

    #[test]
    fn material_snackbar_tokens_match_mdui_2_1_5() {
        let theme = crate::layout::snackbar::theme(&MaterialTheme::new().colors());

        assert_eq!(theme.single_line_min_height, 48.0);
        assert_eq!(theme.corner_radius, 4.0);
        assert_eq!(theme.content_padding.top(), 12.0);
        assert_eq!(theme.content_padding.leading(), 16.0);
        assert_eq!(theme.content_spacing, 12.0);
        assert_eq!(theme.shadow_radius, 5.0);
        assert_eq!(theme.shadow_offset_y, 1.25);
        assert_eq!(theme.motion_offset_y, 20.0);
    }
}
