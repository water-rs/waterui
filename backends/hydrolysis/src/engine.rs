#[path = "engine/cherenkov_backend.rs"]
pub mod cherenkov;

pub use cherenkov::{CherenkovSurface, GpuEngine, shared_engine};
// `macro_rules!` re-exports cap at `pub(crate)` — see cherenkov_backend.
pub(crate) use cherenkov::{cfg_async_fn, engine_await};

pub use waterui_backend_core::widget::{
    RadioIndicatorState, RadioSelectionMotion, TextCaretMotion, TextContextMenuMetrics, WidgetTheme,
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
