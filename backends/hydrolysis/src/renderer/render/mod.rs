use super::*;

mod compositor;
mod measurement;
mod measurement_cache;
mod render_context;
mod state;
mod subview;
mod text_service;
mod view_helpers;

pub use compositor::HydrolysisRenderTarget;
#[cfg(hydrolysis_macos_system_webview)]
pub(crate) use compositor::NativeViewLayer;
#[cfg(test)]
pub(crate) use compositor::take_gpu_surface_redraw_request;
pub(crate) use compositor::{
    ActiveSceneLayer, Compositor, EmbeddedGpuSurfaceRuntime, GpuSurfaceLayer, GpuSurfaceSource,
    LayerShape, RenderLayer, covers_viewport_directly,
};
pub(crate) use measurement::*;
pub(crate) use measurement_cache::MeasurementCaches;
pub use render_context::RenderContext;
pub(crate) use render_context::WidgetRenderContext;
pub(crate) use render_context::{HydrolysisTextContextMenuMode, HydrolysisWindowOrigin};
pub use state::HydroState;
pub(crate) use subview::HydroSubview;
pub(crate) use text_service::{
    ResolvedTextLayoutInput, TextMeasureService, resolve_text_layout_input,
    text_dimensions_from_layout,
};
pub(crate) use view_helpers::*;
pub(crate) use view_helpers::{
    anchor_point, circle_arc_path, effective_stretch_axis, estimate_layout_intrinsic,
    gesture_group_identity, normalize_layout_view, normalize_view_for_render, parley_alignment,
    parley_font_weight, passthrough_content, path_commands_to_path, resolved_color_to_peniko,
    resolved_color_to_rgba8, resolved_gradient_to_brush, resolved_shape_to_path, rgba8_to_peniko,
    transformed_rect,
};
