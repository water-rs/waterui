//! Theme tokens for the self-drawn `Dialog` card.

use waterui_core::layout::Alignment;
use waterui_core::plugin::Plugin;
use waterui_graphics::color::Color;
use waterui_layout::padding::EdgeInsets;

use crate::theme::color::{Accent, Error, Foreground, MutedForeground, Surface};

/// Temporary compatibility shim for style packages migrating from `DialogTheme`
/// to `hydrolysis_style::theme::DialogLook` (`WidgetTheme::dialog`).
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct DialogTheme {
    /// Container surface fill color.
    pub container_color: Color,
    /// Headline text color.
    pub title_color: Color,
    /// Supporting message text color.
    pub message_color: Color,
    /// Emphasis color for the primary confirmation action.
    pub primary_action_color: Color,
    /// Emphasis color for destructive actions.
    pub destructive_action_color: Color,
    /// Text color for non-primary, non-destructive actions (such as cancel).
    pub default_action_color: Color,
    /// Container corner radius in logical points.
    pub corner_radius: f32,
    /// Inner padding around the dialog contents.
    pub content_padding: EdgeInsets,
    /// Minimum card width in logical points.
    pub min_width: f32,
    /// Maximum card width in logical points.
    pub max_width: f32,
    /// Vertical spacing between headline, supporting text, and the action row.
    pub content_spacing: f32,
    /// Extra top spacing above the action row.
    pub actions_top_padding: f32,
    /// Horizontal spacing between adjacent action buttons.
    pub actions_spacing: f32,
    /// Alignment of the action row inside the dialog card.
    pub actions_alignment: Alignment,
}

impl Default for DialogTheme {
    fn default() -> Self {
        Self {
            container_color: Color::new(Surface),
            title_color: Color::new(Foreground),
            message_color: Color::new(MutedForeground),
            primary_action_color: Color::new(Accent),
            destructive_action_color: Color::new(Error),
            default_action_color: Color::new(Foreground),
            // Neutral defaults so a build without a style package still draws
            // a readable, well-proportioned dialog card.
            corner_radius: 12.0,
            content_padding: EdgeInsets::all(20.0),
            min_width: 260.0,
            max_width: 480.0,
            content_spacing: 12.0,
            actions_top_padding: 8.0,
            actions_spacing: 8.0,
            actions_alignment: Alignment::Trailing,
        }
    }
}

impl Plugin for DialogTheme {}
