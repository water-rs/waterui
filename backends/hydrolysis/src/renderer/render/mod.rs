// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;

#[cfg(hydrolysis_gpu_painter)]
pub use crate::text::FontFamilyResolution;
pub use crate::text::{ResolvedTextLayoutInput, TailMark, TextService, resolve_text_layout_input};

#[cfg(hydrolysis_gpu_painter)]
mod compositor;
mod measurement;
mod measurement_cache;
mod render_context;
mod state;
mod subview;
mod view_helpers;

#[cfg(hydrolysis_gpu_painter)]
pub use compositor::HydrolysisRenderTarget;
#[cfg(hydrolysis_gpu_painter)]
pub use compositor::{CherenkovWindow, FrameRenderTarget};
pub use measurement::*;
pub use measurement_cache::{MeasurementCaches, MemoGate, NodeMeasureEntry};
pub use render_context::RenderContext;
pub use render_context::{ChromeGroup, WidgetRenderContext, bounded_proposal};
pub use render_context::{HydrolysisTextContextMenuMode, HydrolysisWindowOrigin};
pub use state::HydroState;
pub use subview::HydroSubview;
pub use view_helpers::*;
pub use view_helpers::{
    anchor_point, circle_arc_path, effective_stretch_axis, estimate_layout_intrinsic,
    gesture_group_identity, normalize_layout_view, normalize_view_for_render, passthrough_content,
    path_commands_to_path, resolved_shape_to_path, transformed_rect, working_color_to_rgba8,
};
