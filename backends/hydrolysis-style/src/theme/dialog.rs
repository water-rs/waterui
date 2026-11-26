//! The dialog look of the theme contract: what a theme returns for a
//! self-drawn modal dialog card ([`DialogLook`]).

use waterui::layout::padding::EdgeInsets;
use waterui::theme::color::{Accent, Error, Foreground, MutedForeground, Surface};
use waterui_core::Environment;
use waterui_core::layout::Alignment;
use waterui_graphics::color::Color;

/// Visual and layout tokens used by Hydrolysis's self-drawn dialog card.
///
/// Style packages return a [`DialogLook`] from
/// [`WidgetTheme::dialog`](super::WidgetTheme::dialog) so Hydrolysis dialogs
/// match the active design system. [`DialogLook::default`] supplies neutral
/// baseline values built from standard theme color tokens.
#[derive(Debug, Clone)]
pub struct DialogLook {
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

impl Default for DialogLook {
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

/// Default framework dialog look used until a style package overrides
/// [`WidgetTheme::dialog`](super::WidgetTheme::dialog).
#[must_use]
pub fn default_dialog_look(_env: &Environment) -> DialogLook {
    DialogLook::default()
}
