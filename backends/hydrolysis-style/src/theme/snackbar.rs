//! The snackbar look of the theme contract: what a style package returns for
//! snackbar presentation ([`SnackbarLook`]).
//!
//! Hydrolysis measures, stacks, animates, and draws snackbar overlays from
//! [`SnackbarLook`]: container fill or glass backdrop, text/action typography
//! and colours, width hugging bounds, close affordance geometry, dual shadow
//! elevation, vertical slide travel, and entrance/exit animations.

use core::time::Duration;

use waterui::Environment;
use waterui::animation::Animation;
use waterui::background::Glass;
use waterui::layout::padding::EdgeInsets;
use waterui::theme::color::{Accent, Background, Foreground};
use waterui_graphics::color::Color;
use waterui_text::font::Font;

/// Visual tokens and motion governing how Hydrolysis presents a snackbar banner.
#[derive(Debug, Clone)]
pub struct SnackbarLook {
    /// Container fill colour when [`Self::background_glass`] is `None`.
    pub container_color: Color,
    /// Optional glass backdrop replacing the solid [`Self::container_color`] fill.
    pub background_glass: Option<Glass>,
    /// Supporting message text, leading icon, and close icon colour.
    pub supporting_text_color: Color,
    /// Action button label colour.
    pub action_label_color: Color,
    /// Supporting message font.
    pub supporting_text_font: Font,
    /// Action button label font.
    pub action_label_font: Font,
    /// Inner content padding when no trailing action/close control is present.
    pub content_padding: EdgeInsets,
    /// Margin between the snackbar and the host's safe-area bounds.
    pub viewport_padding: EdgeInsets,
    /// Horizontal spacing between message, action, and close controls.
    pub content_spacing: f32,
    /// Minimum container width in logical points (`0.0` hugs content).
    pub min_width: f32,
    /// Maximum container width in logical points.
    pub max_width: f32,
    /// Trailing padding used when an action or close button is present.
    pub action_trailing_padding: f32,
    /// Drawn size of the trailing close icon in logical points.
    pub close_icon_size: f32,
    /// Square hit-box / layout size around the close icon in logical points.
    pub close_state_layer_size: f32,
    /// Minimum height for single-line snackbars in logical points.
    pub single_line_min_height: f32,
    /// Vertical gap between stacked snackbars at the same placement.
    pub stack_gap: f32,
    /// Container corner radius in logical points.
    pub corner_radius: f32,
    /// Primary (key) shadow colour.
    pub shadow_color: Color,
    /// Primary (key) shadow blur radius.
    pub shadow_radius: f32,
    /// Primary (key) shadow vertical offset.
    pub shadow_offset_y: f32,
    /// Secondary (ambient) shadow colour.
    pub ambient_shadow_color: Color,
    /// Secondary (ambient) shadow blur radius.
    pub ambient_shadow_radius: f32,
    /// Secondary (ambient) shadow vertical offset.
    pub ambient_shadow_offset_y: f32,
    /// Entrance and dismissal vertical slide travel in logical points.
    pub motion_offset_y: f32,
    /// Entrance and stack-reflow animation.
    pub enter_animation: Animation,
    /// Dismissal animation.
    pub exit_animation: Animation,
}

/// Default framework snackbar look built from theme tokens, used when a style
/// package does not override [`WidgetTheme::snackbar`](super::WidgetTheme::snackbar).
#[must_use]
pub fn default_snackbar_look(_env: &Environment) -> SnackbarLook {
    SnackbarLook {
        container_color: Color::new(Foreground),
        background_glass: None,
        supporting_text_color: Color::new(Background),
        action_label_color: Color::new(Accent),
        supporting_text_font: Font::default(),
        action_label_font: Font::default(),
        content_padding: EdgeInsets::symmetric(12.0, 16.0),
        viewport_padding: EdgeInsets::all(16.0),
        content_spacing: 12.0,
        min_width: 288.0,
        max_width: 568.0,
        action_trailing_padding: 8.0,
        close_icon_size: 24.0,
        close_state_layer_size: 40.0,
        single_line_min_height: 48.0,
        stack_gap: 8.0,
        corner_radius: 6.0,
        shadow_color: Color::srgb(0, 0, 0).with_opacity(0.2),
        shadow_radius: 3.0,
        shadow_offset_y: 3.0,
        ambient_shadow_color: Color::srgb(0, 0, 0).with_opacity(0.0),
        ambient_shadow_radius: 0.0,
        ambient_shadow_offset_y: 0.0,
        motion_offset_y: 20.0,
        enter_animation: Animation::bezier(Duration::from_millis(250), 0.0, 0.0, 0.0, 1.0),
        exit_animation: Animation::bezier(Duration::from_millis(200), 0.3, 0.0, 1.0, 1.0),
    }
}
