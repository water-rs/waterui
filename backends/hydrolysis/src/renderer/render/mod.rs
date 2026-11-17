// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;

#[cfg(hydrolysis_cherenkov)]
pub use crate::text::FontFamilyResolution;
pub use crate::text::{ResolvedTextLayoutInput, TailMark, TextService, resolve_text_layout_input};

#[cfg(hydrolysis_cherenkov)]
mod compositor;
mod measurement;
mod measurement_cache;
mod render_context;
mod state;
mod view_helpers;

#[cfg(hydrolysis_cherenkov)]
pub use compositor::FrameError;
#[cfg(hydrolysis_cpu_painter)]
pub use compositor::FrameGeometry;
#[cfg(hydrolysis_cpu_painter)]
pub use compositor::RasterWindow;
#[cfg(hydrolysis_gpu_painter)]
pub use compositor::{CherenkovWindow, FrameRenderTarget};
#[cfg(any(
    feature = "list",
    feature = "navigation",
    feature = "picker",
    feature = "text-field",
    feature = "secure-field"
))]
pub use measurement::*;
pub use measurement_cache::{MeasurementCaches, MemoGate, NodeMeasureEntry};
#[cfg(feature = "navigation")]
pub use render_context::ChromeGroup;
#[cfg(any(feature = "text-field", feature = "secure-field"))]
pub use render_context::HydrolysisTextContextMenuMode;
#[cfg(any(feature = "menu", feature = "color-picker"))]
pub use render_context::HydrolysisWindowOrigin;
pub use render_context::RenderContext;
pub use render_context::{WidgetRenderContext, bounded_proposal};
pub use state::HydroState;
pub use view_helpers::*;
pub use view_helpers::{
    anchor_point, gesture_group_identity, normalize_layout_view, path_commands_to_path,
    resolved_shape_to_path, transformed_rect, working_color_to_rgba8,
};
