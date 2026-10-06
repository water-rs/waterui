// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;

pub use crate::text::{
    FontFamilyResolution, ResolvedTextLayoutInput, SessionTextEngine, SessionTextLayout, TailMark,
    TextService, resolve_text_layout_input,
};

mod compositor;
mod measurement;
mod measurement_cache;
mod render_context;
mod state;
mod subview;
mod view_helpers;

pub use compositor::HydrolysisRenderTarget;
#[cfg(hydrolysis_macos_system_webview)]
pub(crate) use compositor::NativeViewLayer;
pub use compositor::{
    ActiveSceneLayer, CapturedLayers, CherenkovWindow, Compositor, EngineFrame, ExternalFrameLayer,
    FilteredLayer, FrameRenderTarget, GpuContentLayer, LayerShape, MaterialLayer, RenderLayer,
    SceneContentLayer,
};
pub use measurement::*;
pub use measurement_cache::{MeasurementCaches, MemoGate, NodeMeasureEntry};
pub use render_context::RenderContext;
pub use render_context::{HydrolysisTextContextMenuMode, HydrolysisWindowOrigin};
pub use render_context::{WidgetRenderContext, bounded_proposal};
pub use state::HydroState;
pub use subview::HydroSubview;
pub use view_helpers::*;
pub use view_helpers::{
    anchor_point, circle_arc_path, effective_stretch_axis, estimate_layout_intrinsic,
    gesture_group_identity, normalize_layout_view, normalize_view_for_render, passthrough_content,
    path_commands_to_path, resolved_shape_to_path, rgba8_to_peniko, transformed_rect,
    working_color_to_rgba8,
};
