//! The sheet look of the theme contract: what a theme returns for a
//! self-drawn modal sheet surface ([`SheetLook`]).

use waterui_core::Environment;
use waterui_graphics::color::Color;
use waterui_graphics::color::token::{MutedForeground, Surface};

/// Baseline maximum width for a docked bottom sheet, in logical points.
const DEFAULT_SHEET_MAX_WIDTH: f32 = 640.0;
/// Baseline corner radius for a non-fullscreen sheet surface, in logical points.
const DEFAULT_SHEET_CORNER_RADIUS: f32 = 28.0;
/// Baseline width of the sheet drag handle pill, in logical points.
const DEFAULT_DRAG_HANDLE_WIDTH: f32 = 32.0;
/// Baseline height of the sheet drag handle pill, in logical points.
const DEFAULT_DRAG_HANDLE_HEIGHT: f32 = 4.0;
/// Baseline vertical padding above and below the drag handle pill, in logical points.
const DEFAULT_DRAG_HANDLE_VERTICAL_PADDING: f32 = 12.0;
/// Baseline opacity of the drag handle pill over `MutedForeground`.
const DEFAULT_DRAG_HANDLE_OPACITY: f32 = 0.4;

/// Visual and layout tokens used by Hydrolysis's self-drawn modal sheet surface.
///
/// Style packages return a [`SheetLook`] from
/// [`WidgetTheme::sheet`](super::WidgetTheme::sheet) so Hydrolysis sheets
/// match the active design system. [`SheetLook::default`] supplies neutral
/// baseline values built from standard theme color tokens.
#[derive(Debug, Clone)]
pub struct SheetLook {
    /// Container surface fill color.
    pub container_color: Color,
    /// Maximum width of a non-fullscreen sheet surface, in logical points.
    pub max_width: f32,
    /// Corner radius of a non-fullscreen sheet surface, in logical points.
    pub corner_radius: f32,
    /// Whether dismissible non-fullscreen sheets draw a top drag handle.
    pub show_drag_handle: bool,
    /// Fill color of the drag handle pill.
    pub drag_handle_color: Color,
    /// Width of the drag handle pill, in logical points.
    pub drag_handle_width: f32,
    /// Height of the drag handle pill, in logical points.
    pub drag_handle_height: f32,
    /// Vertical padding above and below the drag handle pill, in logical points.
    pub drag_handle_vertical_padding: f32,
}

impl Default for SheetLook {
    fn default() -> Self {
        Self {
            container_color: Color::new(Surface),
            max_width: DEFAULT_SHEET_MAX_WIDTH,
            corner_radius: DEFAULT_SHEET_CORNER_RADIUS,
            show_drag_handle: true,
            drag_handle_color: Color::new(MutedForeground)
                .with_opacity(DEFAULT_DRAG_HANDLE_OPACITY),
            drag_handle_width: DEFAULT_DRAG_HANDLE_WIDTH,
            drag_handle_height: DEFAULT_DRAG_HANDLE_HEIGHT,
            drag_handle_vertical_padding: DEFAULT_DRAG_HANDLE_VERTICAL_PADDING,
        }
    }
}

/// Default framework sheet look used until a style package overrides
/// [`WidgetTheme::sheet`](super::WidgetTheme::sheet).
#[must_use]
pub fn default_sheet_look(_env: &Environment) -> SheetLook {
    SheetLook::default()
}
