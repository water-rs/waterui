//! The hydrolysis renderer.
//!
//! This module owns [`HydrolysisRenderer`] — its construction and field
//! layout live here; behavior is split into focused submodules:
//!
//! - [`dispatch`]: type-erased view dispatch and `HydroNativeView` registration
//! - [`frame`]: frame lifecycle, layer stack, frame triggers, statistics
//! - [`FrameSignals`]: the shared frame trigger handle (lives in
//!   `waterui-backend-core`, shared with other self-drawn backends)
//! - [`retained`]: retained scene — replayable draws, `Dynamic` placements,
//!   reactive patching, scroll caches, window-frame capture/replay
//! - [`signals`]: signal watching and animated-value sampling
//! - [`effects`]: applied filters, view effects, embedded GPU surfaces
//! - [`views`] / [`metadata`]: raw view and metadata handlers
//! - [`bindings`]: hit-test/gesture/text-input/scroll bindings and queries
//! - [`input`] / [`lifecycle`] / [`navigation`] / [`accessibility`] /
//!   [`render`]: interaction, lifecycle, navigation, a11y, and measurement
//!   subsystems

mod accessibility;
mod bindings;
mod dispatch;
mod effects;
mod frame;
mod input;
mod interaction_layers;
mod lifecycle;
mod metadata;
mod navigation;
mod render;
mod retained;
mod signals;
#[cfg(test)]
mod tests;
mod views;

pub(crate) use dispatch::*;
pub(crate) use effects::*;
pub(crate) use frame::*;
pub(crate) use retained::*;
pub(crate) use views::*;
pub(crate) use waterui_backend_core::frame_signals::FrameSignals;

use accessibility::*;
use core::any::Any;
use core::f64::consts::TAU;
use core::num::NonZeroUsize;
use core::time::Duration;
pub(crate) use input::*;
pub(crate) use interaction_layers::*;
pub(crate) use lifecycle::lazy;
pub(crate) use lifecycle::*;
pub(crate) use navigation::*;
pub use render::HydrolysisRenderTarget;
pub(crate) use render::WidgetRenderContext;
pub(crate) use render::*;
pub(crate) use render::{
    anchor_point, circle_arc_path, estimate_layout_intrinsic, gesture_group_identity,
    normalize_layout_view, normalize_view_for_render, path_commands_to_path,
    resolved_color_to_peniko, resolved_gradient_to_brush, resolved_morph_shape_to_path,
    resolved_shape_to_path, transformed_rect,
};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, ActionData as AccessibilityActionData,
    ActionRequest as AccessibilityActionRequest, Node as AccessibilityNode,
    NodeId as AccessibilityNodeId, Rect as AccessibilityRect, Role as AccessibilityNodeRole,
    TextPosition as AccessibilityTextPosition, TextSelection as AccessibilityTextSelection,
    Toggled as AccessibilityToggled, Tree as AccessibilityTree, TreeId as AccessibilityTreeId,
    TreeUpdate as AccessibilityTreeUpdate,
};
use executor_core::spawn_local;
use nami::{Binding, Signal};
use rustc_hash::FxHashMap;
use waterkit_clipboard::Clipboard;
use waterui::ViewExt;
use waterui::accessibility::{
    AccessibilityChildren, AccessibilityHidden, AccessibilityLabel, AccessibilityRole,
    AccessibilityState, AccessibilityStateSignal,
};
use waterui::animation::Animation;
use waterui::background::{Background, MaterialBackground};
use waterui::border::Border;
use waterui::component::badge::BadgeConfig;
use waterui::component::focus::Focused;
use waterui::component::list::{ListConfig, ListItem};
use waterui::component::progress::{ProgressConfig, ProgressStyle};
use waterui::component::table::{TableColumn, TableConfig};
use waterui::cursor::{Cursor, CursorStyle};
use waterui::drag_drop::{Draggable, DropDestination};
use waterui::filter::Opacity;
use waterui::gesture::{Gesture, GestureObserver};
use waterui::interaction::Hittable;
use waterui::metadata::context_menu::{ContextMenu, ResolvedContextMenu};
use waterui::metadata::secure::{HighDynamicRange, Secure, StandardDynamicRange};
use waterui::navigation::tab::{TabPosition, Tabs};
use waterui::navigation::{
    CustomNavigationController, NavigationController, NavigationSplitLayout, NavigationStack,
    NavigationTransition, NavigationView,
};
use waterui::style::{Offset, Rotation, Scale, Shadow};
use waterui::theme;
use waterui::widget::Divider;
use waterui::window::{Window, WindowManager, WindowState, WindowStyle};
use waterui_canvas::Canvas;
use waterui_controls::button::{Button, ButtonConfig, ButtonStyle};
use waterui_controls::label::Label as SemanticLabel;
use waterui_controls::menu::{ResolvedCommand, ResolvedMenu, ResolvedMenuItem};
use waterui_controls::slider::SliderConfig;
use waterui_controls::stepper::StepperConfig;
use waterui_controls::text_field::{ResolvedTextFieldConfig, TextField};
use waterui_controls::toggle::ToggleConfig;
use waterui_core::dynamic::{Dynamic, DynamicInitialContent};
use waterui_core::event::{Event, HoverEvent, LifeCycle, LifeCycleHook, OnEvent};
use waterui_core::handler::{AnyViewBuilder, BoxedAction, SharedAction};
use waterui_core::layout::{
    HorizontalAlignment, Layout, PlacedSubview, Point as LayoutPoint, ProposalSize,
    Rect as LayoutRect, Size as LayoutSize, StretchAxis, SubView, VerticalAlignment,
    ViewDimensions,
};
use waterui_core::metadata::MetadataKey;
use waterui_core::views::Views;
use waterui_core::{
    AnyView, Environment, IgnorableMetadata, Metadata, Native,
    Retain, Str, View, impl_extractor,
};
use waterui_form::picker::PickerConfig;
use waterui_form::picker::color::ColorPickerConfig;
use waterui_form::picker::date::DatePickerConfig;
use waterui_form::secure::{Secure as FormSecure, SecureFieldConfig};
use waterui_graphics::color::{Color, ResolvedColor, Srgb};
use waterui_graphics::filter_view::{EffectContext, EffectInput, EffectOutput};
use waterui_graphics::gpu_surface::GestureState;
use waterui_graphics::view_effect::{
    ViewEffectContext, ViewEffectErased, ViewEffectInput, ViewEffectOutput,
};
use waterui_graphics::{
    AppliedFilter, GpuContext, GpuFrame, GpuSurface, GradientType, PointerState, RedrawHandle,
    ResolvedGradient, ResolvedGradientStop, SceneView, VelloScene2D,
};
use waterui_icon::SystemIcon;
use waterui_layout::container::{FixedContainer, LazyContainer};
use waterui_layout::safe_area::IgnoreSafeArea;
use waterui_layout::scroll::Axis as ScrollAxis;
use waterui_layout::scroll::ScrollView;
use waterui_layout::spacer::Spacer;
use waterui_map::MapConfig;
use waterui_shape::{ClipShape, PathCommand, ResolvedMorphShape, ResolvedShape, ShapeKind};
use waterui_text::font::FontWeight as TextFontWeight;
use waterui_text::styled::{Style as TextStyle, StyledStr};
use waterui_text::{Text, TextConfig};
use waterui_webview::WebView;

use crate::animation::{AnimatedScalarHandle, AnimationController, AnimationKey};
use crate::engine::{
    RadioIndicatorState, RadioSelectionMotion, TextCaretMotion, TextContextMenuMetrics,
    vello_backend::VelloDrawContext,
};
use crate::gesture::{GestureEngine, GestureTarget};
use crate::platform::{
    KeyCode, Modifiers, PointerButton, TextInputPurpose, TextInputState, TouchPhase,
};
use crate::scroll::{ScrollController, ScrollHandle};
use crate::time::Instant;
use crate::widgets::{inset_rect, widget_theme};

const OPACITY_ANIMATION_KEY: usize = 0x0100_0001;
const SCALE_X_ANIMATION_KEY: usize = 0x0100_0002;
const SCALE_Y_ANIMATION_KEY: usize = 0x0100_0003;
const ROTATION_ANIMATION_KEY: usize = 0x0100_0004;
const OFFSET_X_ANIMATION_KEY: usize = 0x0100_0005;
const OFFSET_Y_ANIMATION_KEY: usize = 0x0100_0006;
const MORPH_PROGRESS_ANIMATION_KEY: usize = 0x0100_0007;

#[cfg(feature = "accessibility")]
pub(crate) use accessibility::{
    AccessibilityActionTarget, accessibility_activation_point,
    collapsed_accessibility_text_selection, register_accessibility_text_run_node,
    slider_step_for_range,
};
pub(crate) use input::{
    TextInputModel, TextInputTargetRegistration, TextSelectionSlot, clamp_to_char_boundary,
    text_editing,
};

/// Core hydrolysis renderer state.
pub struct HydrolysisRenderer {
    dispatcher: HydroDispatcher,
    state: HydroState,
    vello_renderer: vello::Renderer,
    scene: vello::Scene,
    transient_scene: Option<vello::Scene>,
    compositor: Compositor,
    hit_test: HitTestState,
    gesture_engine: GestureEngine,
    gesture_group_ids: BTreeMap<usize, usize>,
    next_gesture_group_id: usize,
    text_editing: TextEditingState,
    popup_menu: PopupMenuState,
    render_depth: usize,
    window_bounds: vello::kurbo::Rect,
    /// Frame triggers shared with reactive closures; see [`FrameSignals`].
    signals: FrameSignals,
    lifecycle: LifecycleState,
    animation_controller: AnimationController,
    frame_instant: Instant,
    scroll_controller: ScrollController,
    scroll_content_caches: BTreeMap<usize, ScrollContentCache>,
    reuse_scroll_content_caches: bool,
    scroll_content_capture_depth: usize,
    scroll_content_viewport_dependent: bool,
    scroll_content_animation_dependent: bool,
    retained_window_frame: Option<RetainedWindowFrame>,
    dynamic_morph_capture_depth: u32,
    dynamic_morph_draws: Vec<DynamicMorphDraw>,
    dynamic_transform_capture_depth: u32,
    dynamic_transform_draws: Vec<DynamicTransformDraw>,
    dynamic_opacity_draws: Vec<DynamicOpacityDraw>,
    dynamic_node_draws: Vec<DynamicNodeDraw>,
    dynamic_scroll_draws: Vec<DynamicScrollDraw>,
    frame_clip_layers: u32,
    frame_max_clip_depth: u32,
    frame_applied_filter_count: u32,
    frame_applied_filter_capture: Duration,
    frame_applied_filter_effect: Duration,
    reuse_applied_filter_inputs: bool,
    active_applied_filters: Vec<ActiveAppliedFilter>,
    active_applied_filter_cursor: usize,
    /// Effect runtimes holding persistent GPU resources; see [`EffectRuntimeSlots`].
    effect_runtime_slots: EffectRuntimeSlots,
    pub(crate) lazy: LazyState,
    pub(crate) navigation: NavigationState,
    accessibility: AccessibilityBuilder,
}

const HIT_TEST_ALPHA_THRESHOLD: f32 = 0.01;

const TEXT_SELECTION_MULTI_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const TEXT_SELECTION_MULTI_CLICK_DISTANCE: f64 = 6.0;
const TEXT_CONTEXT_MENU_WINDOW_TITLE: &str = "";

impl HydrolysisRenderer {
    #[must_use]
    pub fn new(device: &wgpu::Device) -> Self {
        Self::new_with_options(
            device,
            vello::RendererOptions {
                use_cpu: false,
                antialiasing_support: vello::AaSupport::area_only(),
                num_init_threads: std::num::NonZeroUsize::new(1),
                pipeline_cache: None,
            },
        )
    }

    #[must_use]
    pub fn new_with_options(device: &wgpu::Device, options: vello::RendererOptions) -> Self {
        let mut dispatcher = HydroDispatcher::new();
        Self::register_core_handlers(&mut dispatcher);

        let vello_renderer =
            vello::Renderer::new(device, options).expect("failed to create hydrolysis renderer");
        let frame_instant = Instant::now();
        Self {
            dispatcher,
            state: HydroState::default(),
            vello_renderer,
            scene: vello::Scene::new(),
            transient_scene: None,
            compositor: Compositor::default(),
            hit_test: HitTestState::default(),
            gesture_engine: GestureEngine::default(),
            gesture_group_ids: BTreeMap::new(),
            next_gesture_group_id: 0,
            text_editing: TextEditingState::default(),
            popup_menu: PopupMenuState::default(),
            render_depth: 0,
            window_bounds: vello::kurbo::Rect::ZERO,
            signals: FrameSignals::new(frame_instant),
            lifecycle: LifecycleState::default(),
            animation_controller: AnimationController::default(),
            frame_instant,
            scroll_controller: ScrollController::default(),
            scroll_content_caches: BTreeMap::new(),
            reuse_scroll_content_caches: false,
            scroll_content_capture_depth: 0,
            scroll_content_viewport_dependent: false,
            scroll_content_animation_dependent: false,
            retained_window_frame: None,
            dynamic_morph_capture_depth: 0,
            dynamic_morph_draws: Vec::new(),
            dynamic_transform_capture_depth: 0,
            dynamic_transform_draws: Vec::new(),
            dynamic_opacity_draws: Vec::new(),
            dynamic_node_draws: Vec::new(),
            dynamic_scroll_draws: Vec::new(),
            frame_clip_layers: 0,
            frame_max_clip_depth: 0,
            frame_applied_filter_count: 0,
            frame_applied_filter_capture: Duration::ZERO,
            frame_applied_filter_effect: Duration::ZERO,
            reuse_applied_filter_inputs: false,
            active_applied_filters: Vec::new(),
            active_applied_filter_cursor: 0,
            effect_runtime_slots: EffectRuntimeSlots::default(),
            lazy: LazyState::default(),
            navigation: NavigationState::default(),
            accessibility: AccessibilityBuilder::default(),
        }
    }
}

pub use render::HydroState;
use render::HydroSubview;
pub use render::RenderContext;
pub(crate) use render::{HydrolysisTextContextMenuMode, HydrolysisWindowOrigin};
