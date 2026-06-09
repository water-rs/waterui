mod accessibility;
mod input;
mod lifecycle;
mod navigation;
mod render;
#[cfg(test)]
mod tests;

use accessibility::*;
use core::any::Any;
use core::f64::consts::TAU;
use core::num::NonZeroUsize;
use core::time::Duration;
pub(crate) use input::*;
pub(crate) use lifecycle::lazy;
pub(crate) use lifecycle::local_shared;
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
use nami::{Binding, Signal, with_local_binding_factory};
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
    AnyView, Environment, IgnorableMetadata, LocalStateScope, LocalStateStore, Metadata, Native,
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

type HydroRawHandlerFn =
    Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, &mut dyn Any, &Environment)>;
type HydroBoxedHandlerFn =
    Box<dyn Fn(&mut HydrolysisRenderer, RenderContext, AnyView, &Environment)>;

struct HydroHandlerEntry {
    raw: HydroRawHandlerFn,
    boxed: HydroBoxedHandlerFn,
}

#[derive(Clone, Default)]
struct HydroDispatcher {
    handlers: Rc<FxHashMap<core::any::TypeId, HydroHandlerEntry>>,
}

impl core::fmt::Debug for HydroDispatcher {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydroDispatcher")
            .field("handlers", &self.handlers.len())
            .finish()
    }
}

impl HydroDispatcher {
    fn new() -> Self {
        Self {
            handlers: Rc::new(FxHashMap::default()),
        }
    }

    fn register<V: View>(
        &mut self,
        handler: impl 'static + Clone + Fn(&mut HydrolysisRenderer, RenderContext, V, &Environment),
    ) {
        let handlers = Rc::get_mut(&mut self.handlers).unwrap_or_else(|| {
            panic!("hydrolysis dispatcher handlers cannot be mutated after cloning")
        });
        let h_raw = handler.clone();
        let h_boxed = handler;
        handlers.insert(
            core::any::TypeId::of::<V>(),
            HydroHandlerEntry {
                raw: Box::new(move |renderer, ctx, slot: &mut dyn Any, env| {
                    let view = slot
                        .downcast_mut::<Option<V>>()
                        .expect("hydrolysis raw dispatch type mismatch")
                        .take()
                        .expect("hydrolysis raw dispatch view already taken");
                    h_raw(renderer, ctx, view, env);
                }),
                boxed: Box::new(move |renderer, ctx, view: AnyView, env| {
                    let view = *view
                        .downcast::<V>()
                        .expect("hydrolysis boxed dispatch type mismatch");
                    h_boxed(renderer, ctx, view, env);
                }),
            },
        );
    }

    fn register_renderer<V: View>(
        &mut self,
        handler: impl 'static + Clone + Fn(&mut HydrolysisRenderer, RenderContext, V, &Environment),
    ) {
        self.register(handler);
    }

    fn dispatch<V: View>(
        &self,
        renderer: &mut HydrolysisRenderer,
        view: V,
        env: &Environment,
        ctx: RenderContext,
    ) {
        let type_id = core::any::TypeId::of::<V>();

        if type_id == core::any::TypeId::of::<AnyView>() {
            let mut slot = Some(view);
            let any_view = (&mut slot as &mut dyn Any)
                .downcast_mut::<Option<AnyView>>()
                .expect("hydrolysis AnyView downcast should succeed")
                .take()
                .expect("hydrolysis AnyView option should contain a value");
            self.dispatch_boxed(renderer, any_view, env, ctx);
            return;
        }

        if let Some(entry) = self.handlers.get(&type_id) {
            let mut slot = Some(view);
            (entry.raw)(renderer, ctx, &mut slot as &mut dyn Any, env);
            return;
        }

        let body_env = local_state_body_env(env);
        let body_content_env = local_state_body_content_env(env);
        let body = renderer
            .lifecycle
            .with_local_state_env(&body_env, move |render_env| {
                AnyView::new(view.body(render_env))
            });
        self.dispatch_boxed(renderer, body, &body_content_env, ctx);
    }

    fn dispatch_boxed(
        &self,
        renderer: &mut HydrolysisRenderer,
        view: AnyView,
        env: &Environment,
        ctx: RenderContext,
    ) {
        let type_id = view.type_id();
        if let Some(entry) = self.handlers.get(&type_id) {
            (entry.boxed)(renderer, ctx, view, env);
            return;
        }

        let body_env = local_state_body_env(env);
        let body_content_env = local_state_body_content_env(env);
        let body = renderer
            .lifecycle
            .with_local_state_env(&body_env, move |render_env| {
                AnyView::new(view.body(render_env))
            });
        self.dispatch_boxed(renderer, body, &body_content_env, ctx);
    }
}

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
    redraw_requested: Rc<Cell<bool>>,
    pub(crate) rebuild_requested: Rc<Cell<bool>>,
    /// Set when a `Dynamic` node's content changed and can be patched in isolation
    /// (fine-grained reactive update) rather than forcing a full structural rebuild.
    patch_requested: Rc<Cell<bool>>,
    /// Identities of `Dynamic` nodes whose content changed since the last frame and
    /// must be re-dispatched in isolation on the next reactive-patch frame.
    dirty_dynamic_nodes: Rc<RefCell<BTreeSet<usize>>>,
    next_frame_rebuild_requested: Cell<bool>,
    rebuild_generation: Rc<Cell<u64>>,
    rebuild_in_progress: Rc<Cell<bool>>,
    lifecycle: LifecycleState,
    animation_controller: AnimationController,
    frame_instant: Instant,
    frame_clock: Rc<Cell<Instant>>,
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
    pub(crate) lazy: LazyState,
    pub(crate) navigation: NavigationState,
    accessibility: AccessibilityBuilder,
}

/// A retained snapshot of the entire window content captured during a structural
/// rebuild. Parametric frames (animation ticks, scroll offset changes) re-render by
/// replaying this subtree — re-sampling animated transforms/morphs and applying current
/// scroll offsets at the new frame instant — instead of re-walking and re-measuring the
/// WaterUI view tree.
///
/// The subtree is captured in real (already-DPI-scaled) coordinates, so it replays
/// under an identity context. Scrolling is subsumed into it via [`DynamicScrollDraw`].
struct RetainedWindowFrame {
    subtree: DynamicSubtree,
    /// The static root transform (device scale factor) used for the background fill.
    transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
    active_layers: Vec<ActiveSceneLayer>,
    content_morphs: Vec<DynamicMorphDraw>,
    /// Whether this frame can be re-rendered by pure replay. False when the content
    /// baked an animated non-transform value (e.g. opacity), bound a GPU surface, or
    /// used an applied filter — those cannot be reproduced without a real dispatch, so
    /// such frames fall back to a structural rebuild.
    drivable: bool,
}

struct ScrollContentCache {
    lazy_viewport: vello::kurbo::Rect,
    viewport_dependent: bool,
    animation_dependent: bool,
    subtree: DynamicSubtree,
    active_filters: Vec<ActiveAppliedFilter>,
    dynamic_morphs: Vec<DynamicMorphDraw>,
}

#[derive(Clone)]
pub(crate) struct DynamicMorphDraw {
    shape: ResolvedMorphShape,
    bounds: vello::kurbo::Rect,
    transform: vello::kurbo::Affine,
    started_at: Instant,
}

struct DynamicTransformScalar {
    value: f32,
    handle: Option<AnimatedScalarHandle>,
}

struct DynamicScaleTransform {
    x: DynamicTransformScalar,
    y: DynamicTransformScalar,
    center: vello::kurbo::Point,
}

struct DynamicRotationTransform {
    angle: DynamicTransformScalar,
    center: vello::kurbo::Point,
}

struct DynamicOffsetTransform {
    x: DynamicTransformScalar,
    y: DynamicTransformScalar,
}

struct DynamicTransformComponents {
    scale: Option<DynamicScaleTransform>,
    rotation: Option<DynamicRotationTransform>,
    offset: Option<DynamicOffsetTransform>,
}

pub(crate) struct DynamicTransformDraw {
    transform: DynamicTransformComponents,
    base_transform: vello::kurbo::Affine,
    base_hit_transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
    subtree: DynamicSubtree,
}

impl DynamicTransformScalar {
    fn sample(&self, now: Instant) -> f32 {
        self.handle
            .as_ref()
            .map_or(self.value, |handle| handle.sample(now))
    }

    fn collect_active_key(&self, keys: &mut BTreeSet<AnimationKey>) {
        if let Some(handle) = &self.handle
            && handle.is_active()
        {
            keys.insert(handle.key());
        }
    }
}

impl DynamicTransformComponents {
    fn scale(
        x: DynamicTransformScalar,
        y: DynamicTransformScalar,
        center: vello::kurbo::Point,
    ) -> Self {
        Self {
            scale: Some(DynamicScaleTransform { x, y, center }),
            rotation: None,
            offset: None,
        }
    }

    fn rotation(angle: DynamicTransformScalar, center: vello::kurbo::Point) -> Self {
        Self {
            scale: None,
            rotation: Some(DynamicRotationTransform { angle, center }),
            offset: None,
        }
    }

    fn offset(x: DynamicTransformScalar, y: DynamicTransformScalar) -> Self {
        Self {
            scale: None,
            rotation: None,
            offset: Some(DynamicOffsetTransform { x, y }),
        }
    }

    fn affine(&self, now: Instant) -> vello::kurbo::Affine {
        let active_components = usize::from(self.scale.is_some())
            + usize::from(self.rotation.is_some())
            + usize::from(self.offset.is_some());
        assert!(
            active_components == 1,
            "hydrolysis dynamic transform draw must contain exactly one transform component"
        );
        if let Some(scale) = &self.scale {
            return vello::kurbo::Affine::translate((scale.center.x, scale.center.y))
                * vello::kurbo::Affine::scale_non_uniform(
                    f64::from(scale.x.sample(now)),
                    f64::from(scale.y.sample(now)),
                )
                * vello::kurbo::Affine::translate((-scale.center.x, -scale.center.y));
        }
        if let Some(rotation) = &self.rotation {
            let radians = f64::from(rotation.angle.sample(now)).to_radians();
            return vello::kurbo::Affine::translate((rotation.center.x, rotation.center.y))
                * vello::kurbo::Affine::rotate(radians)
                * vello::kurbo::Affine::translate((-rotation.center.x, -rotation.center.y));
        }
        let offset = self
            .offset
            .as_ref()
            .expect("hydrolysis dynamic transform draw missing offset component");
        vello::kurbo::Affine::translate((
            f64::from(offset.x.sample(now)),
            f64::from(offset.y.sample(now)),
        ))
    }

    fn collect_active_scalar_keys(&self, keys: &mut BTreeSet<AnimationKey>) {
        if let Some(scale) = &self.scale {
            scale.x.collect_active_key(keys);
            scale.y.collect_active_key(keys);
        }
        if let Some(rotation) = &self.rotation {
            rotation.angle.collect_active_key(keys);
        }
        if let Some(offset) = &self.offset {
            offset.x.collect_active_key(keys);
            offset.y.collect_active_key(keys);
        }
    }
}

/// A replayable opacity layer captured during a dynamic subtree capture. Its alpha is
/// re-sampled at replay time so animated opacity re-renders without re-dispatching the
/// wrapped content, the opacity counterpart of [`DynamicTransformDraw`].
pub(crate) struct DynamicOpacityDraw {
    alpha: DynamicTransformScalar,
    base_transform: vello::kurbo::Affine,
    base_hit_transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
    subtree: DynamicSubtree,
}

/// A placement of a `Dynamic` node within a captured subtree. The node's content is
/// not baked into the parent scene; instead it is composited from the node's own
/// retained `cached_subtree` (keyed by `identity` in `lifecycle.dynamic_nodes`) at
/// replay time. This is what makes fine-grained reactive patching possible: when one
/// `Dynamic` node's content changes, only that node is re-dispatched and the window is
/// re-composited from the unchanged placements of every other node.
pub(crate) struct DynamicNodeDraw {
    identity: usize,
    base_transform: vello::kurbo::Affine,
    base_hit_transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
}

/// A placement of a scroll view within a captured subtree. Its content is captured once
/// (offset-independently) into `scroll_content_caches[cache_key]`; the current scroll
/// offset is applied at replay, so scrolling re-composites the window frame without
/// re-dispatching the view tree. This subsumes the former standalone retained-scroll
/// fast-path into the single window-frame retention path. Lazy (viewport-dependent)
/// content that scrolls beyond its captured window escalates to a structural rebuild.
pub(crate) struct DynamicScrollDraw {
    handle: crate::scroll::ScrollHandle,
    cache_key: usize,
    axis: ScrollAxis,
    viewport: vello::kurbo::Rect,
    content_width: f64,
    content_height: f64,
    base_transform: vello::kurbo::Affine,
    base_hit_transform: vello::kurbo::Affine,
    content_morphs: Vec<DynamicMorphDraw>,
    needs_viewport_clip: bool,
    env: Environment,
}

impl DynamicScrollDraw {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        handle: crate::scroll::ScrollHandle,
        cache_key: usize,
        axis: ScrollAxis,
        viewport: vello::kurbo::Rect,
        content_width: f64,
        content_height: f64,
        base_transform: vello::kurbo::Affine,
        base_hit_transform: vello::kurbo::Affine,
        content_morphs: Vec<DynamicMorphDraw>,
        needs_viewport_clip: bool,
        env: Environment,
    ) -> Self {
        Self {
            handle,
            cache_key,
            axis,
            viewport,
            content_width,
            content_height,
            base_transform,
            base_hit_transform,
            content_morphs,
            needs_viewport_clip,
            env,
        }
    }
}

pub(crate) struct ScrollContentRender {
    pub(crate) dynamic_morphs: Vec<DynamicMorphDraw>,
}

struct AppliedFilterRuntime {
    filter: AppliedFilter,
    setup_complete: bool,
    input_texture: Option<AppliedFilterInputTexture>,
    output_texture: Option<AppliedFilterOutputTexture>,
    output_image: Option<vello::peniko::ImageData>,
}

impl AppliedFilterRuntime {
    fn new(filter: AppliedFilter) -> Self {
        Self {
            filter,
            setup_complete: false,
            input_texture: None,
            output_texture: None,
            output_image: None,
        }
    }

    fn replace_filter(&mut self, filter: AppliedFilter) {
        self.filter = filter;
        self.setup_complete = false;
        self.input_texture = None;
        self.output_texture = None;
        self.output_image = None;
    }

    fn input_texture(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> (&wgpu::Texture, &wgpu::TextureView) {
        if self
            .input_texture
            .as_ref()
            .is_none_or(|texture| texture.width != width || texture.height != height)
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("hydrolysis_applied_filter_input"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.input_texture = Some(AppliedFilterInputTexture {
                width,
                height,
                texture,
                view,
            });
        }

        let Some(texture) = self.input_texture.as_ref() else {
            panic!("hydrolysis AppliedFilter input texture cache missing after allocation");
        };
        (&texture.texture, &texture.view)
    }

    fn has_input_texture(&self, width: u32, height: u32) -> bool {
        self.input_texture
            .as_ref()
            .is_some_and(|texture| texture.width == width && texture.height == height)
    }

    fn output_texture(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> (&wgpu::Texture, &wgpu::TextureView) {
        if self
            .output_texture
            .as_ref()
            .is_none_or(|texture| texture.width != width || texture.height != height)
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("hydrolysis_applied_filter_output"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.output_texture = Some(AppliedFilterOutputTexture {
                width,
                height,
                texture,
                view,
            });
        }

        let Some(texture) = self.output_texture.as_ref() else {
            panic!("hydrolysis AppliedFilter output texture cache missing after allocation");
        };
        (&texture.texture, &texture.view)
    }

    fn needs_redraw_refresh(&mut self) -> bool {
        self.filter.sync_targets();
        self.filter.redraw_hint()
    }

    fn render_output(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vello_renderer: &mut vello::Renderer,
        width: u32,
        height: u32,
    ) -> (vello::peniko::ImageData, bool) {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("hydrolysis applied filter encoder"),
        });
        let output = self.encode_output(device, queue, vello_renderer, width, height, &mut encoder);
        queue.submit([encoder.finish()]);
        output
    }

    fn encode_output(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vello_renderer: &mut vello::Renderer,
        width: u32,
        height: u32,
        encoder: &mut wgpu::CommandEncoder,
    ) -> (vello::peniko::ImageData, bool) {
        let filter_context = EffectContext {
            device,
            queue,
            input_format: wgpu::TextureFormat::Rgba8Unorm,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            pipeline_cache: None,
        };
        if !self.setup_complete {
            match pollster::block_on(self.filter.setup(&filter_context)) {
                Ok(()) => {}
                Err(err) => {
                    panic!("hydrolysis filter setup failed: {err}");
                }
            }
            self.setup_complete = true;
        }
        self.filter.sync_targets();
        let (output_width, output_height) = self.filter.output_size(width, height);
        let (input_texture, input_view) = {
            let Some(input_texture) = self.input_texture.as_ref() else {
                panic!("hydrolysis AppliedFilter input texture missing before render");
            };
            (input_texture.texture.clone(), input_texture.view.clone())
        };
        let (output_texture, output_view) = {
            let (texture, view) = self.output_texture(device, output_width, output_height);
            (texture.clone(), view.clone())
        };
        let input = EffectInput {
            device,
            queue,
            texture: &input_texture,
            view: input_view,
            format: wgpu::TextureFormat::Rgba8Unorm,
            width,
            height,
        };
        let output = EffectOutput {
            device,
            queue,
            texture: &output_texture,
            view: output_view,
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: output_width,
            height: output_height,
        };
        let needs_redraw = match self.filter.encode_render(&input, &output, encoder) {
            Ok(needs_redraw) => needs_redraw || self.filter.redraw_hint(),
            Err(err) => {
                panic!("hydrolysis filter render failed: {err}");
            }
        };

        let image = if let Some(image) = self
            .output_image
            .as_ref()
            .filter(|image| image.width == output_width && image.height == output_height)
        {
            let texture_base = wgpu::TexelCopyTextureInfoBase {
                texture: output_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            };
            let _ = vello_renderer.override_image(image, Some(texture_base));
            image.clone()
        } else {
            let image = vello_renderer.register_texture(output_texture);
            self.output_image = Some(image.clone());
            image
        };
        (image, needs_redraw)
    }
}

struct AppliedFilterInputTexture {
    width: u32,
    height: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct AppliedFilterOutputTexture {
    width: u32,
    height: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

#[derive(Clone)]
struct ActiveAppliedFilter {
    runtime: Rc<RefCell<AppliedFilterRuntime>>,
    width: u32,
    height: u32,
}

struct ViewEffectRuntime {
    effect: ViewEffectErased,
    setup_complete: bool,
}

impl ViewEffectRuntime {
    fn new(effect: ViewEffectErased) -> Self {
        Self {
            effect,
            setup_complete: false,
        }
    }

    fn replace_effect(&mut self, effect: ViewEffectErased) {
        self.effect = effect;
        self.setup_complete = false;
    }
}

struct SceneViewRuntime {
    content: Box<dyn waterui_graphics::SceneContent>,
}

impl SceneViewRuntime {
    fn new(content: Box<dyn waterui_graphics::SceneContent>) -> Self {
        Self { content }
    }

    fn replace_content(&mut self, content: Box<dyn waterui_graphics::SceneContent>) {
        self.content = content;
    }
}

pub(crate) trait HydroNativeView: View + Sized + 'static {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment);
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize;
    fn accessibility_is_render_driven() -> bool {
        false
    }
    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        _proposal: ProposalSize,
    ) -> ViewDimensions {
        ViewDimensions::new(Self::intrinsic(state, view, env))
    }
    fn accessibility(
        _renderer: &mut HydrolysisRenderer,
        _ctx: RenderContext,
        _view: &Self,
        _env: &Environment,
    ) {
    }
}

#[cfg(feature = "accessibility")]
fn register_native_view<V: HydroNativeView>(dispatcher: &mut HydroDispatcher) {
    dispatcher.register::<V>(|renderer, ctx, view, env| {
        let accessibility_is_render_driven = V::accessibility_is_render_driven();
        let hidden_from_accessibility = env
            .get::<AccessibilityHidden>()
            .is_some_and(AccessibilityHidden::is_hidden);
        if hidden_from_accessibility {
            renderer.push_accessibility_suppression();
            if !accessibility_is_render_driven {
                V::accessibility(renderer, ctx, &view, env);
            }
            let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
            V::render(&mut widget_ctx, view, env);
            renderer.pop_accessibility_suppression();
            return;
        }
        if !accessibility_is_render_driven {
            V::accessibility(renderer, ctx, &view, env);
        }
        let suppress_descendants = env
            .get::<AccessibilityChildren>()
            .is_some_and(AccessibilityChildren::excludes_descendants);
        if suppress_descendants && !accessibility_is_render_driven {
            renderer.push_accessibility_suppression();
            let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
            V::render(&mut widget_ctx, view, env);
            renderer.pop_accessibility_suppression();
            return;
        }
        let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
        V::render(&mut widget_ctx, view, env);
    });
}

#[cfg(not(feature = "accessibility"))]
fn register_native_view<V: HydroNativeView>(dispatcher: &mut HydroDispatcher) {
    dispatcher.register::<V>(|renderer, ctx, view, env| {
        V::accessibility(renderer, ctx, &view, env);
        let mut widget_ctx = WidgetRenderContext::new(renderer, ctx);
        V::render(&mut widget_ctx, view, env);
    });
}

fn dimensions_for_native<V: HydroNativeView>(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> Option<ViewDimensions> {
    view.downcast_ref::<V>()
        .map(|native| V::dimensions(state, native, env, proposal))
}

const HIT_TEST_ALPHA_THRESHOLD: f32 = 0.01;

const TEXT_SELECTION_MULTI_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const TEXT_SELECTION_MULTI_CLICK_DISTANCE: f64 = 6.0;
const TEXT_CONTEXT_MENU_WINDOW_TITLE: &str = "";

pub(crate) fn slider_value_epsilon(span: f64, track_width: f64) -> f64 {
    (span / track_width).abs().max(f64::EPSILON)
}

pub(super) fn call_action_discarding_result<T: 'static>(
    action: &SharedAction<T>,
    env: &Environment,
) {
    let _ = action.call(env);
}

pub(crate) fn popup_menu_nodes(items: &[ResolvedMenuItem]) -> Vec<PopupMenuNode> {
    items.iter().cloned().map(popup_menu_node).collect()
}

fn popup_menu_node(item: ResolvedMenuItem) -> PopupMenuNode {
    match item {
        ResolvedMenuItem::Command(command) => {
            let mut styled = command.label.content.get();
            if command.selected.get() {
                styled = StyledStr::plain("✓ ") + styled;
            }
            let plain_label = styled.to_plain().to_string();
            let label = command.semantic_label.text(Text::new(styled));
            PopupMenuNode::Command {
                label,
                plain_label,
                action: command.action,
                disabled: command.disabled.get(),
            }
        }
        ResolvedMenuItem::Divider => PopupMenuNode::Divider,
        ResolvedMenuItem::Menu(menu) => {
            let styled = menu.label.content.get() + StyledStr::plain(" ›");
            let plain_label = styled.to_plain().to_string();
            let label = menu.semantic_label.text(Text::new(styled));
            PopupMenuNode::Menu {
                label,
                plain_label,
                items: popup_menu_nodes(&menu.items.get()),
            }
        }
    }
}

macro_rules! hydro_native_view_types {
    ($macro:ident) => {
        $macro!(Native<()>);
        $macro!(Native<Spacer>);
        $macro!(Native<TextConfig>);
        $macro!(Native<FixedContainer>);
        $macro!(Native<LazyContainer>);
        $macro!(Native<ScrollView>);
        $macro!(Native<NavigationView>);
        $macro!(Native<NavigationSplitLayout>);
        $macro!(Native<NavigationStack<(), ()>>);
        $macro!(Native<Tabs>);
        $macro!(Native<BadgeConfig>);
        $macro!(Native<ListConfig>);
        $macro!(Native<TableConfig>);
        $macro!(Native<ButtonConfig>);
        $macro!(Native<ResolvedMenu>);
        $macro!(Native<ToggleConfig>);
        $macro!(Native<SliderConfig>);
        $macro!(Native<StepperConfig>);
        $macro!(Native<ProgressConfig>);
        $macro!(Native<ColorPickerConfig>);
        $macro!(Native<DatePickerConfig>);
        $macro!(Native<ResolvedTextFieldConfig>);
        $macro!(Native<SecureFieldConfig>);
        $macro!(Native<PickerConfig>);
        $macro!(Native<Dynamic>);
        $macro!(Native<SystemIcon>);
        $macro!(Native<GpuSurface>);
        $macro!(Native<SceneView>);
        $macro!(Native<ViewEffectErased>);
        $macro!(Native<ResolvedColor>);
        $macro!(Native<ResolvedGradient>);
        $macro!(Native<ResolvedShape>);
        $macro!(Native<ResolvedMorphShape>);
        $macro!(Native<MapConfig>);
        $macro!(WebView);
    };
}

pub(crate) fn is_hydro_native_view(view: &AnyView) -> bool {
    macro_rules! check_native_view {
        ($ty:ty) => {
            if view.downcast_ref::<$ty>().is_some() {
                return true;
            }
        };
    }
    hydro_native_view_types!(check_native_view);
    false
}

fn dimensions_for_known_native_views(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> Option<ViewDimensions> {
    macro_rules! try_native_dimensions {
        ($ty:ty) => {
            if let Some(dimensions) = dimensions_for_native::<$ty>(view, proposal, state, env) {
                return Some(dimensions);
            }
        };
    }
    hydro_native_view_types!(try_native_dimensions);
    None
}

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
            redraw_requested: Rc::new(Cell::new(false)),
            rebuild_requested: Rc::new(Cell::new(false)),
            patch_requested: Rc::new(Cell::new(false)),
            dirty_dynamic_nodes: Rc::new(RefCell::new(BTreeSet::new())),
            next_frame_rebuild_requested: Cell::new(false),
            rebuild_generation: Rc::new(Cell::new(0)),
            rebuild_in_progress: Rc::new(Cell::new(false)),
            lifecycle: LifecycleState::default(),
            animation_controller: AnimationController::default(),
            frame_instant,
            frame_clock: Rc::new(Cell::new(frame_instant)),
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
            lazy: LazyState::default(),
            navigation: NavigationState::default(),
            accessibility: AccessibilityBuilder::default(),
        }
    }

    fn register_core_handlers(dispatcher: &mut HydroDispatcher) {
        dispatcher.register_renderer::<Str>(Self::render_str);
        dispatcher
            .register_renderer::<Divider>(crate::widgets::divider::render_divider_with_renderer);
        macro_rules! register_native {
            ($ty:ty) => {
                register_native_view::<$ty>(dispatcher);
            };
        }
        hydro_native_view_types!(register_native);

        dispatcher.register_renderer::<Metadata<Environment>>(Self::render_environment_metadata);
        dispatcher.register_renderer::<Metadata<Retain>>(Self::render_retain_metadata);
        dispatcher.register_renderer::<Metadata<Opacity>>(Self::render_opacity_metadata);
        dispatcher
            .register_renderer::<Metadata<AppliedFilter>>(Self::render_applied_filter_metadata);
        dispatcher.register_renderer::<Metadata<Scale>>(Self::render_scale_metadata);
        dispatcher.register_renderer::<Metadata<Rotation>>(Self::render_rotation_metadata);
        dispatcher.register_renderer::<Metadata<Offset>>(Self::render_offset_metadata);
        dispatcher.register_renderer::<Metadata<ClipShape>>(Self::render_clip_shape_metadata);
        dispatcher.register_renderer::<Metadata<Border>>(Self::render_border_metadata);
        dispatcher.register_renderer::<Metadata<Shadow>>(Self::render_shadow_metadata);
        dispatcher.register_renderer::<Metadata<Focused>>(Self::render_focused_metadata);
        dispatcher.register_renderer::<Metadata<Hittable>>(Self::render_hittable_metadata);
        dispatcher.register_renderer::<Metadata<Cursor>>(Self::render_cursor_metadata);
        dispatcher
            .register_renderer::<Metadata<GestureObserver>>(Self::render_gesture_observer_metadata);
        dispatcher
            .register_renderer::<Metadata<LifeCycleHook>>(Self::render_lifecycle_hook_metadata);
        dispatcher.register_renderer::<Metadata<OnEvent>>(Self::render_on_event_metadata);

        Self::register_passthrough_metadata::<Secure>(dispatcher);
        Self::register_passthrough_metadata::<StandardDynamicRange>(dispatcher);
        Self::register_passthrough_metadata::<HighDynamicRange>(dispatcher);
        Self::register_passthrough_metadata::<IgnoreSafeArea>(dispatcher);
        Self::register_passthrough_metadata::<ContextMenu>(dispatcher);
        dispatcher
            .register_renderer::<Metadata<ResolvedContextMenu>>(Self::render_context_menu_metadata);
        dispatcher.register_renderer::<Metadata<Draggable>>(Self::render_draggable_metadata);
        dispatcher
            .register_renderer::<Metadata<DropDestination>>(Self::render_drop_destination_metadata);
        Self::register_passthrough_metadata::<Background>(dispatcher);

        Self::register_passthrough_ignorable_metadata::<MaterialBackground>(dispatcher);
        dispatcher.register_renderer::<IgnorableMetadata<AccessibilityLabel>>(
            Self::render_accessibility_label_metadata,
        );
        dispatcher.register_renderer::<IgnorableMetadata<AccessibilityRole>>(
            Self::render_accessibility_role_metadata,
        );
        dispatcher.register_renderer::<IgnorableMetadata<AccessibilityHidden>>(
            Self::render_accessibility_hidden_metadata,
        );
        dispatcher.register_renderer::<IgnorableMetadata<AccessibilityChildren>>(
            Self::render_accessibility_children_metadata,
        );
        dispatcher.register_renderer::<IgnorableMetadata<AccessibilityState>>(
            Self::render_accessibility_state_metadata,
        );
        dispatcher.register_renderer::<IgnorableMetadata<AccessibilityStateSignal>>(
            Self::render_accessibility_state_signal_metadata,
        );
    }

    fn register_passthrough_metadata<T: MetadataKey>(dispatcher: &mut HydroDispatcher) {
        dispatcher.register_renderer::<Metadata<T>>(Self::render_passthrough_metadata::<T>);
    }

    fn register_passthrough_ignorable_metadata<T: MetadataKey>(dispatcher: &mut HydroDispatcher) {
        dispatcher.register_renderer::<IgnorableMetadata<T>>(
            Self::render_passthrough_ignorable_metadata::<T>,
        );
    }

    fn target_hit_priority(depth: usize, order: usize, index: usize) -> (usize, usize, usize) {
        (order, depth, index)
    }

    fn topmost_text_input_index_at_point(&self, point: vello::kurbo::Point) -> Option<usize> {
        self.text_editing
            .text_input_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.bounds.contains(point))
            .max_by(|(left_index, left), (right_index, right)| {
                Self::target_hit_priority(left.depth, left.order, *left_index).cmp(
                    &Self::target_hit_priority(right.depth, right.order, *right_index),
                )
            })
            .map(|(index, _)| index)
    }

    pub(crate) fn set_window_bounds(&mut self, bounds: vello::kurbo::Rect) {
        self.window_bounds = bounds;
    }

    #[cfg(feature = "accessibility")]
    fn focused_text_input_accessibility_node(&self) -> Option<AccessibilityNodeId> {
        let index = self.text_editing.focused_text_input.get()?;
        let target = self.text_editing.text_input_targets.as_slice().get(index)?;
        target.accessibility_node_id
    }

    #[cfg(feature = "accessibility")]
    fn focus_text_input_for_accessibility_node(&mut self, node_id: AccessibilityNodeId) -> bool {
        let focused = self
            .text_editing
            .text_input_targets
            .iter()
            .position(|target| target.accessibility_node_id == Some(node_id))
            .unwrap_or_else(|| {
                panic!(
                    "hydrolysis accessibility focus target node {:?} has no matching text input target",
                    node_id
                )
            });
        self.set_focused_text_input(Some(focused))
    }

    fn push_render_depth(&mut self) {
        self.render_depth = self
            .render_depth
            .checked_add(1)
            .expect("hydrolysis render depth overflow");
    }

    fn next_hit_test_order(&mut self) -> usize {
        self.hit_test.next_hit_test_order()
    }

    fn pop_render_depth(&mut self) {
        self.render_depth = self
            .render_depth
            .checked_sub(1)
            .expect("hydrolysis render depth underflow");
    }

    fn dispatch_with_render_depth<V: View>(
        &mut self,
        view: V,
        env: &Environment,
        ctx: RenderContext,
    ) {
        assert!(
            self.render_depth < 256,
            "hydrolysis render dispatch exceeded recursion budget for {}",
            core::any::type_name::<V>()
        );
        self.push_render_depth();
        let dispatcher = self.dispatcher.clone();
        dispatcher.dispatch(self, view, env, ctx);
        self.pop_render_depth();
    }

    fn dispatch_boxed_with_render_depth(
        &mut self,
        view: AnyView,
        env: &Environment,
        ctx: RenderContext,
    ) {
        assert!(
            self.render_depth < 256,
            "hydrolysis render dispatch exceeded recursion budget for {}",
            view.name()
        );
        self.push_render_depth();
        let dispatcher = self.dispatcher.clone();
        dispatcher.dispatch_boxed(self, view, env, ctx);
        self.pop_render_depth();
    }

    fn replay_target_depth(&self, subtree_depth_base: usize, target_depth: usize) -> usize {
        let relative_depth = target_depth
            .checked_sub(subtree_depth_base)
            .expect("hydrolysis dynamic subtree target depth underflow");
        self.render_depth
            .checked_add(relative_depth)
            .expect("hydrolysis dynamic subtree target depth overflow")
    }

    pub(crate) fn dispatch_any(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
    ) {
        renderer.dispatch_boxed_with_render_depth(content, env, ctx);
    }

    pub(crate) fn dispatch_any_without_accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
    ) {
        #[cfg(feature = "accessibility")]
        {
            renderer.push_accessibility_suppression();
            renderer.dispatch_boxed_with_render_depth(content, env, ctx);
            renderer.pop_accessibility_suppression();
        }
        #[cfg(not(feature = "accessibility"))]
        Self::dispatch_any(renderer, ctx, env, content);
    }

    pub(crate) fn dispatch_in_rect(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
        rect: vello::kurbo::Rect,
    ) {
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let child_transform = vello::kurbo::Affine::translate((rect.x0, rect.y0));
        let child_bounds = vello::kurbo::Rect::new(0.0, 0.0, rect.width(), rect.height());
        Self::dispatch_any(
            renderer,
            ctx.child(child_transform, child_bounds),
            env,
            content,
        );
    }

    pub(crate) fn dispatch_in_rect_without_accessibility(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
        rect: vello::kurbo::Rect,
    ) {
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let child_transform = vello::kurbo::Affine::translate((rect.x0, rect.y0));
        let child_bounds = vello::kurbo::Rect::new(0.0, 0.0, rect.width(), rect.height());
        Self::dispatch_any_without_accessibility(
            renderer,
            ctx.child(child_transform, child_bounds),
            env,
            content,
        );
    }

    pub(crate) fn render_subtree_scene(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
    ) -> vello::Scene {
        let mut subtree_scene = vello::Scene::new();
        let local_ctx = ctx.with_identity_transforms(vello::kurbo::Rect::new(
            0.0,
            0.0,
            ctx.bounds.width(),
            ctx.bounds.height(),
        ));
        core::mem::swap(&mut renderer.scene, &mut subtree_scene);
        renderer.dispatch_boxed_with_render_depth(content, env, local_ctx);
        core::mem::swap(&mut renderer.scene, &mut subtree_scene);
        subtree_scene
    }

    fn watch_signal<S>(&mut self, signal: &S)
    where
        S: Signal + Clone + 'static,
    {
        let rebuild_requested = Rc::clone(&self.rebuild_requested);
        let guard = signal.watch(move |_| rebuild_requested.set(true));
        self.lifecycle.current_frame_retain.push(Retain::new(guard));
    }

    pub(crate) fn read_signal<S>(&mut self, signal: &S) -> S::Output
    where
        S: Signal + Clone + 'static,
    {
        self.watch_signal(signal);
        signal.get()
    }

    pub(crate) fn read_resolved_text_styled(
        &mut self,
        text: &Text,
        env: &Environment,
    ) -> StyledStr {
        let resolved = text.resolve(env);
        self.read_signal(&resolved.content)
    }

    pub(crate) fn push_pending_scroll_handle(&mut self, handle: ScrollHandle) {
        self.lazy.push_pending_scroll_handle(handle);
    }

    pub(crate) fn bind_scroll_handle(
        &mut self,
        axis: ScrollAxis,
        viewport_width: f64,
        viewport_height: f64,
        content_width: f64,
        content_height: f64,
    ) -> ScrollHandle {
        let handle = self.scroll_controller.bind(
            axis,
            viewport_width,
            viewport_height,
            content_width,
            content_height,
        );
        self.push_pending_scroll_handle(handle.clone());
        handle
    }

    pub(crate) fn bind_render_scroll_handle(
        &mut self,
        axis: ScrollAxis,
        viewport_width: f64,
        viewport_height: f64,
        content_width: f64,
        content_height: f64,
    ) -> ScrollHandle {
        self.scroll_controller.bind(
            axis,
            viewport_width,
            viewport_height,
            content_width,
            content_height,
        )
    }

    pub(crate) fn take_pending_scroll_handle(&mut self, caller: &'static str) -> ScrollHandle {
        self.lazy.take_pending_scroll_handle(caller)
    }

    pub(crate) fn push_lazy_viewport(&mut self, viewport: vello::kurbo::Rect) {
        self.lazy.lazy_viewport_stack.push(viewport);
    }

    pub(crate) fn pop_lazy_viewport(&mut self, caller: &'static str) {
        self.lazy
            .lazy_viewport_stack
            .pop()
            .unwrap_or_else(|| panic!("lazy viewport stack underflow in {caller}"));
    }

    pub(crate) fn bind_picker_menu_state(&mut self) -> Rc<Cell<bool>> {
        self.popup_menu.bind_picker_menu_state()
    }

    pub(crate) fn bind_text_selection_slot(&mut self) -> Rc<RefCell<TextSelectionSlot>> {
        let index = self.text_editing.text_selection_cursor;
        self.text_editing.text_selection_cursor = self
            .text_editing
            .text_selection_cursor
            .checked_add(1)
            .expect("text selection slot cursor overflow");
        if index == self.text_editing.text_selection_slots.len() {
            self.text_editing
                .text_selection_slots
                .push(Rc::new(RefCell::new(TextSelectionSlot::default())));
        }
        Rc::clone(&self.text_editing.text_selection_slots[index])
    }

    pub(crate) fn next_text_input_index(&self) -> usize {
        self.text_editing.text_input_targets.len()
    }

    pub(crate) fn is_text_input_focused(&self, index: usize) -> bool {
        self.text_editing.focused_text_input.get() == Some(index)
    }

    pub(crate) fn current_ime_preedit(&self) -> Option<Str> {
        self.text_editing.ime_preedit.clone()
    }

    pub(crate) fn set_frame_instant(&mut self, at: Instant) {
        self.frame_instant = at;
        self.frame_clock.set(at);
    }

    pub(crate) fn frame_instant(&self) -> Instant {
        self.frame_instant
    }

    fn resolve_animated_scalar_with_discriminator<S>(
        &mut self,
        signal: &S,
        discriminator: usize,
    ) -> f32
    where
        S: Signal<Output = f32> + Clone + 'static,
    {
        let Some(identity) = signal.identity() else {
            return signal.get();
        };
        self.mark_scroll_content_animation_dependent();
        let now = self.frame_instant;
        let key = AnimationKey::scalar_with_discriminator(identity, discriminator);
        let handle = self
            .animation_controller
            .bind_scalar(key, signal.get(), now);
        let watcher_handle = handle.clone();
        let frame_clock = Rc::clone(&self.frame_clock);
        let redraw_requested = Rc::clone(&self.redraw_requested);
        let guard = signal.watch(move |update| {
            watcher_handle.apply_update_from_context(update, frame_clock.get());
            redraw_requested.set(true);
        });
        self.lifecycle.current_frame_retain.push(Retain::new(guard));
        handle.sample(now)
    }

    fn dynamic_transform_scalar_with_discriminator<S>(
        &mut self,
        signal: &S,
        discriminator: usize,
    ) -> DynamicTransformScalar
    where
        S: Signal<Output = f32> + Clone + 'static,
    {
        let Some(identity) = signal.identity() else {
            return DynamicTransformScalar {
                value: signal.get(),
                handle: None,
            };
        };
        let now = self.frame_instant;
        let key = AnimationKey::scalar_with_discriminator(identity, discriminator);
        let handle = self
            .animation_controller
            .bind_scalar(key, signal.get(), now);
        let watcher_handle = handle.clone();
        let frame_clock = Rc::clone(&self.frame_clock);
        let redraw_requested = Rc::clone(&self.redraw_requested);
        let guard = signal.watch(move |update| {
            watcher_handle.apply_update_from_context(update, frame_clock.get());
            redraw_requested.set(true);
        });
        let value = handle.sample(now);
        self.lifecycle.current_frame_retain.push(Retain::new(guard));
        DynamicTransformScalar {
            value,
            handle: Some(handle),
        }
    }

    fn capture_dynamic_transform(
        &mut self,
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
        transform: DynamicTransformComponents,
    ) {
        let local_ctx = ctx.with_identity_transforms(ctx.bounds);
        let subtree = Self::render_dynamic_subtree_with_local_interactions(
            self, ctx, local_ctx, env, content,
        );
        self.dynamic_transform_draws.push(DynamicTransformDraw {
            transform,
            base_transform: ctx.transform,
            base_hit_transform: ctx.hit_transform,
            bounds: ctx.bounds,
            subtree,
        });
    }

    fn capture_dynamic_opacity(
        &mut self,
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
        alpha: DynamicTransformScalar,
    ) {
        let local_ctx = ctx.with_identity_transforms(ctx.bounds);
        let subtree = Self::render_dynamic_subtree_with_local_interactions(
            self, ctx, local_ctx, env, content,
        );
        self.dynamic_opacity_draws.push(DynamicOpacityDraw {
            alpha,
            base_transform: ctx.transform,
            base_hit_transform: ctx.hit_transform,
            bounds: ctx.bounds,
            subtree,
        });
    }

    pub(crate) fn resolve_toggle_progress<S>(
        &mut self,
        signal: &S,
        default_animation: Animation,
    ) -> f32
    where
        S: Signal<Output = bool> + Clone + 'static,
    {
        let Some(identity) = signal.identity() else {
            return if signal.get() { 1.0 } else { 0.0 };
        };
        let now = self.frame_instant;
        let target = if signal.get() { 1.0 } else { 0.0 };
        let key = AnimationKey::scalar(identity);
        let handle = self.animation_controller.bind_scalar_target(
            key,
            target,
            default_animation.clone(),
            now,
        );
        let watcher_handle = handle.clone();
        let frame_clock = Rc::clone(&self.frame_clock);
        let rebuild_requested = Rc::clone(&self.rebuild_requested);
        let guard = signal.watch(move |update| {
            let target = if *update.value() { 1.0 } else { 0.0 };
            let animation = update
                .metadata()
                .try_get::<Animation>()
                .unwrap_or_else(|| default_animation.clone());
            watcher_handle.apply_target(target, Some(animation), frame_clock.get());
            rebuild_requested.set(true);
        });
        self.lifecycle.current_frame_retain.push(Retain::new(guard));
        handle.sample(now).clamp(0.0, 1.0)
    }

    pub(crate) fn sample_widget_scalar_target(
        &mut self,
        key: AnimationKey,
        target: f32,
        animation: Animation,
    ) -> f32 {
        let now = self.frame_instant;
        self.animation_controller
            .bind_scalar_target(key, target, animation, now)
            .sample(now)
    }

    pub(crate) fn sample_radio_indicator_state(
        &mut self,
        key: AnimationKey,
        selected: bool,
        motion: &RadioSelectionMotion,
    ) -> RadioIndicatorState {
        self.animation_controller
            .bind_radio_indicator(key, selected, motion, self.frame_instant)
    }

    pub(crate) fn sample_morph_progress(
        &mut self,
        animation: waterui_shape::MorphAnimation,
    ) -> f32 {
        if animation.duration.is_zero() {
            return 1.0;
        }
        let key = AnimationKey::renderer_local_repeating(self.render_depth);
        let elapsed = self.animation_controller.bind_timeline_phase(
            key,
            animation.duration,
            animation.repeat,
            self.frame_instant,
        );
        let raw = elapsed.as_secs_f32() / animation.duration.as_secs_f32();
        let cycle = if animation.repeat {
            let base = raw.fract();
            assert!(
                raw.is_finite() && raw >= 0.0,
                "morph animation cycle index must be finite and non-negative"
            );
            let index = raw.floor() as u64;
            if animation.autoreverse && index % 2 == 1 {
                1.0 - base
            } else {
                base
            }
        } else {
            raw.clamp(0.0, 1.0)
        };
        animation.easing.ease(cycle).clamp(0.0, 1.0)
    }

    fn sample_morph_draw_progress(&self, draw: &DynamicMorphDraw) -> f32 {
        let animation = draw.shape.animation;
        if animation.duration.is_zero() {
            return 1.0;
        }
        let elapsed = self
            .frame_instant
            .saturating_duration_since(draw.started_at);
        let raw = elapsed.as_secs_f32() / animation.duration.as_secs_f32();
        let cycle = if animation.repeat {
            let base = raw.fract();
            assert!(
                raw.is_finite() && raw >= 0.0,
                "morph animation cycle index must be finite and non-negative"
            );
            let index = raw.floor() as u64;
            if animation.autoreverse && index % 2 == 1 {
                1.0 - base
            } else {
                base
            }
        } else {
            raw.clamp(0.0, 1.0)
        };
        animation.easing.ease(cycle).clamp(0.0, 1.0)
    }

    fn dynamic_morph_is_active(&self, draw: &DynamicMorphDraw) -> bool {
        let animation = draw.shape.animation;
        animation.repeat
            || self
                .frame_instant
                .saturating_duration_since(draw.started_at)
                < animation.duration
    }

    fn draw_dynamic_morphs(
        &mut self,
        morphs: &[DynamicMorphDraw],
        parent_transform: vello::kurbo::Affine,
    ) {
        for morph in morphs {
            let progress = self.sample_morph_draw_progress(morph);
            let path = resolved_morph_shape_to_path(&morph.shape, progress, morph.bounds);
            let fill = resolved_color_to_peniko(morph.shape.fill);
            self.scene.fill(
                vello::peniko::Fill::NonZero,
                parent_transform * morph.transform,
                fill,
                None,
                &path,
            );
        }
    }

    fn draw_dynamic_transforms(
        &mut self,
        parent_ctx: RenderContext,
        transforms: &[DynamicTransformDraw],
    ) {
        for draw in transforms {
            let dynamic_transform = draw.transform.affine(self.frame_instant);
            let ctx = RenderContext::with_transforms(
                draw.bounds,
                parent_ctx.transform * draw.base_transform * dynamic_transform,
                parent_ctx.hit_transform * draw.base_hit_transform * dynamic_transform,
            );
            self.replay_dynamic_subtree(ctx, &draw.subtree);
        }
    }

    fn draw_dynamic_opacities(
        &mut self,
        parent_ctx: RenderContext,
        opacities: &[DynamicOpacityDraw],
    ) {
        for draw in opacities {
            let alpha = draw.alpha.sample(self.frame_instant).clamp(0.0, 1.0);
            let transform = parent_ctx.transform * draw.base_transform;
            let hit_transform = parent_ctx.hit_transform * draw.base_hit_transform;
            self.push_layer_rect(alpha, transform, draw.bounds);
            let previous_opacity = self.hit_test.hit_test_opacity;
            self.hit_test.hit_test_opacity = previous_opacity * alpha;
            let ctx = RenderContext::with_transforms(draw.bounds, transform, hit_transform);
            self.replay_dynamic_subtree(ctx, &draw.subtree);
            self.hit_test.hit_test_opacity = previous_opacity;
            self.pop_layer();
        }
    }

    /// Composites each placed `Dynamic` node from its retained `cached_subtree`. The
    /// subtree is taken out, replayed, and returned, so a content change to one node
    /// (which only refreshes that node's `cached_subtree`) is picked up here without
    /// touching any other node's placement.
    fn replay_dynamic_node_placements(
        &mut self,
        parent_ctx: RenderContext,
        placements: &[DynamicNodeDraw],
    ) {
        for placement in placements {
            let Some(subtree) = self
                .lifecycle
                .dynamic_nodes
                .get_mut(&placement.identity)
                .and_then(|node| node.cached_subtree.take())
            else {
                continue;
            };
            let ctx = RenderContext::with_transforms(
                placement.bounds,
                parent_ctx.transform * placement.base_transform,
                parent_ctx.hit_transform * placement.base_hit_transform,
            );
            self.replay_dynamic_subtree(ctx, &subtree);
            self.lifecycle
                .dynamic_nodes
                .get_mut(&placement.identity)
                .expect("hydrolysis dynamic node missing after placement replay")
                .cached_subtree = Some(subtree);
        }
    }

    /// Collects the animation keys of every active replayable scalar (transform and
    /// opacity) reachable from `subtree`, recursing through nested dynamic draws and
    /// through placed `Dynamic` nodes (whose content lives in their `cached_subtree`).
    fn collect_subtree_active_scalar_keys(
        &self,
        subtree: &DynamicSubtree,
        keys: &mut BTreeSet<AnimationKey>,
    ) {
        for transform in &subtree.dynamic_transforms {
            transform.transform.collect_active_scalar_keys(keys);
            self.collect_subtree_active_scalar_keys(&transform.subtree, keys);
        }
        for opacity in &subtree.dynamic_opacities {
            opacity.alpha.collect_active_key(keys);
            self.collect_subtree_active_scalar_keys(&opacity.subtree, keys);
        }
        for placement in &subtree.dynamic_node_draws {
            if let Some(cached) = self
                .lifecycle
                .dynamic_nodes
                .get(&placement.identity)
                .and_then(|node| node.cached_subtree.as_ref())
            {
                self.collect_subtree_active_scalar_keys(cached, keys);
            }
        }
        for scroll in &subtree.dynamic_scroll_draws {
            if let Some(cache) = self.scroll_content_caches.get(&scroll.cache_key) {
                self.collect_subtree_active_scalar_keys(&cache.subtree, keys);
            }
        }
    }

    /// Re-dispatches a `Dynamic` node's content into its `cached_subtree`, refreshing the
    /// intrinsic/proposal dimension caches. If the content's intrinsic size changed, the
    /// surrounding layout must reflow, so this escalates to a full structural rebuild.
    fn capture_dynamic_node_content(
        &mut self,
        identity: usize,
        content: AnyView,
        ctx: RenderContext,
        env: &Environment,
    ) {
        let content = normalize_layout_view(content, env);
        let dimensions = measure_view_dimensions(&content, &mut self.state, env);
        let proposal = ProposalSize::new(
            Some(ctx.bounds.width() as f32),
            Some(ctx.bounds.height() as f32),
        );
        let proposal_dimensions =
            measure_view_dimensions_with_proposal(&content, proposal, &mut self.state, env);
        let previous_dimensions = self.state.dynamic_intrinsic_cache.get(&identity).cloned();
        self.state
            .dynamic_intrinsic_cache
            .insert(identity, dimensions.clone());
        self.state.dynamic_dimensions_cache.insert(
            (
                identity,
                proposal.width.map(f32::to_bits),
                proposal.height.map(f32::to_bits),
            ),
            proposal_dimensions,
        );
        let local_ctx = ctx.with_identity_transforms(ctx.bounds);
        let subtree = Self::render_dynamic_subtree_with_local_interactions(
            self, ctx, local_ctx, env, content,
        );
        self.lifecycle
            .dynamic_nodes
            .get_mut(&identity)
            .expect("hydrolysis dynamic node missing after connect")
            .cached_subtree = Some(subtree);
        if previous_dimensions.is_some() && previous_dimensions.as_ref() != Some(&dimensions) {
            self.request_rebuild();
        }
    }

    /// Re-dispatches every dirty `Dynamic` node in isolation, refreshing only those
    /// nodes' cached subtrees. Returns `false` if any patch reflowed layout (escalating
    /// to a structural rebuild), in which case the caller must rebuild instead of
    /// compositing a patched frame.
    fn patch_dirty_dynamic_nodes(&mut self) -> bool {
        let dirty = core::mem::take(&mut *self.dirty_dynamic_nodes.borrow_mut());
        for identity in dirty {
            let Some((pending_view, ctx, env)) =
                self.lifecycle.dynamic_nodes.get(&identity).and_then(|node| {
                    Some((
                        Rc::clone(&node.pending_view),
                        node.dispatch_ctx?,
                        node.dispatch_env.clone()?,
                    ))
                })
            else {
                continue;
            };
            let Some(content) = pending_view.borrow_mut().take() else {
                continue;
            };
            // Re-dispatch under a retained capture so nested dynamic draws and Dynamic
            // node placements inside the patched content are captured, not baked.
            self.dynamic_transform_capture_depth = self
                .dynamic_transform_capture_depth
                .checked_add(1)
                .expect("hydrolysis reactive patch transform capture depth overflow");
            self.dynamic_morph_capture_depth = self
                .dynamic_morph_capture_depth
                .checked_add(1)
                .expect("hydrolysis reactive patch morph capture depth overflow");
            self.scroll_content_capture_depth = self
                .scroll_content_capture_depth
                .checked_add(1)
                .expect("hydrolysis reactive patch scroll capture depth overflow");
            self.capture_dynamic_node_content(identity, content, ctx, &env);
            self.scroll_content_capture_depth = self
                .scroll_content_capture_depth
                .checked_sub(1)
                .expect("hydrolysis reactive patch scroll capture depth underflow");
            self.dynamic_morph_capture_depth = self
                .dynamic_morph_capture_depth
                .checked_sub(1)
                .expect("hydrolysis reactive patch morph capture depth underflow");
            self.dynamic_transform_capture_depth = self
                .dynamic_transform_capture_depth
                .checked_sub(1)
                .expect("hydrolysis reactive patch transform capture depth underflow");
            if self.rebuild_requested.get() {
                return false;
            }
        }
        true
    }

    pub(crate) fn sample_repeating_motion(&mut self, cycle: Duration) -> Duration {
        let key = AnimationKey::renderer_local_repeating(self.render_depth);
        self.animation_controller
            .bind_repeating_phase(key, cycle, self.frame_instant)
    }

    fn canonical_geometry_bits(value: f64) -> u64 {
        if value == 0.0 {
            0.0f64.to_bits()
        } else {
            value.to_bits()
        }
    }

    fn lazy_stack_slot_key(&self, ctx: RenderContext) -> LazyStackSlotKey {
        let [scale_x, skew_y, skew_x, scale_y, translate_x, translate_y] =
            ctx.transform.as_coeffs();
        LazyStackSlotKey {
            depth: self.render_depth,
            transform: [
                Self::canonical_geometry_bits(scale_x),
                Self::canonical_geometry_bits(skew_y),
                Self::canonical_geometry_bits(skew_x),
                Self::canonical_geometry_bits(scale_y),
                Self::canonical_geometry_bits(translate_x),
                Self::canonical_geometry_bits(translate_y),
            ],
            bounds: [
                Self::canonical_geometry_bits(ctx.bounds.x0),
                Self::canonical_geometry_bits(ctx.bounds.y0),
                Self::canonical_geometry_bits(ctx.bounds.x1),
                Self::canonical_geometry_bits(ctx.bounds.y1),
            ],
        }
    }

    fn render_layout_container(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        layout: Box<dyn Layout>,
        children: Vec<AnyView>,
        env: &Environment,
    ) {
        let mut resolved_children = Vec::with_capacity(children.len());
        let mut child_envs = Vec::with_capacity(children.len());
        for (index, child) in children.into_iter().enumerate() {
            let child_env = local_state_child_env(env, index);
            resolved_children.push(normalize_layout_view(child, &child_env));
            child_envs.push(child_env);
        }

        let state = RefCell::new(&mut renderer.state);
        let mut subviews = Vec::with_capacity(resolved_children.len());
        for (child, child_env) in resolved_children.iter().zip(&child_envs) {
            subviews.push(HydroSubview::from_view(child, &state, child_env));
        }
        let refs: Vec<&dyn SubView> = subviews.iter().map(|view| view as &dyn SubView).collect();

        let proposal = ProposalSize::new(
            Some(ctx.bounds.width() as f32),
            Some(ctx.bounds.height() as f32),
        );
        let layout_size = layout.size_that_fits(proposal, &refs);
        let stretch_axis = layout.stretch_axis();
        let width = if matches!(stretch_axis, StretchAxis::Horizontal | StretchAxis::Both) {
            ctx.bounds.width() as f32
        } else {
            layout_size.width.min(ctx.bounds.width() as f32)
        };
        let height = if matches!(stretch_axis, StretchAxis::Vertical | StretchAxis::Both) {
            ctx.bounds.height() as f32
        } else {
            layout_size.height.min(ctx.bounds.height() as f32)
        };
        let bounds = LayoutRect::from_size(LayoutSize::new(width, height));
        let child_rects = layout.place(bounds, &refs);

        for ((index, child), rect) in resolved_children.into_iter().enumerate().zip(child_rects) {
            let child_transform =
                vello::kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y())));
            let child_bounds = vello::kurbo::Rect::new(
                0.0,
                0.0,
                f64::from(rect.width()),
                f64::from(rect.height()),
            );
            Self::dispatch_any(
                renderer,
                ctx.child(child_transform, child_bounds),
                &child_envs[index],
                child,
            );
        }
    }

    pub(crate) fn render_fixed_container(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        container: Native<FixedContainer>,
        env: &Environment,
    ) {
        let (layout, children) = container.into_inner().into_inner();
        Self::render_layout_container(renderer, ctx, layout, children, env);
    }

    pub(crate) fn render_lazy_container(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        container: Native<LazyContainer>,
        env: &Environment,
    ) {
        renderer.mark_scroll_content_viewport_dependent();
        let (layout, children) = container.into_inner().into_inner();
        let axis_config = lazy_stack_axis_config(layout.as_ref());
        let count = children.len().get();
        if count == 0 {
            return;
        }
        let visible_bounds = {
            renderer
                .lazy
                .lazy_viewport_stack
                .last()
                .copied()
                .unwrap_or(ctx.bounds)
        };
        let slot_key = renderer.lazy_stack_slot_key(ctx);
        renderer
            .lazy
            .lazy_stack_controller
            .bind(slot_key)
            .prepare_len(count);
        let (visible_start, visible_end, spacing) = match axis_config {
            LazyStackAxisConfig::Vertical { spacing, .. } => {
                (visible_bounds.y0, visible_bounds.y1, spacing)
            }
            LazyStackAxisConfig::Horizontal { spacing, .. } => {
                (visible_bounds.x0, visible_bounds.x1, spacing)
            }
        };
        let window = resolve_visible_index_window(count, visible_start, visible_end, |index| {
            let cached_extent = {
                renderer
                    .lazy
                    .lazy_stack_controller
                    .slot(slot_key)
                    .item_extents[index]
            };
            let extent = if let Some(extent) = cached_extent {
                extent
            } else {
                let child = children.get_view(index).unwrap_or_else(|| {
                    panic!("LazyContainer failed to materialize child at index {index}")
                });
                let child_env = local_state_child_env(env, index);
                let child = normalize_layout_view(child, &child_env);
                let state = RefCell::new(&mut renderer.state);
                let subview = HydroSubview::from_view(&child, &state, &child_env);
                let proposal = match axis_config {
                    LazyStackAxisConfig::Vertical { .. } => {
                        ProposalSize::new(Some(ctx.bounds.width() as f32), None)
                    }
                    LazyStackAxisConfig::Horizontal { .. } => {
                        ProposalSize::new(None, Some(ctx.bounds.height() as f32))
                    }
                };
                let size = subview.measure(proposal).size;
                let extent = match axis_config {
                    LazyStackAxisConfig::Vertical { .. } => f64::from(size.height),
                    LazyStackAxisConfig::Horizontal { .. } => f64::from(size.width),
                };
                renderer
                    .lazy
                    .lazy_stack_controller
                    .slot_mut(slot_key)
                    .item_extents[index] = Some(extent);
                extent
            };
            if index + 1 < count {
                extent + spacing
            } else {
                extent
            }
        });

        let mut cursor = window.leading_offset;
        for index in window.start..window.end {
            let child = children.get_view(index).unwrap_or_else(|| {
                panic!("LazyContainer failed to materialize child at index {index}")
            });
            let child_env = local_state_child_env(env, index);
            let child = normalize_layout_view(child, &child_env);
            let state = RefCell::new(&mut renderer.state);
            let subview = HydroSubview::from_view(&child, &state, &child_env);
            let proposal = match axis_config {
                LazyStackAxisConfig::Vertical { .. } => {
                    ProposalSize::new(Some(ctx.bounds.width() as f32), None)
                }
                LazyStackAxisConfig::Horizontal { .. } => {
                    ProposalSize::new(None, Some(ctx.bounds.height() as f32))
                }
            };
            let size = subview.measure(proposal).size;
            let child_rect = match axis_config {
                LazyStackAxisConfig::Vertical { alignment, .. } => {
                    assert!(
                        !(matches!(
                            subview.stretch_axis(),
                            StretchAxis::Vertical | StretchAxis::Both | StretchAxis::MainAxis
                        )),
                        "hydrolysis LazyContainer VStackLayout does not support children stretching on main axis"
                    );
                    let child_width = if matches!(
                        subview.stretch_axis(),
                        StretchAxis::Horizontal | StretchAxis::Both | StretchAxis::CrossAxis
                    ) || size.width.is_infinite()
                    {
                        ctx.bounds.width()
                    } else {
                        f64::from(size.width).min(ctx.bounds.width())
                    };
                    let child_height = f64::from(size.height);
                    let x = if alignment == HorizontalAlignment::Leading {
                        ctx.bounds.x0
                    } else if alignment == HorizontalAlignment::Trailing {
                        ctx.bounds.x1 - child_width
                    } else {
                        ctx.bounds.x0 + (ctx.bounds.width() - child_width) / 2.0
                    };
                    vello::kurbo::Rect::new(x, cursor, x + child_width, cursor + child_height)
                }
                LazyStackAxisConfig::Horizontal { alignment, .. } => {
                    assert!(
                        !(matches!(
                            subview.stretch_axis(),
                            StretchAxis::Horizontal | StretchAxis::Both | StretchAxis::MainAxis
                        )),
                        "hydrolysis LazyContainer HStackLayout does not support children stretching on main axis"
                    );
                    let child_width = f64::from(size.width);
                    let child_height = if matches!(
                        subview.stretch_axis(),
                        StretchAxis::Vertical | StretchAxis::Both | StretchAxis::CrossAxis
                    ) || size.height.is_infinite()
                    {
                        ctx.bounds.height()
                    } else {
                        f64::from(size.height).min(ctx.bounds.height())
                    };
                    let y = if alignment == VerticalAlignment::Top {
                        ctx.bounds.y0
                    } else if alignment == VerticalAlignment::Bottom {
                        ctx.bounds.y1 - child_height
                    } else {
                        ctx.bounds.y0 + (ctx.bounds.height() - child_height) / 2.0
                    };
                    vello::kurbo::Rect::new(cursor, y, cursor + child_width, y + child_height)
                }
            };
            let extent = match axis_config {
                LazyStackAxisConfig::Vertical { .. } => child_rect.height(),
                LazyStackAxisConfig::Horizontal { .. } => child_rect.width(),
            };
            {
                renderer
                    .lazy
                    .lazy_stack_controller
                    .slot_mut(slot_key)
                    .item_extents[index] = Some(extent);
            }
            Self::dispatch_any(
                renderer,
                ctx.child(
                    vello::kurbo::Affine::translate((child_rect.x0, child_rect.y0)),
                    vello::kurbo::Rect::new(0.0, 0.0, child_rect.width(), child_rect.height()),
                ),
                &child_env,
                child,
            );
            cursor += extent;
            if index + 1 < count {
                cursor += spacing;
            }
        }
    }

    pub(crate) fn render_str(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        text: Str,
        env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            if !env
                .get::<AccessibilityHidden>()
                .is_some_and(AccessibilityHidden::is_hidden)
            {
                let label =
                    renderer.resolve_accessibility_label(env, Some(text.as_str().to_owned()));
                if let Some(label) = label {
                    let mut node = AccessibilityNode::new(
                        renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Label),
                    );
                    node.set_label(label);
                    let _ = renderer.register_accessibility_node(
                        node,
                        transformed_rect(ctx.hit_transform, ctx.bounds),
                        env,
                        None,
                    );
                }
            }
        }
        Self::render_styled_text(
            &mut renderer.state,
            &mut renderer.scene,
            ctx,
            StyledStr::plain(text),
            HorizontalAlignment::Leading,
            env,
        );
    }

    pub(crate) fn render_dynamic(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        dynamic: Native<Dynamic>,
        env: &Environment,
    ) {
        let dynamic = dynamic.into_inner();
        let identity = dynamic.identity();
        renderer
            .lifecycle
            .dynamic_identities_current_frame
            .push(identity);
        let pending_view = {
            if let Some(node) = renderer.lifecycle.dynamic_nodes.get(&identity) {
                Rc::clone(&node.pending_view)
            } else {
                let pending_view = Rc::new(RefCell::new(None::<AnyView>));
                let patch_requested = Rc::clone(&renderer.patch_requested);
                let dirty_dynamic_nodes = Rc::clone(&renderer.dirty_dynamic_nodes);
                let rebuild_generation = Rc::clone(&renderer.rebuild_generation);
                let rebuild_in_progress = Rc::clone(&renderer.rebuild_in_progress);
                let render_generation = Rc::new(Cell::new(0));
                dynamic.connect_with_pending_view(Rc::clone(&pending_view), {
                    let pending_view = Rc::clone(&pending_view);
                    let patch_requested = Rc::clone(&patch_requested);
                    let dirty_dynamic_nodes = Rc::clone(&dirty_dynamic_nodes);
                    let rebuild_generation = Rc::clone(&rebuild_generation);
                    let rebuild_in_progress = Rc::clone(&rebuild_in_progress);
                    let render_generation = Rc::clone(&render_generation);
                    move |update| {
                        let is_initial_content = update
                            .metadata()
                            .try_get::<DynamicInitialContent>()
                            .is_some();
                        if is_initial_content
                            && rebuild_in_progress.get()
                            && render_generation.get() == rebuild_generation.get()
                        {
                            return;
                        }
                        *pending_view.borrow_mut() = Some(update.into_value());
                        // A real content change is a fine-grained reactive update: mark
                        // this node dirty so it can be re-dispatched in isolation. If the
                        // re-dispatch reflows layout, render_dynamic escalates to a full
                        // rebuild itself.
                        if !is_initial_content
                            && (!rebuild_in_progress.get()
                                || render_generation.get() == rebuild_generation.get())
                        {
                            dirty_dynamic_nodes.borrow_mut().insert(identity);
                            patch_requested.set(true);
                        }
                    }
                });
                renderer.lifecycle.dynamic_nodes.insert(
                    identity,
                    DynamicNode {
                        pending_view: Rc::clone(&pending_view),
                        cached_subtree: None,
                        render_generation,
                        dispatch_ctx: None,
                        dispatch_env: None,
                    },
                );
                pending_view
            }
        };
        let current_generation = renderer.rebuild_generation.get();
        renderer
            .lifecycle
            .dynamic_nodes
            .get(&identity)
            .expect("hydrolysis dynamic node missing before render")
            .render_generation
            .set(current_generation);

        let update = pending_view.borrow_mut().take();
        if let Some(content) = update {
            renderer.capture_dynamic_node_content(identity, content, ctx, env);
        }
        if renderer
            .lifecycle
            .dynamic_nodes
            .get(&identity)
            .is_some_and(|node| node.cached_subtree.is_none())
        {
            let local_ctx = ctx.with_identity_transforms(ctx.bounds);
            let subtree = Self::render_dynamic_subtree_with_local_interactions(
                renderer,
                ctx,
                local_ctx,
                env,
                AnyView::new(()),
            );
            renderer
                .lifecycle
                .dynamic_nodes
                .get_mut(&identity)
                .expect("hydrolysis dynamic node missing after empty subtree initialization")
                .cached_subtree = Some(subtree);
        }

        // Remember where and with what environment this node was dispatched, so a later
        // content change can re-dispatch just this node in isolation (reactive patch).
        if let Some(node) = renderer.lifecycle.dynamic_nodes.get_mut(&identity) {
            node.dispatch_ctx = Some(ctx);
            node.dispatch_env = Some(env.clone());
        }

        // Inside a retained capture, record a placement instead of baking the node's
        // content into the parent scene. The content stays in `cached_subtree` and is
        // composited at replay, so a later content change to this node can be patched
        // in isolation without re-walking the rest of the window.
        if renderer.dynamic_transform_capture_depth > 0 {
            renderer.dynamic_node_draws.push(DynamicNodeDraw {
                identity,
                base_transform: ctx.transform,
                base_hit_transform: ctx.hit_transform,
                bounds: ctx.bounds,
            });
            return;
        }

        let subtree = renderer
            .lifecycle
            .dynamic_nodes
            .get_mut(&identity)
            .and_then(|node| node.cached_subtree.take())
            .expect("hydrolysis Dynamic must provide an initial view before dispatch");
        renderer.replay_dynamic_subtree(ctx, &subtree);
        renderer
            .lifecycle
            .dynamic_nodes
            .get_mut(&identity)
            .expect("hydrolysis dynamic node missing after replay")
            .cached_subtree = Some(subtree);
    }

    pub(crate) fn render_system_icon(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        icon: Native<SystemIcon>,
        env: &Environment,
    ) {
        let styled = StyledStr::plain(icon.into_inner().name);
        Self::render_styled_text(
            &mut renderer.state,
            &mut renderer.scene,
            ctx,
            styled,
            HorizontalAlignment::Leading,
            env,
        );
    }

    pub(crate) fn render_gpu_surface(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        surface: Native<GpuSurface>,
        env: &Environment,
    ) {
        let slot_index = renderer.bind_gpu_surface_slot(surface.into_inner(), env);
        renderer.push_gpu_surface_layer(slot_index, ctx.transform, ctx.bounds);
    }

    pub(crate) fn render_scene_view(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        scene_view: Native<SceneView>,
        env: &Environment,
    ) {
        let scene_view = scene_view.into_inner();
        let incoming_content = Rc::new(RefCell::new(Some(scene_view.into_content())));
        let init_content = Rc::clone(&incoming_content);
        let runtime = local_shared(env, move || {
            RefCell::new(SceneViewRuntime::new(
                init_content
                    .borrow_mut()
                    .take()
                    .expect("hydrolysis SceneView local state initializer must run exactly once"),
            ))
        });
        if let Some(content) = incoming_content.borrow_mut().take() {
            let incoming_type = content.concrete_type_id();
            let mut runtime = runtime.borrow_mut();
            if runtime.content.concrete_type_id() != incoming_type {
                runtime.replace_content(content);
            }
        }
        let rebuild_handle = renderer.rebuild_handle();
        let mut runtime = runtime.borrow_mut();
        runtime
            .content
            .set_invalidator(Some(Rc::new(move || rebuild_handle.set(true))));

        let mut scene = vello::Scene::new();
        let mut scene2d = VelloScene2D::new(&mut scene);
        #[allow(clippy::cast_precision_loss)]
        let needs_next_frame = runtime.content.build_scene(
            &mut scene2d,
            ctx.bounds.width() as f32,
            ctx.bounds.height() as f32,
        );
        renderer.scene.append(
            &scene,
            Some(ctx.transform * vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0))),
        );
        if needs_next_frame {
            renderer.request_next_frame_rebuild();
        }
    }

    pub(crate) fn render_view_effect(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        effect: Native<ViewEffectErased>,
        env: &Environment,
    ) {
        let incoming_effect = Rc::new(RefCell::new(Some(effect.into_inner())));
        let init_effect = Rc::clone(&incoming_effect);
        let runtime = local_shared(env, move || {
            RefCell::new(ViewEffectRuntime::new(
                init_effect
                    .borrow_mut()
                    .take()
                    .expect("hydrolysis ViewEffect local state initializer must run exactly once"),
            ))
        });
        let mut runtime = runtime.borrow_mut();
        if let Some(mut effect) = incoming_effect.borrow_mut().take() {
            let incoming_type = effect.concrete_type_id();
            if runtime.effect.concrete_type_id() != incoming_type {
                runtime.replace_effect(effect);
            } else {
                runtime.effect.replace_content(effect.take_content());
                runtime.effect.set_output_size(effect.output_size());
            }
        }
        let (device, queue) = {
            let (device, queue) = renderer.state().frame_resources();
            (device.clone(), queue.clone())
        };

        let input_width = (ctx.bounds.width().max(1.0).round()) as u32;
        let input_height = (ctx.bounds.height().max(1.0).round()) as u32;
        let output_size = runtime.effect.output_size();
        let (output_width, output_height) = output_size.compute(input_width, input_height);
        assert!(
            !(output_width == 0 || output_height == 0),
            "hydrolysis ViewEffect requires non-zero output dimensions"
        );

        let subtree = Self::render_subtree_scene(renderer, ctx, env, runtime.effect.take_content());

        let input_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_view_effect_input"),
            size: wgpu::Extent3d {
                width: input_width,
                height: input_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor::default());
        renderer
            .vello_renderer
            .render_to_texture(
                &device,
                &queue,
                &subtree,
                &input_view,
                &vello::RenderParams {
                    base_color: vello::peniko::Color::TRANSPARENT,
                    width: input_width,
                    height: input_height,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .expect("hydrolysis ViewEffect failed to capture child scene");

        let setup_context = ViewEffectContext {
            device: &device,
            queue: &queue,
            input_format: wgpu::TextureFormat::Rgba8Unorm,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            pipeline_cache: None,
        };
        if !runtime.setup_complete {
            pollster::block_on(runtime.effect.setup(&setup_context));
            runtime.setup_complete = true;
        }

        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_view_effect_output"),
            size: wgpu::Extent3d {
                width: output_width,
                height: output_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let input = ViewEffectInput {
            device: &device,
            queue: &queue,
            texture: &input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: input_width,
            height: input_height,
        };
        let output = ViewEffectOutput {
            device: &device,
            queue: &queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: output_width,
            height: output_height,
        };
        runtime.effect.render(&input, &output);
        let needs_redraw = runtime.effect.needs_redraw();
        drop(runtime);
        if needs_redraw {
            renderer.request_next_frame_rebuild();
        }

        let image = renderer.vello_renderer.register_texture(output_texture);
        renderer.compositor.active_filter_images.push(image.clone());
        let image_transform = vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0))
            * vello::kurbo::Affine::scale_non_uniform(
                ctx.bounds.width() / f64::from(output_width),
                ctx.bounds.height() / f64::from(output_height),
            );
        renderer.scene.draw_image(
            &vello::peniko::ImageBrush::new(image),
            ctx.transform * image_transform,
        );
    }

    pub(crate) fn render_resolved_color(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        color: Native<ResolvedColor>,
        _env: &Environment,
    ) {
        let brush = resolved_color_to_peniko(color.into_inner());
        renderer.scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            brush,
            None,
            &ctx.bounds,
        );
    }

    pub(crate) fn render_resolved_gradient(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        gradient: Native<ResolvedGradient>,
        _env: &Environment,
    ) {
        let brush = resolved_gradient_to_brush(&gradient.into_inner(), ctx.bounds);
        renderer.scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            &brush,
            None,
            &ctx.bounds,
        );
    }

    pub(crate) fn render_resolved_shape(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        shape: Native<ResolvedShape>,
        _env: &Environment,
    ) {
        let resolved = shape.into_inner();
        let path = resolved_shape_to_path(&resolved, ctx.bounds);
        let fill = resolved_color_to_peniko(resolved.fill);
        renderer.scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            fill,
            None,
            &path,
        );
    }

    pub(crate) fn render_resolved_morph_shape(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        shape: Native<ResolvedMorphShape>,
        _env: &Environment,
    ) {
        let resolved = shape.into_inner();
        if resolved.progress.is_none() && renderer.dynamic_morph_capture_depth > 0 {
            renderer.dynamic_morph_draws.push(DynamicMorphDraw {
                shape: resolved,
                bounds: ctx.bounds,
                transform: ctx.transform,
                started_at: renderer.frame_instant,
            });
            return;
        }
        let progress = if let Some(progress) = resolved.progress.as_ref() {
            renderer
                .resolve_animated_scalar_with_discriminator(progress, MORPH_PROGRESS_ANIMATION_KEY)
        } else {
            renderer.sample_morph_progress(resolved.animation)
        };
        let path = resolved_morph_shape_to_path(&resolved, progress, ctx.bounds);
        let fill = resolved_color_to_peniko(resolved.fill);
        renderer.scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            fill,
            None,
            &path,
        );
    }

    fn render_environment_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Environment>,
        env: &Environment,
    ) {
        let (content, scoped_env) = flatten_environment_metadata_owned(AnyView::new(metadata), env);
        renderer.dispatch_boxed_with_render_depth(content, &scoped_env, ctx);
    }

    fn render_retain_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Retain>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        renderer.lifecycle.current_frame_retain.push(value);
        renderer.dispatch_boxed_with_render_depth(content, env, ctx);
    }

    fn render_opacity_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Opacity>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        // Inside a dynamic-subtree capture, an animated opacity is captured as a
        // replayable dynamic layer (re-sampled at replay) instead of baked into the
        // scene, so animation-only frames can refresh by replay without re-dispatch.
        if renderer.dynamic_transform_capture_depth > 0 && value.value.identity().is_some() {
            let alpha = renderer
                .dynamic_transform_scalar_with_discriminator(&value.value, OPACITY_ANIMATION_KEY);
            renderer.capture_dynamic_opacity(ctx, env, content, alpha);
            return;
        }
        let alpha =
            renderer.resolve_animated_scalar_with_discriminator(&value.value, OPACITY_ANIMATION_KEY);
        renderer.push_layer_rect(alpha, ctx.transform, ctx.bounds);

        let previous_opacity = renderer.hit_test.hit_test_opacity;
        renderer.hit_test.hit_test_opacity = previous_opacity * alpha;
        renderer.dispatch_boxed_with_render_depth(content, env, ctx);
        renderer.hit_test.hit_test_opacity = previous_opacity;

        renderer.pop_layer();
    }

    fn render_applied_filter_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<AppliedFilter>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let incoming_filter = Rc::new(RefCell::new(Some(value)));
        let init_filter = Rc::clone(&incoming_filter);
        let runtime = local_shared(env, move || {
            RefCell::new(AppliedFilterRuntime::new(
                init_filter.borrow_mut().take().expect(
                    "hydrolysis AppliedFilter local state initializer must run exactly once",
                ),
            ))
        });
        if let Some(filter) = incoming_filter.borrow_mut().take() {
            let incoming_type = filter.concrete_type_id();
            let mut runtime = runtime.borrow_mut();
            if runtime.filter.concrete_type_id() != incoming_type {
                runtime.replace_filter(filter);
            }
        }
        let (device, queue) = {
            let (device, queue) = renderer.state().frame_resources();
            (device.clone(), queue.clone())
        };

        let width = (ctx.bounds.width().max(1.0).round()) as u32;
        let height = (ctx.bounds.height().max(1.0).round()) as u32;
        let should_capture_input = {
            let runtime = runtime.borrow();
            !renderer.reuse_applied_filter_inputs || !runtime.has_input_texture(width, height)
        };
        let input_view = {
            let mut runtime = runtime.borrow_mut();
            let (_, view) = runtime.input_texture(&device, width, height);
            view.clone()
        };
        if should_capture_input {
            let capture_started_at = Instant::now();
            let subtree_scene = Self::render_subtree_scene(renderer, ctx, env, content);
            renderer
                .vello_renderer
                .render_to_texture(
                    &device,
                    &queue,
                    &subtree_scene,
                    &input_view,
                    &vello::RenderParams {
                        base_color: vello::peniko::Color::TRANSPARENT,
                        width,
                        height,
                        antialiasing_method: vello::AaConfig::Area,
                    },
                )
                .expect("hydrolysis AppliedFilter: failed to render subtree");
            renderer.frame_applied_filter_capture += capture_started_at.elapsed();
        }

        let effect_started_at = Instant::now();
        let (image, needs_redraw) = runtime.borrow_mut().render_output(
            &device,
            &queue,
            &mut renderer.vello_renderer,
            width,
            height,
        );
        renderer.frame_applied_filter_effect += effect_started_at.elapsed();
        renderer.frame_applied_filter_count = renderer
            .frame_applied_filter_count
            .checked_add(1)
            .expect("hydrolysis applied filter counter overflow");
        renderer.remember_active_applied_filter(Rc::clone(&runtime), width, height);
        if needs_redraw {
            renderer.request_redraw();
        }

        let image_transform = vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0))
            * vello::kurbo::Affine::scale_non_uniform(
                ctx.bounds.width() / f64::from(image.width),
                ctx.bounds.height() / f64::from(image.height),
            );
        let scene = renderer.scene_mut();
        scene.draw_image(
            &vello::peniko::ImageBrush::new(image),
            ctx.transform * image_transform,
        );
    }

    fn render_scale_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Scale>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let center = anchor_point(ctx.bounds, value.anchor);
        if renderer.dynamic_transform_capture_depth > 0
            && (value.x.identity().is_some() || value.y.identity().is_some())
        {
            let scale_x = renderer
                .dynamic_transform_scalar_with_discriminator(&value.x, SCALE_X_ANIMATION_KEY);
            let scale_y = renderer
                .dynamic_transform_scalar_with_discriminator(&value.y, SCALE_Y_ANIMATION_KEY);
            renderer.capture_dynamic_transform(
                ctx,
                env,
                content,
                DynamicTransformComponents::scale(scale_x, scale_y, center),
            );
            return;
        }
        let (scale_x, scale_y) = (
            renderer.resolve_animated_scalar_with_discriminator(&value.x, SCALE_X_ANIMATION_KEY),
            renderer.resolve_animated_scalar_with_discriminator(&value.y, SCALE_Y_ANIMATION_KEY),
        );
        let transform = vello::kurbo::Affine::translate((center.x, center.y))
            * vello::kurbo::Affine::scale_non_uniform(f64::from(scale_x), f64::from(scale_y))
            * vello::kurbo::Affine::translate((-center.x, -center.y));
        Self::dispatch_any(renderer, ctx.child(transform, ctx.bounds), env, content);
    }

    fn render_rotation_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Rotation>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let center = anchor_point(ctx.bounds, value.anchor);
        if renderer.dynamic_transform_capture_depth > 0 && value.angle.identity().is_some() {
            let angle = renderer
                .dynamic_transform_scalar_with_discriminator(&value.angle, ROTATION_ANIMATION_KEY);
            renderer.capture_dynamic_transform(
                ctx,
                env,
                content,
                DynamicTransformComponents::rotation(angle, center),
            );
            return;
        }
        let radians = f64::from(
            renderer
                .resolve_animated_scalar_with_discriminator(&value.angle, ROTATION_ANIMATION_KEY),
        )
        .to_radians();
        let transform = vello::kurbo::Affine::translate((center.x, center.y))
            * vello::kurbo::Affine::rotate(radians)
            * vello::kurbo::Affine::translate((-center.x, -center.y));
        Self::dispatch_any(renderer, ctx.child(transform, ctx.bounds), env, content);
    }

    fn render_offset_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Offset>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        if renderer.dynamic_transform_capture_depth > 0
            && (value.x.identity().is_some() || value.y.identity().is_some())
        {
            let offset_x = renderer
                .dynamic_transform_scalar_with_discriminator(&value.x, OFFSET_X_ANIMATION_KEY);
            let offset_y = renderer
                .dynamic_transform_scalar_with_discriminator(&value.y, OFFSET_Y_ANIMATION_KEY);
            renderer.capture_dynamic_transform(
                ctx,
                env,
                content,
                DynamicTransformComponents::offset(offset_x, offset_y),
            );
            return;
        }
        let (offset_x, offset_y) = (
            renderer.resolve_animated_scalar_with_discriminator(&value.x, OFFSET_X_ANIMATION_KEY),
            renderer.resolve_animated_scalar_with_discriminator(&value.y, OFFSET_Y_ANIMATION_KEY),
        );
        let transform = vello::kurbo::Affine::translate((f64::from(offset_x), f64::from(offset_y)));
        Self::dispatch_any(renderer, ctx.child(transform, ctx.bounds), env, content);
    }

    fn render_clip_shape_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<ClipShape>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let clip_path = path_commands_to_path(value.commands(), ctx.bounds);
        renderer.push_layer_path(1.0, ctx.transform, clip_path);
        Self::dispatch_any(renderer, ctx, env, content);
        renderer.pop_layer();
    }

    fn render_border_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Border>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let border = value;
        Self::dispatch_any(renderer, ctx, env, content);

        if border.width <= 0.0 {
            return;
        }

        let brush = resolved_color_to_peniko(border.color.resolve(env).get());
        let width = f64::from(border.width);

        if border.edges.all() && border.corner_radius > 0.0 {
            let rounded =
                vello::kurbo::RoundedRect::from_rect(ctx.bounds, f64::from(border.corner_radius));
            let stroke = vello::kurbo::Stroke::new(width);
            renderer
                .scene
                .stroke(&stroke, ctx.transform, brush, None, &rounded);
            return;
        }

        if border.edges.top {
            let top = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                ctx.bounds.y0 + width,
            );
            renderer.scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                brush,
                None,
                &top,
            );
        }
        if border.edges.bottom {
            let bottom = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y1 - width,
                ctx.bounds.x1,
                ctx.bounds.y1,
            );
            renderer.scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                brush,
                None,
                &bottom,
            );
        }
        if border.edges.leading {
            let leading = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x0 + width,
                ctx.bounds.y1,
            );
            renderer.scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                brush,
                None,
                &leading,
            );
        }
        if border.edges.trailing {
            let trailing = vello::kurbo::Rect::new(
                ctx.bounds.x1 - width,
                ctx.bounds.y0,
                ctx.bounds.x1,
                ctx.bounds.y1,
            );
            renderer.scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                brush,
                None,
                &trailing,
            );
        }
    }

    fn render_shadow_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Shadow>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let shadow = value;
        let blur = f64::from(shadow.radius.max(0.0));
        let offset_x = f64::from(shadow.offset.x);
        let offset_y = f64::from(shadow.offset.y);
        let shadow_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0 + offset_x,
            ctx.bounds.y0 + offset_y,
            ctx.bounds.x1 + offset_x,
            ctx.bounds.y1 + offset_y,
        );
        let shadow_color = resolved_color_to_peniko(shadow.color.resolve(env).get());

        renderer.scene.draw_blurred_rounded_rect(
            ctx.transform,
            shadow_rect,
            shadow_color,
            blur,
            blur,
        );
        Self::dispatch_any(renderer, ctx, env, content);
    }

    fn render_focused_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Focused>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let should_focus = renderer.read_signal(&value.0);
        let start = renderer.text_editing.text_input_targets.len();
        Self::dispatch_any(renderer, ctx, env, content);
        let end = renderer.text_editing.text_input_targets.len();
        let focus_target_count = end - start;
        assert!(
            focus_target_count == 1,
            "hydrolysis .focused() requires exactly one TextField or SecureField in the wrapped subtree, found {focus_target_count}"
        );
        let target = renderer
            .text_editing
            .text_input_targets
            .get_mut(start)
            .expect("hydrolysis focused metadata missing registered text input target");
        assert!(
            target.focus_binding.is_none(),
            "hydrolysis does not allow multiple .focused() modifiers to target the same control"
        );
        target.focus_binding = Some(value.0.clone());

        if should_focus {
            renderer.set_focused_text_input(Some(start));
            return;
        }

        if matches!(
            renderer.text_editing.focused_text_input.get(),
            Some(index) if index >= start && index < end
        ) {
            renderer.set_focused_text_input(None);
        }
    }

    fn render_hittable_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Hittable>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let enabled = renderer.read_signal(&value.enabled);
        let pointer_start = renderer.hit_test.pointer_targets.len();
        let gesture_start = renderer.gesture_engine.target_count();
        let cursor_start = renderer.hit_test.cursor_targets.len();
        let hover_start = renderer.hit_test.hover_targets.len();
        let hover_cursor_start = renderer.hit_test.interaction.hover_cursor();
        let scroll_start = renderer.hit_test.scroll_targets.len();
        let text_start = renderer.text_editing.text_input_targets.len();

        Self::dispatch_any(renderer, ctx, env, content);

        if enabled {
            return;
        }

        renderer.hit_test.pointer_targets.truncate(pointer_start);
        renderer.ensure_active_pointer_drag_target_is_live();
        renderer.gesture_engine.truncate_targets(gesture_start);
        renderer.hit_test.cursor_targets.truncate(cursor_start);
        renderer.hit_test.hover_targets.truncate(hover_start);
        renderer
            .hit_test
            .interaction
            .rewind_hover_to(hover_cursor_start);
        renderer.hit_test.scroll_targets.truncate(scroll_start);
        let text_end = renderer.text_editing.text_input_targets.len();
        renderer
            .text_editing
            .text_input_targets
            .truncate(text_start);

        if matches!(
            renderer.text_editing.focused_text_input.get(),
            Some(index) if index >= text_start && index < text_end
        ) {
            renderer.set_focused_text_input(None);
        }
    }

    fn render_cursor_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Cursor>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let style = renderer.read_signal(&value.style);
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        renderer.register_cursor_target(bounds, style);
        Self::dispatch_any(renderer, ctx, env, content);
    }

    fn render_gesture_observer_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<GestureObserver>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let GestureObserver {
            gesture,
            mut action,
            ..
        } = value;
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        #[cfg(feature = "accessibility")]
        if matches!(gesture, Gesture::Tap(_)) && env.get::<AccessibilityRole>().is_some() {
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Button),
            );
            let default_label = renderer.accessibility_label_from_view(&content, env);
            if let Some(label) = renderer.resolve_accessibility_label(env, default_label) {
                node.set_label(label);
            }
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            let activation_point = accessibility_activation_point(bounds);
            let _ = renderer.register_accessibility_node(
                node,
                bounds,
                env,
                Some(AccessibilityActionTarget::PointerPrimaryClick {
                    point: activation_point,
                }),
            );
        }
        let gesture_group_identity = gesture_group_identity(&content);
        let group_id = renderer.gesture_group_id_for_identity(gesture_group_identity);
        let captured_env = env.clone();
        let layered_action: BoxedAction<()> = Box::new(move |runtime_env: &Environment| {
            let action_env = captured_env.layered_on(runtime_env);
            action(&action_env);
        });
        renderer.register_gesture_target(bounds, group_id, gesture, layered_action);

        #[cfg(feature = "accessibility")]
        if env
            .get::<AccessibilityChildren>()
            .is_some_and(AccessibilityChildren::excludes_descendants)
        {
            renderer.push_accessibility_suppression();
            Self::dispatch_any(renderer, ctx, env, content);
            renderer.pop_accessibility_suppression();
            return;
        }
        Self::dispatch_any(renderer, ctx, env, content);
    }

    fn render_on_event_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<OnEvent>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let event = value.event();
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        match event {
            Event::HoverEnter => {
                let mut handler = value;
                let captured_env = env.clone();
                renderer.register_hover_enter_target(bounds, move |env| {
                    let action_env = captured_env.layered_on(env);
                    handler.handle(&action_env);
                    true
                });
            }
            Event::HoverMove => {
                let mut handler = value;
                let captured_env = env.clone();
                renderer.register_hover_move_target(bounds, move |point, env| {
                    let hover_event = HoverEvent::new(waterui_core::layout::Point::new(
                        point.x as f32 - bounds.x0 as f32,
                        point.y as f32 - bounds.y0 as f32,
                    ));
                    let hover_env = captured_env.layered_on(&env.extending(hover_event));
                    handler.handle(&hover_env);
                    true
                });
            }
            Event::HoverExit => {
                let mut handler = value;
                let captured_env = env.clone();
                renderer.register_hover_exit_target(bounds, move |env| {
                    let action_env = captured_env.layered_on(env);
                    handler.handle(&action_env);
                    true
                });
            }
            _ => panic!("hydrolysis event variant is not supported"),
        }
        Self::dispatch_any(renderer, ctx, env, content);
    }

    fn render_context_menu_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<ResolvedContextMenu>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        renderer.register_context_menu_target(bounds, value.items);
        Self::dispatch_any(renderer, ctx, env, content);
    }

    fn render_draggable_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<Draggable>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        renderer.register_draggable_target(bounds, value.data);
        Self::dispatch_any(renderer, ctx, env, content);
    }

    fn render_drop_destination_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<DropDestination>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        renderer.register_drop_destination_target(bounds, value, env);
        Self::dispatch_any(renderer, ctx, env, content);
    }

    fn render_passthrough_metadata<T: MetadataKey>(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: Metadata<T>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let _ = value;
        Self::dispatch_any(renderer, ctx, env, content);
    }

    fn render_passthrough_ignorable_metadata<T: MetadataKey>(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: IgnorableMetadata<T>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let _ = value;
        Self::dispatch_any(renderer, ctx, env, content);
    }

    fn render_accessibility_label_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityLabel>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let mut local_env = env.clone();
        local_env.insert(value);
        Self::dispatch_any(renderer, ctx, &local_env, content);
    }

    fn render_accessibility_role_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityRole>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let mut local_env = env.clone();
        local_env.insert(value);
        Self::dispatch_any(renderer, ctx, &local_env, content);
    }

    fn render_accessibility_hidden_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityHidden>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let mut local_env = env.clone();
        local_env.insert(value);
        Self::dispatch_any(renderer, ctx, &local_env, content);
    }

    fn render_accessibility_children_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityChildren>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let mut local_env = env.clone();
        local_env.insert(value);
        Self::dispatch_any(renderer, ctx, &local_env, content);
    }

    fn render_accessibility_state_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityState>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let mut local_env = env.clone();
        if value.is_hidden() {
            local_env.insert(AccessibilityHidden::new(true));
        }
        local_env.insert(value);
        Self::dispatch_any(renderer, ctx, &local_env, content);
    }

    fn render_accessibility_state_signal_metadata(
        renderer: &mut HydrolysisRenderer,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityStateSignal>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let state = value.state().get();
        let mut local_env = env.clone();
        if state.is_hidden() {
            local_env.insert(AccessibilityHidden::new(true));
        }
        local_env.insert(state);
        Self::dispatch_any(renderer, ctx, &local_env, content);
    }

    #[must_use]
    pub fn state(&self) -> &HydroState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut HydroState {
        &mut self.state
    }

    pub(crate) fn state_and_scene_mut(&mut self) -> (&mut HydroState, &mut vello::Scene) {
        (&mut self.state, &mut self.scene)
    }

    pub(crate) fn table_slot_and_state_mut(
        &mut self,
        index: usize,
    ) -> (&mut crate::renderer::lazy::LazyTableSlot, &mut HydroState) {
        (
            &mut self.lazy.lazy_table_controller.slots[index],
            &mut self.state,
        )
    }

    #[must_use]
    pub fn scene(&self) -> &vello::Scene {
        &self.scene
    }

    pub fn reset_scene(&mut self) {
        for image in self.compositor.active_filter_images.drain(..) {
            self.vello_renderer.unregister_texture(image);
        }
        self.hit_test.reset_scene();
        self.gesture_engine.clear_targets();
        self.text_editing.text_input_targets.clear();
        self.scene.reset();
        self.compositor.render_layers.clear();
        self.compositor.active_scene_layers.clear();
        self.state.measurement_cache_hits = 0;
        self.state.measurement_cache_misses = 0;
        self.frame_clip_layers = 0;
        self.frame_max_clip_depth = 0;
        self.frame_applied_filter_count = 0;
        self.frame_applied_filter_capture = Duration::ZERO;
        self.frame_applied_filter_effect = Duration::ZERO;
        #[cfg(feature = "accessibility")]
        self.accessibility.reset_scene();
    }

    pub fn begin_rebuild_frame(&mut self) {
        self.rebuild_in_progress.set(true);
        if !self.reuse_scroll_content_caches {
            self.scroll_content_caches.clear();
        }
        self.retained_window_frame = None;
        // A full rebuild re-dispatches every Dynamic node, so any pending isolated
        // reactive patch is subsumed by it.
        self.patch_requested.set(false);
        self.dirty_dynamic_nodes.borrow_mut().clear();
        self.state.measurement_cache.clear();
        self.state.measurement_cache_hits = 0;
        self.state.measurement_cache_misses = 0;
        self.frame_clip_layers = 0;
        self.frame_max_clip_depth = 0;
        self.frame_applied_filter_count = 0;
        self.frame_applied_filter_capture = Duration::ZERO;
        self.frame_applied_filter_effect = Duration::ZERO;
        self.active_applied_filter_cursor = 0;
        self.rebuild_generation.set(
            self.rebuild_generation
                .get()
                .checked_add(1)
                .expect("hydrolysis renderer rebuild generation overflow"),
        );
        self.lifecycle.begin_rebuild_frame();
        self.hit_test.begin_rebuild_frame();
        self.gesture_group_ids.clear();
        self.next_gesture_group_id = 0;
        self.animation_controller.begin_rebuild_frame();
        self.scroll_controller.begin_rebuild_frame();
        self.lazy.begin_rebuild_frame();
        self.navigation.begin_rebuild_frame();
        self.compositor.gpu_surface_cursor = 0;
        self.compositor.render_layers.clear();
        self.compositor.active_scene_layers.clear();
        self.popup_menu.begin_rebuild_frame();
        self.text_editing.text_selection_cursor = 0;
        #[cfg(feature = "accessibility")]
        self.accessibility.begin_rebuild_frame();
    }

    pub(crate) fn set_scroll_content_cache_reuse(&mut self, reuse: bool) {
        self.reuse_scroll_content_caches = reuse;
    }

    pub(crate) fn set_applied_filter_input_cache_reuse(&mut self, reuse: bool) {
        self.reuse_applied_filter_inputs = reuse;
    }

    fn remember_active_applied_filter(
        &mut self,
        runtime: Rc<RefCell<AppliedFilterRuntime>>,
        width: u32,
        height: u32,
    ) {
        self.remember_active_applied_filter_entry(ActiveAppliedFilter {
            runtime,
            width,
            height,
        });
    }

    fn remember_active_applied_filter_entry(&mut self, active: ActiveAppliedFilter) {
        let index = self.active_applied_filter_cursor;
        self.active_applied_filter_cursor = self
            .active_applied_filter_cursor
            .checked_add(1)
            .expect("hydrolysis active AppliedFilter cursor overflow");
        if index == self.active_applied_filters.len() {
            self.active_applied_filters.push(active);
        } else {
            self.active_applied_filters[index] = active;
        }
    }

    pub(crate) fn begin_redraw_frame(&mut self) {
        self.state.measurement_cache_hits = 0;
        self.state.measurement_cache_misses = 0;
        self.frame_clip_layers = 0;
        self.frame_max_clip_depth = 0;
        self.frame_applied_filter_count = 0;
        self.frame_applied_filter_capture = Duration::ZERO;
        self.frame_applied_filter_effect = Duration::ZERO;
    }

    pub(crate) fn refresh_active_applied_filters(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let active_filters = self
            .active_applied_filters
            .iter()
            .map(|filter| (Rc::clone(&filter.runtime), filter.width, filter.height))
            .collect::<Vec<_>>();
        if active_filters.is_empty() {
            return;
        }
        let mut encoder = None;
        for (runtime, width, height) in active_filters {
            if !runtime.borrow_mut().needs_redraw_refresh() {
                continue;
            }
            let encoder = encoder.get_or_insert_with(|| {
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("hydrolysis active applied filters encoder"),
                })
            });
            let effect_started_at = Instant::now();
            let needs_redraw = runtime
                .borrow_mut()
                .encode_output(
                    device,
                    queue,
                    &mut self.vello_renderer,
                    width,
                    height,
                    encoder,
                )
                .1;
            self.frame_applied_filter_effect += effect_started_at.elapsed();
            self.frame_applied_filter_count = self
                .frame_applied_filter_count
                .checked_add(1)
                .expect("hydrolysis applied filter counter overflow");
            if needs_redraw {
                self.request_redraw();
            }
        }
        if let Some(encoder) = encoder {
            queue.submit([encoder.finish()]);
        }
    }

    pub(crate) fn invalidate_retained_scroll_content(&mut self) {
        self.scroll_content_caches.clear();
        self.retained_window_frame = None;
    }

    /// Whether the retained window frame has any active (repeating or in-flight) dynamic
    /// morph — at the window root or inside a scroll draw — so the runner keeps issuing
    /// parametric refreshes to advance the morph animation.
    pub(crate) fn window_dynamic_morphs_active(&self) -> bool {
        let Some(frame) = &self.retained_window_frame else {
            return false;
        };
        if frame
            .content_morphs
            .iter()
            .any(|draw| self.dynamic_morph_is_active(draw))
        {
            return true;
        }
        self.subtree_scroll_morphs_active(&frame.subtree)
    }

    fn subtree_scroll_morphs_active(&self, subtree: &DynamicSubtree) -> bool {
        subtree.dynamic_scroll_draws.iter().any(|draw| {
            draw.content_morphs
                .iter()
                .any(|morph| self.dynamic_morph_is_active(morph))
        })
    }

    pub(crate) fn active_scene_layers_snapshot(&self) -> Vec<ActiveSceneLayer> {
        self.compositor.active_scene_layers.clone()
    }

    pub(crate) fn scene_is_empty(&self) -> bool {
        self.scene.encoding().is_empty()
    }

    pub(crate) fn viewport_matches_window_bounds(&self, viewport: vello::kurbo::Rect) -> bool {
        (viewport.x0 - self.window_bounds.x0).abs() <= f64::EPSILON
            && (viewport.y0 - self.window_bounds.y0).abs() <= f64::EPSILON
            && (viewport.x1 - self.window_bounds.x1).abs() <= f64::EPSILON
            && (viewport.y1 - self.window_bounds.y1).abs() <= f64::EPSILON
    }

    pub(crate) fn push_dynamic_scroll_draw(&mut self, draw: DynamicScrollDraw) {
        self.dynamic_scroll_draws.push(draw);
    }

    /// Composites each scroll view from its retained offset-independent content cache,
    /// applying the current scroll offset, viewport clip, content morphs, scroll target,
    /// accessibility node, and indicators. This is the per-frame body of the former
    /// `refresh_retained_scroll_scene`, generalized to run inside the window-frame replay
    /// for any number of (possibly nested) scroll views.
    fn replay_dynamic_scroll_draws(
        &mut self,
        parent_ctx: RenderContext,
        draws: &[DynamicScrollDraw],
    ) {
        for draw in draws {
            let transform = parent_ctx.transform * draw.base_transform;
            let hit_transform = parent_ctx.hit_transform * draw.base_hit_transform;
            if draw.needs_viewport_clip {
                self.record_clip_layer_push();
                self.scene.push_layer(
                    vello::peniko::Fill::NonZero,
                    vello::peniko::BlendMode::default(),
                    1.0,
                    transform,
                    &draw.viewport,
                );
                self.compositor.active_scene_layers.push(ActiveSceneLayer {
                    alpha: 1.0,
                    transform,
                    shape: LayerShape::Rect(draw.viewport),
                });
            }
            let metrics = draw.handle.metrics();
            let scroll_content_transform =
                vello::kurbo::Affine::translate((-metrics.offset_x, -metrics.offset_y));
            let content_transform = transform * scroll_content_transform;
            let content_hit_transform = hit_transform * scroll_content_transform;
            let content_bounds =
                vello::kurbo::Rect::new(0.0, 0.0, draw.content_width, draw.content_height);
            let content_ctx = RenderContext::with_transforms(
                content_bounds,
                content_transform,
                content_hit_transform,
            );
            if let Some(cache) = self.scroll_content_caches.remove(&draw.cache_key) {
                self.replay_dynamic_subtree(content_ctx, &cache.subtree);
                self.scroll_content_caches.insert(draw.cache_key, cache);
            }
            self.draw_dynamic_morphs(&draw.content_morphs, content_transform);
            if draw.needs_viewport_clip {
                self.pop_layer();
            }
            let target_handle = draw.handle.clone();
            self.register_scroll_target(
                transformed_rect(hit_transform, draw.viewport),
                move |dx, dy, is_line_delta| target_handle.apply_scroll_delta(dx, dy, is_line_delta),
            );
            crate::widgets::scroll::register_scroll_accessibility_node(
                self,
                &draw.env,
                transformed_rect(hit_transform, draw.viewport),
                &draw.handle,
                metrics,
                draw.axis,
            );
            let scroll_ctx =
                RenderContext::with_transforms(draw.viewport, transform, hit_transform);
            let mut widget_ctx = WidgetRenderContext::new(self, scroll_ctx);
            crate::widgets::draw_scroll_indicators(
                &mut widget_ctx,
                &draw.env,
                draw.viewport,
                metrics,
                draw.axis,
            );
        }
    }

    /// Whether every scroll view reachable from the retained window frame can be
    /// re-composited from its cached content at the current scroll offset. Lazy
    /// (viewport-dependent) content that scrolled beyond its captured window returns
    /// false, forcing a structural rebuild that re-materializes the visible items.
    pub(crate) fn window_scroll_draws_reusable(&self) -> bool {
        match &self.retained_window_frame {
            Some(frame) => self.subtree_scroll_draws_reusable(&frame.subtree),
            None => true,
        }
    }

    fn subtree_scroll_draws_reusable(&self, subtree: &DynamicSubtree) -> bool {
        for draw in &subtree.dynamic_scroll_draws {
            let metrics = draw.handle.metrics();
            let lazy_viewport = vello::kurbo::Rect::new(
                metrics.offset_x,
                metrics.offset_y,
                metrics.offset_x + draw.viewport.width(),
                metrics.offset_y + draw.viewport.height(),
            );
            let Some(cache) = self.scroll_content_caches.get(&draw.cache_key) else {
                return false;
            };
            if !self.can_reuse_scroll_content_cache(cache, lazy_viewport)
                || !self.subtree_scroll_draws_reusable(&cache.subtree)
            {
                return false;
            }
        }
        for placement in &subtree.dynamic_node_draws {
            if let Some(cached) = self
                .lifecycle
                .dynamic_nodes
                .get(&placement.identity)
                .and_then(|node| node.cached_subtree.as_ref())
                && !self.subtree_scroll_draws_reusable(cached)
            {
                return false;
            }
        }
        for transform in &subtree.dynamic_transforms {
            if !self.subtree_scroll_draws_reusable(&transform.subtree) {
                return false;
            }
        }
        for opacity in &subtree.dynamic_opacities {
            if !self.subtree_scroll_draws_reusable(&opacity.subtree) {
                return false;
            }
        }
        true
    }

    fn mark_scroll_content_viewport_dependent(&mut self) {
        if self.scroll_content_capture_depth > 0 {
            self.scroll_content_viewport_dependent = true;
        }
    }

    fn mark_scroll_content_animation_dependent(&mut self) {
        if self.scroll_content_capture_depth > 0 {
            self.scroll_content_animation_dependent = true;
        }
    }

    fn can_reuse_scroll_content_cache(
        &self,
        cache: &ScrollContentCache,
        lazy_viewport: vello::kurbo::Rect,
    ) -> bool {
        let viewport_reusable =
            !cache.viewport_dependent || rect_near(cache.lazy_viewport, lazy_viewport);
        let animation_reusable = !cache.animation_dependent || !self.animations_active();
        viewport_reusable && animation_reusable
    }

    pub(crate) fn render_scroll_content(
        &mut self,
        cache_key: usize,
        lazy_viewport: vello::kurbo::Rect,
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
    ) -> ScrollContentRender {
        if self.reuse_scroll_content_caches
            && let Some(cache) = self.scroll_content_caches.remove(&cache_key)
        {
            if self.can_reuse_scroll_content_cache(&cache, lazy_viewport) {
                // Capture-only: the content is composited by the scroll draw at replay,
                // not baked into the parent scene here. Re-register applied filters since
                // no dispatch happened to advance them this frame.
                let dynamic_morphs = cache.dynamic_morphs.clone();
                for active_filter in cache.active_filters.iter().cloned() {
                    self.remember_active_applied_filter_entry(active_filter);
                }
                self.scroll_content_caches.insert(cache_key, cache);
                return ScrollContentRender { dynamic_morphs };
            }
            self.scroll_content_caches.insert(cache_key, cache);
        }

        let local_ctx = ctx.with_identity_transforms(ctx.bounds);
        let active_filter_start = self.active_applied_filter_cursor;
        let previous_morphs = core::mem::take(&mut self.dynamic_morph_draws);
        let previous_scroll_content_viewport_dependent = self.scroll_content_viewport_dependent;
        let previous_scroll_content_animation_dependent = self.scroll_content_animation_dependent;
        self.scroll_content_viewport_dependent = false;
        self.scroll_content_animation_dependent = false;
        self.scroll_content_capture_depth = self
            .scroll_content_capture_depth
            .checked_add(1)
            .expect("hydrolysis scroll content capture depth overflow");
        self.dynamic_morph_capture_depth = self
            .dynamic_morph_capture_depth
            .checked_add(1)
            .expect("hydrolysis dynamic morph capture depth overflow");
        self.dynamic_transform_capture_depth = self
            .dynamic_transform_capture_depth
            .checked_add(1)
            .expect("hydrolysis dynamic transform capture depth overflow");
        let subtree = Self::render_dynamic_subtree_with_local_interactions(
            self, ctx, local_ctx, env, content,
        );
        self.dynamic_transform_capture_depth = self
            .dynamic_transform_capture_depth
            .checked_sub(1)
            .expect("hydrolysis dynamic transform capture depth underflow");
        self.dynamic_morph_capture_depth = self
            .dynamic_morph_capture_depth
            .checked_sub(1)
            .expect("hydrolysis dynamic morph capture depth underflow");
        self.scroll_content_capture_depth = self
            .scroll_content_capture_depth
            .checked_sub(1)
            .expect("hydrolysis scroll content capture depth underflow");
        let viewport_dependent = self.scroll_content_viewport_dependent;
        let animation_dependent = self.scroll_content_animation_dependent;
        self.scroll_content_viewport_dependent = previous_scroll_content_viewport_dependent;
        self.scroll_content_animation_dependent = previous_scroll_content_animation_dependent;
        let dynamic_morphs = core::mem::replace(&mut self.dynamic_morph_draws, previous_morphs);
        // Capture-only: do not bake content into the parent scene; the scroll draw
        // composites it from the cache at replay, applying the current scroll offset.
        let active_filters = self.active_applied_filters
            [active_filter_start..self.active_applied_filter_cursor]
            .to_vec();
        self.scroll_content_caches.insert(
            cache_key,
            ScrollContentCache {
                lazy_viewport,
                viewport_dependent,
                animation_dependent,
                subtree,
                active_filters,
                dynamic_morphs: dynamic_morphs.clone(),
            },
        );
        ScrollContentRender { dynamic_morphs }
    }

    /// Dispatches the whole window content while capturing it as a retained,
    /// replayable [`DynamicSubtree`], then renders this frame by replaying that
    /// capture. Animated transforms and morphs are captured as replayable dynamic
    /// draws (not baked), so later animation-only frames can refresh via
    /// [`Self::refresh_window_frame`] without re-walking or re-measuring the view tree.
    ///
    /// The subtree is captured in real (DPI-scaled) coordinates so it replays under an
    /// identity context; this keeps any nested scroll retention working in real space.
    pub fn capture_window_scene<V: View>(
        &mut self,
        view: V,
        env: &Environment,
        bounds: vello::kurbo::Rect,
        transform: vello::kurbo::Affine,
        hit_transform: vello::kurbo::Affine,
    ) {
        self.retained_window_frame = None;
        #[cfg(feature = "accessibility")]
        {
            self.accessibility.root_bounds = transformed_rect(hit_transform, bounds);
        }
        let local_env = self.lifecycle.install_local_state_env(env);
        let ctx = RenderContext::with_transforms(bounds, transform, hit_transform);
        self.render_depth = 0;

        let gpu_surface_cursor_start = self.compositor.gpu_surface_cursor;
        let active_filter_start = self.active_applied_filter_cursor;
        let previous_morphs = core::mem::take(&mut self.dynamic_morph_draws);
        let previous_viewport_dependent = self.scroll_content_viewport_dependent;
        let previous_animation_dependent = self.scroll_content_animation_dependent;
        self.scroll_content_viewport_dependent = false;
        self.scroll_content_animation_dependent = false;
        self.scroll_content_capture_depth = self
            .scroll_content_capture_depth
            .checked_add(1)
            .expect("hydrolysis window scene capture depth overflow");
        self.dynamic_morph_capture_depth = self
            .dynamic_morph_capture_depth
            .checked_add(1)
            .expect("hydrolysis window morph capture depth overflow");
        self.dynamic_transform_capture_depth = self
            .dynamic_transform_capture_depth
            .checked_add(1)
            .expect("hydrolysis window transform capture depth overflow");
        let subtree = Self::render_dynamic_subtree(self, ctx, &local_env, AnyView::new(view));
        self.dynamic_transform_capture_depth = self
            .dynamic_transform_capture_depth
            .checked_sub(1)
            .expect("hydrolysis window transform capture depth underflow");
        self.dynamic_morph_capture_depth = self
            .dynamic_morph_capture_depth
            .checked_sub(1)
            .expect("hydrolysis window morph capture depth underflow");
        self.scroll_content_capture_depth = self
            .scroll_content_capture_depth
            .checked_sub(1)
            .expect("hydrolysis window scene capture depth underflow");
        let animation_dependent = self.scroll_content_animation_dependent;
        self.scroll_content_viewport_dependent = previous_viewport_dependent;
        self.scroll_content_animation_dependent = previous_animation_dependent;
        let content_morphs = core::mem::replace(&mut self.dynamic_morph_draws, previous_morphs);

        let used_gpu_surface = self.compositor.gpu_surface_cursor != gpu_surface_cursor_start;
        let used_applied_filter = self.active_applied_filter_cursor != active_filter_start;
        let drivable = !animation_dependent && !used_gpu_surface && !used_applied_filter;
        let active_layers = self.active_scene_layers_snapshot();

        // Replay immediately so this structural frame renders pixels identical to a
        // direct dispatch. Captured in real coordinates, so replay uses identity.
        let replay_ctx = RenderContext::with_transforms(
            bounds,
            vello::kurbo::Affine::IDENTITY,
            vello::kurbo::Affine::IDENTITY,
        );
        self.replay_dynamic_subtree(replay_ctx, &subtree);
        self.draw_dynamic_morphs(&content_morphs, vello::kurbo::Affine::IDENTITY);

        self.retained_window_frame = Some(RetainedWindowFrame {
            subtree,
            transform,
            bounds,
            active_layers,
            content_morphs,
            drivable,
        });
    }

    /// Whether the retained window frame can re-render active animations by pure
    /// replay this frame (no structural rebuild). Mirrors
    /// [`Self::retained_scroll_can_drive_active_animations`] but for non-scroll roots.
    pub(crate) fn retained_window_can_drive_active_animations(&self) -> bool {
        let Some(frame) = &self.retained_window_frame else {
            return false;
        };
        if !frame.drivable {
            return false;
        }
        if self.navigation.slots.iter().any(|slot| {
            slot.transition
                .as_ref()
                .is_some_and(|state| state.is_active(self.frame_instant))
        }) {
            return false;
        }
        if self.animation_controller.has_active_radio_indicator() {
            return false;
        }
        // All animations driving this frame must be captured as replayable dynamic
        // draws (transform/opacity; renderer-local interaction scalars replay too).
        // Any active top-level scalar that is not captured would render stale.
        let mut retained_scalar_keys = BTreeSet::new();
        self.collect_subtree_active_scalar_keys(&frame.subtree, &mut retained_scalar_keys);
        let active_scalar_keys: BTreeSet<_> = self
            .animation_controller
            .active_scalar_keys()
            .into_iter()
            .filter(|key| !key.is_renderer_local_scalar())
            .collect();
        active_scalar_keys
            .iter()
            .all(|key| retained_scalar_keys.contains(key))
    }

    /// Re-renders the retained window frame by replaying its captured subtree at the
    /// current frame instant — re-sampling animated transforms and morphs — without
    /// re-dispatching or re-measuring. Returns `false` when the frame cannot be driven
    /// by replay, in which case the caller must fall back to a structural rebuild.
    pub(crate) fn refresh_window_frame(&mut self, env: &Environment) -> bool {
        if self.retained_window_frame.is_none() {
            return false;
        }
        // Apply any pending fine-grained reactive patches before compositing. If a patch
        // reflowed layout it escalates to a full rebuild, so bail to the rebuild path.
        if !self.patch_dirty_dynamic_nodes() {
            return false;
        }
        // A scroll that moved a lazy (viewport-dependent) list beyond its captured window
        // cannot be re-composited from the cache; escalate to a rebuild that re-materializes.
        if !self.window_scroll_draws_reusable() {
            return false;
        }
        let Some(frame) = self.retained_window_frame.take() else {
            return false;
        };
        // A frame that baked an animated non-transform value can only be replayed safely
        // while no animation is active; otherwise the baked value would be stale.
        if !frame.drivable && self.animations_active() {
            self.retained_window_frame = Some(frame);
            return false;
        }
        self.reset_scene();
        #[cfg(feature = "accessibility")]
        self.accessibility.begin_rebuild_frame();
        let background_color =
            resolved_color_to_peniko(Color::new(theme::color::Background).resolve(env).get());
        self.scene.fill(
            vello::peniko::Fill::NonZero,
            frame.transform,
            background_color,
            None,
            &self.window_bounds,
        );
        for layer in &frame.active_layers {
            layer.push_to_scene(&mut self.scene);
            self.compositor.active_scene_layers.push(layer.clone());
        }
        let replay_ctx = RenderContext::with_transforms(
            frame.bounds,
            vello::kurbo::Affine::IDENTITY,
            vello::kurbo::Affine::IDENTITY,
        );
        self.replay_dynamic_subtree(replay_ctx, &frame.subtree);
        self.draw_dynamic_morphs(&frame.content_morphs, vello::kurbo::Affine::IDENTITY);
        while !self.compositor.active_scene_layers.is_empty() {
            self.pop_layer();
        }
        #[cfg(feature = "accessibility")]
        self.finalize_accessibility_tree_update();
        self.flush_vello_scene_layer();
        self.retained_window_frame = Some(frame);
        true
    }

    pub fn finish_rebuild_frame(&mut self) {
        assert!(
            self.compositor.active_scene_layers.is_empty(),
            "hydrolysis renderer: scene layer stack must be empty at end of rebuild (len={})",
            self.compositor.active_scene_layers.len()
        );
        self.flush_vello_scene_layer();
        self.lifecycle
            .finish_rebuild_frame(&mut self.state, self.reuse_scroll_content_caches);

        if matches!(
            self.text_editing.focused_text_input.get(),
            Some(index) if index >= self.text_editing.text_input_targets.len()
        ) {
            self.set_focused_text_input(None);
        }
        if matches!(
            self.text_editing.active_text_selection_drag,
            Some(index) if index >= self.text_editing.text_input_targets.len()
        ) {
            self.text_editing.active_text_selection_drag = None;
        }

        self.animation_controller
            .finish_rebuild_frame_with_inactive_slot_retention(self.reuse_scroll_content_caches);
        self.scroll_controller.finish_rebuild_frame();
        self.hit_test.finish_rebuild_frame();
        self.lazy.finish_rebuild_frame();
        self.navigation.finish_rebuild_frame();
        self.compositor
            .gpu_surface_slots
            .truncate(self.compositor.gpu_surface_cursor);
        self.active_applied_filters
            .truncate(self.active_applied_filter_cursor);
        self.popup_menu.finish_rebuild_frame();
        self.text_editing
            .text_selection_slots
            .truncate(self.text_editing.text_selection_cursor);
        self.rebuild_in_progress.set(false);
        #[cfg(feature = "accessibility")]
        self.finalize_accessibility_tree_update();
    }

    pub fn scene_mut(&mut self) -> &mut vello::Scene {
        &mut self.scene
    }

    pub(crate) fn draw_context(&mut self, ctx: RenderContext) -> VelloDrawContext<'_> {
        VelloDrawContext::with_root_transform(&mut self.scene, ctx.transform)
    }

    pub fn vello_renderer(&mut self) -> &mut vello::Renderer {
        &mut self.vello_renderer
    }

    pub fn set_frame_resources(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.state.set_frame_resources(device, queue);
    }

    pub fn clear_frame_resources(&mut self) {
        self.state.clear_frame_resources();
    }

    fn bind_gpu_surface_slot(&mut self, surface: GpuSurface, env: &Environment) -> usize {
        let index = self.compositor.gpu_surface_cursor;
        self.compositor.gpu_surface_cursor = self
            .compositor
            .gpu_surface_cursor
            .checked_add(1)
            .expect("hydrolysis gpu surface slot cursor overflow");

        if index == self.compositor.gpu_surface_slots.len() {
            self.compositor
                .gpu_surface_slots
                .push(EmbeddedGpuSurfaceRuntime::new(surface, env));
        } else {
            self.compositor.gpu_surface_slots[index].replace_surface(surface, env);
        }

        index
    }

    pub(crate) fn push_layer_rect(
        &mut self,
        alpha: f32,
        transform: vello::kurbo::Affine,
        rect: vello::kurbo::Rect,
    ) {
        self.record_clip_layer_push();
        self.scene.push_layer(
            vello::peniko::Fill::NonZero,
            vello::peniko::BlendMode::default(),
            alpha,
            transform,
            &rect,
        );
        self.compositor.active_scene_layers.push(ActiveSceneLayer {
            alpha,
            transform,
            shape: LayerShape::Rect(rect),
        });
    }

    fn push_layer_path(
        &mut self,
        alpha: f32,
        transform: vello::kurbo::Affine,
        path: vello::kurbo::BezPath,
    ) {
        self.record_clip_layer_push();
        self.scene.push_layer(
            vello::peniko::Fill::NonZero,
            vello::peniko::BlendMode::default(),
            alpha,
            transform,
            &path,
        );
        self.compositor.active_scene_layers.push(ActiveSceneLayer {
            alpha,
            transform,
            shape: LayerShape::Path(path),
        });
    }

    pub(crate) fn pop_layer(&mut self) {
        self.scene.pop_layer();
        self.compositor
            .active_scene_layers
            .pop()
            .expect("hydrolysis renderer: pop_layer underflow");
    }

    fn record_clip_layer_push(&mut self) {
        self.frame_clip_layers = self
            .frame_clip_layers
            .checked_add(1)
            .expect("hydrolysis frame clip layer counter overflow");
        let depth = u32::try_from(self.compositor.active_scene_layers.len() + 1)
            .expect("hydrolysis active scene layer depth exceeds u32");
        self.frame_max_clip_depth = self.frame_max_clip_depth.max(depth);
    }

    fn flush_vello_scene_layer(&mut self) {
        assert!(
            (self.scene.encoding().n_open_clips as usize)
                == self.compositor.active_scene_layers.len(),
            "hydrolysis renderer: scene clip count {} does not match tracked scene layers {}",
            self.scene.encoding().n_open_clips,
            self.compositor.active_scene_layers.len()
        );

        for _ in 0..self.compositor.active_scene_layers.len() {
            self.scene.pop_layer();
        }

        if self.scene.encoding().is_empty() {
            for layer in &self.compositor.active_scene_layers {
                layer.push_to_scene(&mut self.scene);
            }
            return;
        }
        let scene = core::mem::take(&mut self.scene);
        self.compositor
            .render_layers
            .push(RenderLayer::Vello(scene));

        for layer in &self.compositor.active_scene_layers {
            layer.push_to_scene(&mut self.scene);
        }
    }

    fn push_gpu_surface_layer(
        &mut self,
        slot_index: usize,
        transform: vello::kurbo::Affine,
        bounds: vello::kurbo::Rect,
    ) {
        if self
            .compositor
            .active_scene_layers
            .iter()
            .any(|layer| layer.alpha <= HIT_TEST_ALPHA_THRESHOLD)
        {
            return;
        }

        self.flush_vello_scene_layer();
        let direct_to_target = self.compositor.render_layers.is_empty()
            && self.compositor.active_scene_layers.is_empty()
            && affine_near(transform, vello::kurbo::Affine::IDENTITY)
            && rect_near(bounds, self.window_bounds);
        self.compositor
            .render_layers
            .push(RenderLayer::GpuSurface(GpuSurfaceLayer {
                slot_index,
                transform,
                bounds,
                active_layers: self.compositor.active_scene_layers.clone(),
                direct_to_target,
            }));
    }

    pub fn poll_gpu_surface_redraw_handles(&mut self) -> bool {
        let mut requested = false;
        for runtime in &self.compositor.gpu_surface_slots {
            if runtime.take_external_redraw_request() {
                requested = true;
            }
        }
        if requested {
            self.redraw_requested.set(true);
        }
        requested
    }

    pub fn request_redraw(&self) {
        self.redraw_requested.set(true);
    }

    pub fn take_redraw_request(&self) -> bool {
        self.redraw_requested.replace(false)
    }

    pub fn request_rebuild(&self) {
        self.rebuild_requested.set(true);
    }

    #[must_use]
    pub fn has_rebuild_request(&self) -> bool {
        self.rebuild_requested.get()
    }

    pub fn request_next_frame_rebuild(&self) {
        self.next_frame_rebuild_requested.set(true);
        self.redraw_requested.set(true);
    }

    #[must_use]
    pub fn rebuild_handle(&self) -> Rc<Cell<bool>> {
        Rc::clone(&self.rebuild_requested)
    }

    pub fn take_rebuild_request(&self) -> bool {
        self.rebuild_requested.replace(false)
    }

    #[must_use]
    pub fn has_patch_request(&self) -> bool {
        self.patch_requested.get()
    }

    pub fn take_patch_request(&self) -> bool {
        self.patch_requested.replace(false)
    }

    #[must_use]
    pub fn has_retained_window_frame(&self) -> bool {
        self.retained_window_frame.is_some()
    }

    pub fn take_next_frame_rebuild_request(&self) -> bool {
        self.next_frame_rebuild_requested.replace(false)
    }

    pub(crate) fn measurement_cache_stats(&self) -> (u32, u32) {
        (
            self.state.measurement_cache_hits,
            self.state.measurement_cache_misses,
        )
    }

    pub(crate) fn render_layer_stats(&self) -> (u32, u32, u32) {
        let scene_layers = u32::try_from(self.compositor.render_layers.len())
            .expect("hydrolysis render layer count exceeds u32");
        let vello_scene_layers = u32::try_from(
            self.compositor
                .render_layers
                .iter()
                .filter(|layer| matches!(layer, RenderLayer::Vello(_)))
                .count(),
        )
        .expect("hydrolysis Vello scene layer count exceeds u32");
        let direct_gpu_surfaces = u32::try_from(
            self.compositor
                .render_layers
                .iter()
                .filter(|layer| {
                    matches!(
                        layer,
                        RenderLayer::GpuSurface(GpuSurfaceLayer {
                            direct_to_target: true,
                            ..
                        })
                    )
                })
                .count(),
        )
        .expect("hydrolysis direct GpuSurface count exceeds u32");
        let gpu_surface_layers = scene_layers
            .checked_sub(vello_scene_layers)
            .and_then(|count| count.checked_sub(direct_gpu_surfaces))
            .expect("hydrolysis render layer count accounting underflow");
        let composited_scene_layers = scene_layers
            .checked_sub(direct_gpu_surfaces)
            .expect("hydrolysis render layer count accounting underflow");
        (
            composited_scene_layers,
            vello_scene_layers,
            gpu_surface_layers,
        )
    }

    pub(crate) fn clip_layer_stats(&self) -> (u32, u32) {
        (self.frame_clip_layers, self.frame_max_clip_depth)
    }

    pub(crate) fn applied_filter_stats(&self) -> (u32, u64, u64) {
        (
            self.frame_applied_filter_count,
            duration_micros_u64(self.frame_applied_filter_capture),
            duration_micros_u64(self.frame_applied_filter_effect),
        )
    }

    #[must_use]
    pub fn focused_text_input_state(&self) -> Option<TextInputState> {
        let index = self.text_editing.focused_text_input.get()?;
        let target = self.text_editing.text_input_targets.as_slice().get(index)?;
        Some(TextInputState {
            x: target.cursor_area.x0,
            y: target.cursor_area.y0,
            width: target.cursor_area.width().max(1.0),
            height: target.cursor_area.height().max(1.0),
            purpose: target.purpose,
        })
    }

    #[cfg(feature = "accessibility")]
    #[must_use]
    pub fn focused_ui_node(&self) -> Option<AccessibilityNodeId> {
        self.focused_text_input_accessibility_node()
    }

    pub fn clear_ui_focus(&mut self) -> bool {
        self.set_focused_text_input(None)
    }

    #[must_use]
    pub fn cursor_style_at(&self, x: f32, y: f32) -> CursorStyle {
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        self.hit_test.cursor_style_at(point)
    }

    pub fn advance_animations(&mut self) -> bool {
        let now = self.frame_instant;
        self.animation_controller.tick(now)
            || self.navigation.slots.iter().any(|slot| {
                slot.transition
                    .as_ref()
                    .is_some_and(|state| state.is_active(now))
            })
    }

    pub fn animations_active(&self) -> bool {
        let now = self.frame_instant;
        self.animation_controller.has_active(now)
            || self.navigation.slots.iter().any(|slot| {
                slot.transition
                    .as_ref()
                    .is_some_and(|state| state.is_active(now))
            })
    }

    pub fn dispatch<V: View>(&mut self, view: V, env: &Environment, bounds: vello::kurbo::Rect) {
        self.dispatch_with_transform(
            view,
            env,
            bounds,
            vello::kurbo::Affine::IDENTITY,
            vello::kurbo::Affine::IDENTITY,
        );
    }

    pub fn dispatch_with_transform<V: View>(
        &mut self,
        view: V,
        env: &Environment,
        bounds: vello::kurbo::Rect,
        transform: vello::kurbo::Affine,
        hit_transform: vello::kurbo::Affine,
    ) {
        #[cfg(feature = "accessibility")]
        {
            self.accessibility.root_bounds = transformed_rect(hit_transform, bounds);
        }
        let local_env = self.lifecycle.install_local_state_env(env);
        let ctx = RenderContext::with_transforms(bounds, transform, hit_transform);
        self.render_depth = 0;
        self.dispatch_with_render_depth(view, &local_env, ctx);
    }

    pub fn handle_magnification(
        &mut self,
        x: f32,
        y: f32,
        delta: f32,
        phase: TouchPhase,
        env: &Environment,
    ) -> bool {
        let center = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        let at = self.frame_instant;
        self.gesture_engine
            .handle_magnification(center, delta, phase, at, env)
    }

    pub fn apply_magnification_gesture(
        &mut self,
        x: f32,
        y: f32,
        factor: f32,
        env: &Environment,
    ) -> bool {
        assert!(
            factor.is_finite() && factor > 0.0,
            "hydrolysis magnification factor must be finite and positive"
        );
        let mut changed = self.handle_magnification(x, y, 0.0, TouchPhase::Started, env);
        changed |= self.handle_magnification(x, y, factor - 1.0, TouchPhase::Moved, env);
        changed |= self.handle_magnification(x, y, 0.0, TouchPhase::Ended, env);
        changed
    }

    pub fn handle_rotation(
        &mut self,
        x: f32,
        y: f32,
        delta: f32,
        phase: TouchPhase,
        env: &Environment,
    ) -> bool {
        let center = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        let at = self.frame_instant;
        self.gesture_engine
            .handle_rotation(center, delta, phase, at, env)
    }

    pub fn handle_gesture_tick(&mut self, at: Instant, env: &Environment) -> bool {
        self.gesture_engine.handle_tick(at, env)
    }

    pub fn next_gesture_deadline(&self) -> Option<Instant> {
        let gesture_deadline = self.gesture_engine.next_deadline();
        let caret_deadline = self
            .text_editing
            .focused_text_input
            .get()
            .and(self.text_editing.text_caret_next_frame_at);
        match (gesture_deadline, caret_deadline) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (Some(left), None) => Some(left),
            (None, Some(right)) => Some(right),
            (None, None) => None,
        }
    }

    pub fn sync_active_interactions_after_layout(&mut self, pointer: Option<(f32, f32)>) {
        let pointer = pointer.map(|(x, y)| vello::kurbo::Point::new(f64::from(x), f64::from(y)));
        self.gesture_engine.sync_after_layout(pointer);
        self.sync_active_pointer_drag_target_after_layout(pointer);
    }

    fn register_gesture_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        group_id: usize,
        gesture: Gesture,
        action: BoxedAction<()>,
    ) {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let order = self.next_hit_test_order();
        self.gesture_engine.register_target(
            bounds,
            gesture,
            action,
            self.render_depth,
            order,
            group_id,
        );
    }

    fn register_gesture_target_recognizer(
        &mut self,
        bounds: vello::kurbo::Rect,
        target: GestureTarget,
        depth: usize,
        group_id: usize,
    ) {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        self.gesture_engine
            .register_existing_target(target.with_bounds_depth_and_group(bounds, depth, group_id));
    }

    fn allocate_gesture_group_id(&mut self) -> usize {
        let group_id = self.next_gesture_group_id;
        self.next_gesture_group_id = self
            .next_gesture_group_id
            .checked_add(1)
            .expect("hydrolysis gesture group id overflow");
        group_id
    }

    fn gesture_group_id_for_identity(&mut self, identity: usize) -> usize {
        if let Some(group_id) = self.gesture_group_ids.get(&identity).copied() {
            return group_id;
        }
        let group_id = self.allocate_gesture_group_id();
        self.gesture_group_ids.insert(identity, group_id);
        group_id
    }

    pub(crate) fn register_text_input_target(&mut self, target: TextInputTargetRegistration) {
        #[cfg(feature = "accessibility")]
        let accessibility_node_id = self.take_pending_text_input_accessibility_node();
        self.register_text_input_target_data(text_editing::TextInputTargetData {
            target,
            depth: self.render_depth,
            focus_binding: None,
            #[cfg(feature = "accessibility")]
            accessibility_node_id,
        });
    }

    fn register_text_input_target_data(&mut self, data: text_editing::TextInputTargetData) {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let order = self.next_hit_test_order();
        self.text_editing.text_input_targets.push(TextInputTarget {
            bounds: data.target.bounds,
            cursor_area: data.target.cursor_area,
            text_bounds: data.target.text_bounds,
            text_clip_bounds: data.target.text_clip_bounds,
            content_alpha: data.target.content_alpha,
            layout: data.target.layout,
            purpose: data.target.purpose,
            depth: data.depth,
            order,
            model: data.target.model,
            selection: data.target.selection,
            focus_binding: data.focus_binding,
            #[cfg(feature = "accessibility")]
            accessibility_node_id: data.accessibility_node_id,
        });
    }
}

fn duration_micros_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn affine_near(left: vello::kurbo::Affine, right: vello::kurbo::Affine) -> bool {
    left.as_coeffs()
        .iter()
        .zip(right.as_coeffs())
        .all(|(left, right)| (*left - right).abs() <= 0.001)
}

fn rect_near(left: vello::kurbo::Rect, right: vello::kurbo::Rect) -> bool {
    (left.x0 - right.x0).abs() <= 0.001
        && (left.y0 - right.y0).abs() <= 0.001
        && (left.x1 - right.x1).abs() <= 0.001
        && (left.y1 - right.y1).abs() <= 0.001
}

fn color_to_wgpu(color: vello::peniko::Color) -> wgpu::Color {
    let linear = ResolvedColor::from_srgb(Srgb::new(
        color.components[0],
        color.components[1],
        color.components[2],
    ))
    .linear_with_headroom();
    wgpu::Color {
        r: f64::from(linear[0]),
        g: f64::from(linear[1]),
        b: f64::from(linear[2]),
        a: f64::from(color.components[3]),
    }
}

impl Drop for HydrolysisRenderer {
    fn drop(&mut self) {
        self.lifecycle.drop_all_hooks();
    }
}

#[cfg(feature = "accessibility")]
fn kurbo_rect_to_accesskit_rect(rect: vello::kurbo::Rect) -> AccessibilityRect {
    AccessibilityRect {
        x0: rect.x0,
        y0: rect.y0,
        x1: rect.x1,
        y1: rect.y1,
    }
}

#[cfg(feature = "accessibility")]
fn accesskit_rect_to_kurbo_rect(rect: AccessibilityRect) -> vello::kurbo::Rect {
    vello::kurbo::Rect::new(rect.x0, rect.y0, rect.x1, rect.y1)
}

#[cfg(feature = "accessibility")]
#[derive(Clone, Copy)]
pub(crate) struct AccessibilityNodeIdRemap {
    first_mapped: u64,
}

#[cfg(feature = "accessibility")]
impl AccessibilityNodeIdRemap {
    pub(crate) const fn new(first_mapped: u64) -> Self {
        Self { first_mapped }
    }

    pub(crate) fn map(self, node_id: AccessibilityNodeId) -> AccessibilityNodeId {
        let offset = node_id
            .0
            .checked_sub(ACCESSIBILITY_FIRST_NODE_ID)
            .expect("hydrolysis dynamic accessibility node id underflow");
        AccessibilityNodeId(
            self.first_mapped
                .checked_add(offset)
                .expect("hydrolysis dynamic accessibility node id overflow"),
        )
    }
}

#[cfg(feature = "accessibility")]
fn remap_accessibility_node_id(
    node_id: AccessibilityNodeId,
    id_map: AccessibilityNodeIdRemap,
) -> AccessibilityNodeId {
    id_map.map(node_id)
}

#[cfg(feature = "accessibility")]
fn remap_accessibility_node_id_vec(
    node_ids: &[AccessibilityNodeId],
    id_map: AccessibilityNodeIdRemap,
) -> Vec<AccessibilityNodeId> {
    node_ids
        .iter()
        .copied()
        .map(|node_id| remap_accessibility_node_id(node_id, id_map))
        .collect()
}

#[cfg(feature = "accessibility")]
fn remap_accessibility_node_references(
    node: &mut AccessibilityNode,
    id_map: AccessibilityNodeIdRemap,
) {
    let children = node.children();
    if !children.is_empty() {
        let node_ids = remap_accessibility_node_id_vec(children, id_map);
        node.set_children(node_ids);
    }
    let controls = node.controls();
    if !controls.is_empty() {
        let node_ids = remap_accessibility_node_id_vec(controls, id_map);
        node.set_controls(node_ids);
    }
    let details = node.details();
    if !details.is_empty() {
        let node_ids = remap_accessibility_node_id_vec(details, id_map);
        node.set_details(node_ids);
    }
    let described_by = node.described_by();
    if !described_by.is_empty() {
        let node_ids = remap_accessibility_node_id_vec(described_by, id_map);
        node.set_described_by(node_ids);
    }
    let flow_to = node.flow_to();
    if !flow_to.is_empty() {
        let node_ids = remap_accessibility_node_id_vec(flow_to, id_map);
        node.set_flow_to(node_ids);
    }
    let labelled_by = node.labelled_by();
    if !labelled_by.is_empty() {
        let node_ids = remap_accessibility_node_id_vec(labelled_by, id_map);
        node.set_labelled_by(node_ids);
    }
    let owns = node.owns();
    if !owns.is_empty() {
        let node_ids = remap_accessibility_node_id_vec(owns, id_map);
        node.set_owns(node_ids);
    }
    let radio_group = node.radio_group();
    if !radio_group.is_empty() {
        let node_ids = remap_accessibility_node_id_vec(radio_group, id_map);
        node.set_radio_group(node_ids);
    }

    if let Some(node_id) = node.active_descendant() {
        node.set_active_descendant(remap_accessibility_node_id(node_id, id_map));
    }
    if let Some(node_id) = node.error_message() {
        node.set_error_message(remap_accessibility_node_id(node_id, id_map));
    }
    if let Some(node_id) = node.in_page_link_target() {
        node.set_in_page_link_target(remap_accessibility_node_id(node_id, id_map));
    }
    if let Some(node_id) = node.member_of() {
        node.set_member_of(remap_accessibility_node_id(node_id, id_map));
    }
    if let Some(node_id) = node.next_on_line() {
        node.set_next_on_line(remap_accessibility_node_id(node_id, id_map));
    }
    if let Some(node_id) = node.previous_on_line() {
        node.set_previous_on_line(remap_accessibility_node_id(node_id, id_map));
    }
    if let Some(node_id) = node.popup_for() {
        node.set_popup_for(remap_accessibility_node_id(node_id, id_map));
    }
}

pub use render::HydroState;
use render::HydroSubview;
pub use render::RenderContext;
pub(crate) use render::{HydrolysisTextContextMenuMode, HydrolysisWindowOrigin};
