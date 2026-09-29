#[path = "engine/vello_backend.rs"]
pub(crate) mod legacy;

pub(crate) use legacy::{
    LegacyBumpReadback, LegacyRenderer, LegacyRendererOptions, legacy_init_threads,
    rasterize_blurred_silhouette,
};

pub use waterui_backend_core::widget::{
    Brush, DrawContext, RadioIndicatorState, RadioSelectionMotion, TextCaretMotion,
    TextContextMenuMetrics, WidgetTheme,
};

/// Marks the subtree of a button label that resolved to an icon-only
/// presentation.
///
/// The backend installs it into the icon-only label's environment so a
/// theme's resolvable [`WidgetTheme::button_label_color`] can paint the
/// standard icon button's content color rather than the filled text
/// button's — `ButtonStyle::Automatic` resolves to a different variant for
/// the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IconOnlyButtonLabel;

impl waterui::Plugin for IconOnlyButtonLabel {}
