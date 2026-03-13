use core::any::{Any, TypeId};
use core::f64::consts::TAU;
use core::num::NonZeroUsize;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[cfg(feature = "accessibility")]
use std::ops::RangeInclusive;
use std::rc::Rc;

use crate::platform::PlatformWindow as _;
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
use waterkit_clipboard::Clipboard;
use waterui::ViewExt;
use waterui::accessibility::{
    AccessibilityChildren, AccessibilityHidden, AccessibilityLabel, AccessibilityRole,
    AccessibilityState,
};
use waterui::animation::Animation;
use waterui::background::{Background, MaterialBackground};
use waterui::border::Border;
use waterui::component::focus::Focused;
use waterui::component::list::{ListConfig, ListItem, Move};
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
use waterui_backend_core::ViewDispatcher;
use waterui_canvas::Canvas;
use waterui_controls::button::{Button, ButtonConfig, ButtonStyle};
use waterui_controls::label::Label as SemanticLabel;
use waterui_controls::menu::{ResolvedCommand, ResolvedMenu, ResolvedMenuItem};
use waterui_controls::slider::SliderConfig;
use waterui_controls::stepper::StepperConfig;
use waterui_controls::text_field::{ResolvedTextFieldConfig, TextField};
use waterui_controls::toggle::{ToggleConfig, ToggleStyle};
use waterui_core::dynamic::Dynamic;
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
use waterui_form::picker::{PickerConfig, PickerStyle};
use waterui_form::secure::{Secure as FormSecure, SecureFieldConfig};
use waterui_graphics::color::{Color, ResolvedColor};
use waterui_graphics::gpu_surface::GestureState;
use waterui_graphics::view_effect::{EffectContext, EffectInput, EffectOutput, ViewEffectErased};
use waterui_graphics::{
    AppliedFilter, FilterContext, FilterInput, FilterOutput, GpuContext, GpuFrame, GpuSurface,
    GradientType, PointerState, RedrawHandle, ResolvedGradient, ResolvedGradientStop, SceneView,
    VelloScene2D,
};
use waterui_icon::SystemIcon;
use waterui_layout::container::{FixedContainer, LazyContainer};
use waterui_layout::safe_area::IgnoreSafeArea;
use waterui_layout::scroll::Axis as ScrollAxis;
use waterui_layout::scroll::ScrollView;
use waterui_layout::spacer::Spacer;
use waterui_layout::stack::{Axis as StackAxis, HStackLayout, VStackLayout};
use waterui_shape::{ClipShape, PathCommand, ResolvedShape};
use waterui_text::font::FontWeight as TextFontWeight;
use waterui_text::styled::{Style as TextStyle, StyledStr};
use waterui_text::{Text, TextConfig};

use crate::animation::AnimationController;
use crate::gesture::{GestureEngine, GestureTarget};
use crate::platform::{
    KeyCode, Modifiers, PointerButton, TextInputPurpose, TextInputState, TouchPhase,
};
use crate::scroll::{ScrollController, ScrollHandle, ScrollMetrics};
use crate::time::Instant;

const GPU_SURFACE_COMPOSITOR_SHADER: &str = include_str!("shaders/gpu_surface_compositor.wgsl");

/// Shared mutable state carried by the hydrolysis dispatcher.
pub struct HydroState {
    pub font_cx: parley::FontContext,
    pub layout_cx: parley::LayoutContext,
    dynamic_intrinsic_cache: BTreeMap<usize, ViewDimensions>,
    frame_device: *const wgpu::Device,
    frame_queue: *const wgpu::Queue,
}

impl Default for HydroState {
    fn default() -> Self {
        Self {
            font_cx: parley::FontContext::new(),
            layout_cx: parley::LayoutContext::new(),
            dynamic_intrinsic_cache: BTreeMap::new(),
            frame_device: core::ptr::null(),
            frame_queue: core::ptr::null(),
        }
    }
}

impl HydroState {
    fn set_frame_resources(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.frame_device = device as *const _;
        self.frame_queue = queue as *const _;
    }

    fn clear_frame_resources(&mut self) {
        self.frame_device = core::ptr::null();
        self.frame_queue = core::ptr::null();
    }

    fn frame_resource_ptrs(&self) -> (*const wgpu::Device, *const wgpu::Queue) {
        assert!(
            !self.frame_device.is_null() && !self.frame_queue.is_null(),
            "hydrolysis frame resources are unavailable during AppliedFilter dispatch"
        );
        (self.frame_device, self.frame_queue)
    }
}

/// Render context passed to handlers.
#[derive(Debug, Clone, Copy)]
pub struct RenderContext {
    renderer_ptr: *mut HydrolysisRenderer,
    pub transform: vello::kurbo::Affine,
    pub hit_transform: vello::kurbo::Affine,
    pub bounds: vello::kurbo::Rect,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HydrolysisWindowOrigin {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HydrolysisTextContextMenuMode {
    Overlay,
    NativeWindow,
}

impl RenderContext {
    pub(crate) fn with_renderer(
        renderer: &mut HydrolysisRenderer,
        bounds: vello::kurbo::Rect,
    ) -> Self {
        Self {
            renderer_ptr: renderer as *mut HydrolysisRenderer,
            transform: vello::kurbo::Affine::IDENTITY,
            hit_transform: vello::kurbo::Affine::IDENTITY,
            bounds,
        }
    }

    pub(crate) fn with_renderer_transforms(
        renderer: &mut HydrolysisRenderer,
        bounds: vello::kurbo::Rect,
        transform: vello::kurbo::Affine,
        hit_transform: vello::kurbo::Affine,
    ) -> Self {
        Self {
            renderer_ptr: renderer as *mut HydrolysisRenderer,
            transform,
            hit_transform,
            bounds,
        }
    }

    /// # Safety
    /// The caller guarantees the render context belongs to an active render pass.
    pub unsafe fn renderer(&self) -> &mut HydrolysisRenderer {
        unsafe { &mut *self.renderer_ptr }
    }

    /// # Safety
    /// The caller guarantees the render context belongs to an active render pass.
    pub unsafe fn scene(&self) -> &mut vello::Scene {
        unsafe { &mut (*self.renderer_ptr).scene }
    }

    #[must_use]
    pub fn child(&self, transform: vello::kurbo::Affine, bounds: vello::kurbo::Rect) -> Self {
        Self {
            renderer_ptr: self.renderer_ptr,
            transform: self.transform * transform,
            hit_transform: self.hit_transform * transform,
            bounds,
        }
    }
}

/// Core hydrolysis renderer state.
pub struct HydrolysisRenderer {
    dispatcher: ViewDispatcher<HydroState, RenderContext, ()>,
    vello_renderer: vello::Renderer,
    scene: vello::Scene,
    vello_layer_surface: Option<VelloLayerSurfaceState>,
    gpu_surface_compositor: Option<GpuSurfaceCompositorState>,
    gpu_surface_slots: Vec<EmbeddedGpuSurfaceRuntime>,
    gpu_surface_cursor: usize,
    render_layers: Vec<RenderLayer>,
    active_scene_layers: Vec<ActiveSceneLayer>,
    active_filter_images: Vec<vello::peniko::ImageData>,
    pointer_targets: Vec<PointerTarget>,
    active_pointer_drag_target: Option<PointerAction>,
    active_pointer_drag_signature: Option<(usize, usize)>,
    gesture_engine: GestureEngine,
    gesture_group_ids: BTreeMap<usize, usize>,
    next_gesture_group_id: usize,
    cursor_targets: Vec<CursorTarget>,
    hover_targets: Vec<HoverTarget>,
    context_menu_targets: Vec<ContextMenuTarget>,
    text_input_targets: Vec<TextInputTarget>,
    text_selection_slots: Vec<Rc<RefCell<TextSelectionSlot>>>,
    text_selection_cursor: usize,
    active_text_selection_drag: Option<usize>,
    last_text_selection_click: Option<TextSelectionClickState>,
    active_text_context_menu: Option<ActiveTextContextMenu>,
    active_popup_menu_group: Option<PopupMenuStateGroup>,
    scroll_targets: Vec<ScrollTarget>,
    hit_test_opacity: f32,
    render_depth: usize,
    hit_test_order: usize,
    window_bounds: vello::kurbo::Rect,
    focused_text_input: Cell<Option<usize>>,
    ime_preedit: Option<Str>,
    text_caret_fade_started_at: Option<Instant>,
    text_caret_next_frame_at: Option<Instant>,
    lifecycle_disappear_previous: BTreeMap<usize, DeferredLifeCycleHook>,
    lifecycle_disappear_current: BTreeMap<usize, DeferredLifeCycleHook>,
    lifecycle_disappear_slot: usize,
    redraw_requested: Rc<Cell<bool>>,
    rebuild_requested: Rc<Cell<bool>>,
    dynamic_nodes: BTreeMap<usize, DynamicNode>,
    animation_controller: AnimationController,
    scroll_controller: ScrollController,
    hover_controller: HoverController,
    lazy_stack_controller: LazyStackController,
    lazy_list_controller: LazyListController,
    lazy_table_controller: LazyTableController,
    lazy_viewport_stack: Vec<vello::kurbo::Rect>,
    pending_scroll_handles: Vec<ScrollHandle>,
    navigation_slots: Vec<NavigationSlot>,
    pending_navigation_entries: Vec<(usize, NavigationEntries)>,
    navigation_cursor: usize,
    dynamic_identities_current_frame: Vec<usize>,
    picker_menu_slots: Vec<PickerMenuSlot>,
    picker_menu_cursor: usize,
    local_state_registry: Rc<RefCell<LocalStateRegistry>>,
    current_frame_retain: Vec<Retain>,
    previous_frame_retain: Vec<Retain>,
    #[cfg(feature = "accessibility")]
    accessibility_nodes: Vec<(AccessibilityNodeId, AccessibilityNode)>,
    #[cfg(feature = "accessibility")]
    accessibility_root_children: Vec<AccessibilityNodeId>,
    #[cfg(feature = "accessibility")]
    accessibility_actions: BTreeMap<AccessibilityNodeId, AccessibilityActionTarget>,
    #[cfg(feature = "accessibility")]
    accessibility_next_node_id: u64,
    #[cfg(feature = "accessibility")]
    accessibility_root_bounds: vello::kurbo::Rect,
    #[cfg(feature = "accessibility")]
    accessibility_root_label: String,
    #[cfg(feature = "accessibility")]
    accessibility_focus: AccessibilityNodeId,
    #[cfg(feature = "accessibility")]
    pending_text_input_accessibility_nodes: VecDeque<AccessibilityNodeId>,
    #[cfg(feature = "accessibility")]
    accessibility_suppression_depth: usize,
    #[cfg(feature = "accessibility")]
    pending_accessibility_tree_update: Option<AccessibilityTreeUpdate>,
}

#[derive(Debug, Clone, Copy)]
struct HydroSubview<'a> {
    view: &'a AnyView,
    state: *mut HydroState,
    env: &'a Environment,
    stretch_axis: StretchAxis,
}

#[derive(Clone)]
struct PointerTarget {
    bounds: vello::kurbo::Rect,
    captures_drag: bool,
    depth: usize,
    order: usize,
    action: PointerAction,
}

#[derive(Clone)]
struct CursorTarget {
    bounds: vello::kurbo::Rect,
    style: CursorStyle,
}

#[derive(Clone)]
struct HoverTarget {
    bounds: vello::kurbo::Rect,
    slot: HoverSlot,
    on_enter: Option<HoverAction>,
    on_move: Option<PointerAction>,
    on_exit: Option<HoverAction>,
}

#[derive(Clone)]
struct ContextMenuTarget {
    bounds: vello::kurbo::Rect,
    depth: usize,
    order: usize,
    items: nami::Computed<Vec<ResolvedMenuItem>>,
}

#[derive(Clone)]
enum PopupMenuNode {
    Command {
        label: SemanticLabel,
        plain_label: String,
        action: SharedAction<()>,
        disabled: bool,
    },
    Divider,
    Menu {
        label: SemanticLabel,
        plain_label: String,
        items: Vec<PopupMenuNode>,
    },
}

#[derive(Clone)]
struct PopupMenuStateGroup(Rc<RefCell<Vec<Binding<WindowState>>>>);

impl PopupMenuStateGroup {
    fn new() -> Self {
        Self(Rc::new(RefCell::new(Vec::new())))
    }

    fn push(&self, state: Binding<WindowState>) {
        self.0.borrow_mut().push(state);
    }

    fn truncate(&self, len: usize) {
        let mut states = self.0.borrow_mut();
        for state in states.drain(len..) {
            state.set(WindowState::Closed);
        }
    }

    fn close_all(&self) {
        self.truncate(0);
    }
}

impl_extractor!(PopupMenuStateGroup);

#[derive(Clone)]
enum TextInputModel {
    TextField {
        value: nami::Binding<StyledStr>,
        line_limit: Option<usize>,
        selection_menu: nami::Computed<Vec<ResolvedMenuItem>>,
    },
    SecureField {
        value: nami::Binding<FormSecure>,
    },
}

#[derive(Debug, Default)]
struct TextSelectionSlot {
    anchor: usize,
    focus: usize,
    initialized: bool,
}
type LocalStateKey = (u64, usize);

#[derive(Clone)]
struct LocalStateSlot {
    type_id: TypeId,
    value: Rc<dyn Any>,
}

#[derive(Default)]
struct LocalStateRegistry {
    slots: BTreeMap<LocalStateKey, LocalStateSlot>,
    active_keys: BTreeSet<LocalStateKey>,
}

impl LocalStateRegistry {
    fn begin_rebuild_frame(&mut self) {
        self.active_keys.clear();
    }

    fn finish_rebuild_frame(&mut self) {
        self.slots.retain(|key, _| self.active_keys.contains(key));
    }

    fn bind_slot(
        &mut self,
        path: u64,
        index: usize,
        type_id: TypeId,
        init: &dyn Fn() -> Rc<dyn Any>,
    ) -> Rc<dyn Any> {
        let key = (path, index);
        self.active_keys.insert(key);
        if let Some(slot) = self.slots.get(&key) {
            assert!(
                slot.type_id == type_id,
                "hydrolysis local state slot type mismatch at path {} slot {}",
                path,
                index
            );
            return Rc::clone(&slot.value);
        }
        let value = init();
        self.slots.insert(
            key,
            LocalStateSlot {
                type_id,
                value: Rc::clone(&value),
            },
        );
        value
    }
}

#[derive(Debug, Clone, Copy)]
struct TextSelectionClickState {
    target_index: usize,
    point: vello::kurbo::Point,
    at: Instant,
    count: u8,
}

#[derive(Clone)]
struct TextInputTarget {
    bounds: vello::kurbo::Rect,
    cursor_area: vello::kurbo::Rect,
    text_bounds: vello::kurbo::Rect,
    layout: parley::Layout<[u8; 4]>,
    purpose: TextInputPurpose,
    depth: usize,
    order: usize,
    model: TextInputModel,
    selection: Rc<RefCell<TextSelectionSlot>>,
    focus_binding: Option<Binding<bool>>,
    #[cfg(feature = "accessibility")]
    accessibility_node_id: Option<AccessibilityNodeId>,
}

#[derive(Clone)]
enum TextContextMenuAction {
    Copy,
    Cut,
    Paste,
    SelectAll,
    Custom(ResolvedCommand),
}

#[derive(Clone)]
enum TextContextMenuEntry {
    Command {
        label: String,
        action: TextContextMenuAction,
    },
    Divider,
}

#[derive(Clone)]
struct TextContextMenuOverlayRow {
    bounds: vello::kurbo::Rect,
    entry: TextContextMenuEntry,
}

#[derive(Clone)]
struct TextContextMenuOverlay {
    bounds: vello::kurbo::Rect,
    rows: Vec<TextContextMenuOverlayRow>,
    model: TextInputModel,
    selection: Rc<RefCell<TextSelectionSlot>>,
    env: Environment,
}

#[derive(Clone)]
enum ActiveTextContextMenu {
    Overlay {
        index: usize,
        overlay: TextContextMenuOverlay,
    },
    NativeWindow {
        index: usize,
        state: nami::Binding<WindowState>,
    },
}

#[derive(Clone)]
struct ScrollTarget {
    bounds: vello::kurbo::Rect,
    action: ScrollAction,
}

type PointerAction = Rc<RefCell<dyn FnMut(vello::kurbo::Point, &Environment) -> bool>>;
type HoverAction = Rc<RefCell<dyn FnMut(&Environment) -> bool>>;
type ScrollAction = Rc<RefCell<dyn FnMut(f32, f32, bool) -> bool>>;

#[derive(Debug, Default)]
struct HoverController {
    slots: Vec<HoverStateSlot>,
    cursor: usize,
}

#[derive(Debug, Clone, Copy)]
struct HoverSlot {
    index: usize,
}

#[derive(Debug)]
struct HoverStateSlot {
    hovering: bool,
}

#[derive(Debug, Default)]
struct LazyStackController {
    slots: Vec<LazyStackSlot>,
    cursor: usize,
}

#[derive(Debug, Default)]
struct LazyListController {
    slots: Vec<LazyListSlot>,
    cursor: usize,
}

#[derive(Debug, Default)]
struct LazyTableController {
    slots: Vec<LazyTableSlot>,
    cursor: usize,
}

#[derive(Debug, Default)]
struct LazyStackSlot {
    item_extents: Vec<Option<f64>>,
}

#[derive(Debug, Default)]
struct LazyListSlot {
    row_extents: Vec<Option<f64>>,
}

#[derive(Debug, Default)]
struct LazyTableSlot {
    column_widths: Vec<f64>,
    max_rows: usize,
}

#[derive(Debug, Clone, Copy)]
struct VisibleIndexWindow {
    start: usize,
    end: usize,
    leading_offset: f64,
}

#[derive(Debug, Clone, Copy)]
struct VisibleColumnWindow {
    start: usize,
    end: usize,
    leading_offset: f64,
}

#[derive(Debug, Clone, Copy)]
enum LazyStackAxisConfig {
    Vertical {
        spacing: f64,
        alignment: HorizontalAlignment,
    },
    Horizontal {
        spacing: f64,
        alignment: VerticalAlignment,
    },
}

#[cfg(feature = "accessibility")]
const ACCESSIBILITY_ROOT_NODE_ID: AccessibilityNodeId = AccessibilityNodeId(0);
#[cfg(feature = "accessibility")]
const ACCESSIBILITY_FIRST_NODE_ID: u64 = 1;

#[cfg(feature = "accessibility")]
#[derive(Clone)]
enum AccessibilityActionTarget {
    PointerPrimaryClick {
        point: vello::kurbo::Point,
    },
    Toggle {
        binding: nami::Binding<bool>,
    },
    Slider {
        value: nami::Binding<f64>,
        range: RangeInclusive<f64>,
        step: f64,
    },
    Stepper {
        value: nami::Binding<i32>,
        step: nami::Computed<i32>,
        range: RangeInclusive<i32>,
    },
    TextField {
        value: nami::Binding<StyledStr>,
        line_limit: Option<usize>,
    },
    SecureField {
        value: nami::Binding<FormSecure>,
    },
    PickerCycle {
        selection: nami::Binding<waterui_core::id::Id>,
        ids: Vec<waterui_core::id::Id>,
    },
    PickerSelect {
        selection: nami::Binding<waterui_core::id::Id>,
        target: waterui_core::id::Id,
    },
    Scroll {
        handle: ScrollHandle,
        axis: ScrollAxis,
    },
}

struct DeferredLifeCycleHook {
    env: Environment,
    hook: LifeCycleHook,
}

struct DynamicNode {
    pending_view: Rc<RefCell<Option<AnyView>>>,
    cached_subtree: Option<DynamicSubtree>,
}

struct DynamicSubtree {
    scene: vello::Scene,
    depth_base: usize,
    pointer_targets: Vec<PointerTarget>,
    gesture_targets: Vec<GestureTarget>,
    cursor_targets: Vec<CursorTarget>,
    hover_targets: Vec<HoverTarget>,
    text_input_targets: Vec<TextInputTarget>,
    scroll_targets: Vec<ScrollTarget>,
    #[cfg(feature = "accessibility")]
    accessibility: DynamicAccessibilitySubtree,
}

#[cfg(feature = "accessibility")]
struct DynamicAccessibilitySubtree {
    nodes: Vec<(AccessibilityNodeId, AccessibilityNode)>,
    root_children: Vec<AccessibilityNodeId>,
    actions: BTreeMap<AccessibilityNodeId, AccessibilityActionTarget>,
}

struct VelloLayerSurfaceState {
    size: (u32, u32),
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct GpuSurfaceCompositorState {
    target_format: wgpu::TextureFormat,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    _white_mask_texture: wgpu::Texture,
    white_mask_view: wgpu::TextureView,
}

struct EmbeddedGpuSurfaceRuntime {
    surface: GpuSurface,
    env: Environment,
    setup_complete: bool,
    output_format: wgpu::TextureFormat,
    output_size: (u32, u32),
    output_texture: Option<wgpu::Texture>,
    output_view: Option<wgpu::TextureView>,
    redraw_handle: RedrawHandle,
    start_time: Instant,
    last_frame_time: Instant,
}

#[derive(Clone)]
enum LayerShape {
    Rect(vello::kurbo::Rect),
    Path(vello::kurbo::BezPath),
}

#[derive(Clone)]
struct ActiveSceneLayer {
    alpha: f32,
    transform: vello::kurbo::Affine,
    shape: LayerShape,
}

impl ActiveSceneLayer {
    fn push_to_scene(&self, scene: &mut vello::Scene) {
        match &self.shape {
            LayerShape::Rect(rect) => {
                scene.push_layer(
                    vello::peniko::Fill::NonZero,
                    vello::peniko::BlendMode::default(),
                    self.alpha,
                    self.transform,
                    rect,
                );
            }
            LayerShape::Path(path) => {
                scene.push_layer(
                    vello::peniko::Fill::NonZero,
                    vello::peniko::BlendMode::default(),
                    self.alpha,
                    self.transform,
                    path,
                );
            }
        }
    }
}

#[derive(Clone)]
struct GpuSurfaceLayer {
    slot_index: usize,
    transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
    active_layers: Vec<ActiveSceneLayer>,
}

enum RenderLayer {
    Vello(vello::Scene),
    GpuSurface(GpuSurfaceLayer),
}

struct PreparedGpuSurfaceLayer {
    view: wgpu::TextureView,
    uniform_bytes: [u8; 64],
    needs_redraw: bool,
}

type NavigationEntries = Rc<RefCell<Vec<AnyViewBuilder<NavigationView>>>>;

struct NavigationSlot {
    entries: NavigationEntries,
    last_depth: usize,
    last_scene: Option<vello::Scene>,
    transition: Option<NavigationTransitionState>,
}

struct PickerMenuSlot {
    open: Rc<Cell<bool>>,
}

#[derive(Clone)]
struct NavigationTransitionState {
    style: NavigationTransition,
    direction: NavigationTransitionDirection,
    from_scene: vello::Scene,
    to_scene: vello::Scene,
    started_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NavigationTransitionDirection {
    Push,
    Pop,
}

struct HydroNavigationController {
    entries: NavigationEntries,
    rebuild_requested: Rc<Cell<bool>>,
}

impl DeferredLifeCycleHook {
    fn new(hook: LifeCycleHook, env: Environment) -> Self {
        Self { env, hook }
    }

    fn call(self) {
        self.hook.handle(&self.env);
    }
}

impl NavigationSlot {
    fn new() -> Self {
        Self {
            entries: Rc::new(RefCell::new(Vec::new())),
            last_depth: 0,
            last_scene: None,
            transition: None,
        }
    }
}

impl NavigationTransitionState {
    fn new(
        style: NavigationTransition,
        direction: NavigationTransitionDirection,
        from_scene: vello::Scene,
        to_scene: vello::Scene,
        started_at: Instant,
    ) -> Self {
        Self {
            style,
            direction,
            from_scene,
            to_scene,
            started_at,
        }
    }

    fn progress(&self, now: Instant) -> f64 {
        let elapsed = now.saturating_duration_since(self.started_at);
        (elapsed.as_secs_f64() / NAVIGATION_TRANSITION_DURATION.as_secs_f64()).clamp(0.0, 1.0)
    }

    fn is_active(&self, now: Instant) -> bool {
        self.progress(now) < 1.0
    }
}

impl GpuSurfaceCompositorState {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, target_format: wgpu::TextureFormat) -> Self {
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_uniform"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(
                            core::num::NonZeroU64::new(64)
                                .expect("static compositor uniform size must be non-zero"),
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(GPU_SURFACE_COMPOSITOR_SHADER)),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let white_mask_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_white_mask"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            white_mask_texture.as_image_copy(),
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let white_mask_view =
            white_mask_texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            target_format,
            uniform_buffer,
            sampler,
            bind_group_layout,
            pipeline,
            _white_mask_texture: white_mask_texture,
            white_mask_view,
        }
    }

    fn ensure_target_format(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) {
        if self.target_format == target_format {
            return;
        }
        *self = Self::new(device, queue, target_format);
    }
}

impl EmbeddedGpuSurfaceRuntime {
    fn new(surface: GpuSurface, env: &Environment) -> Self {
        let now = Instant::now();
        Self {
            surface,
            env: env.clone(),
            setup_complete: false,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            output_size: (1, 1),
            output_texture: None,
            output_view: None,
            redraw_handle: RedrawHandle::new(),
            start_time: now,
            last_frame_time: now - Duration::from_secs_f32(1.0 / 60.0),
        }
    }

    fn replace_surface(&mut self, surface: GpuSurface, env: &Environment) {
        let now = Instant::now();
        self.surface = surface;
        self.env = env.clone();
        self.setup_complete = false;
        self.output_format = wgpu::TextureFormat::Rgba8Unorm;
        self.output_size = (1, 1);
        self.output_texture = None;
        self.output_view = None;
        self.redraw_handle = RedrawHandle::new();
        self.start_time = now;
        self.last_frame_time = now - Duration::from_secs_f32(1.0 / 60.0);
    }

    fn take_external_redraw_request(&self) -> bool {
        self.redraw_handle.take_dirty()
    }

    fn prepare_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        target_width: u32,
        target_height: u32,
        transform: vello::kurbo::Affine,
        bounds: vello::kurbo::Rect,
    ) -> PreparedGpuSurfaceLayer {
        let top_left = transform * vello::kurbo::Point::new(bounds.x0, bounds.y0);
        let top_right = transform * vello::kurbo::Point::new(bounds.x1, bounds.y0);
        let bottom_right = transform * vello::kurbo::Point::new(bounds.x1, bounds.y1);
        let bottom_left = transform * vello::kurbo::Point::new(bounds.x0, bounds.y1);

        let layer_width =
            edge_length_in_pixels(top_left, top_right, target_width, target_height).max(1);
        let layer_height =
            edge_length_in_pixels(top_left, bottom_left, target_width, target_height).max(1);
        let output_format =
            select_embedded_surface_format(target_format, self.surface.get_surface_prefers_hdr());
        self.ensure_setup(device, queue, output_format);
        self.ensure_output_target(device, layer_width, layer_height, output_format);

        let texture = self
            .output_texture
            .as_ref()
            .expect("hydrolysis embedded GpuSurface missing output texture");
        let view = self
            .output_view
            .as_ref()
            .expect("hydrolysis embedded GpuSurface missing output view")
            .clone();
        let now = Instant::now();
        let elapsed = now.duration_since(self.start_time);
        let delta = now
            .duration_since(self.last_frame_time)
            .min(Duration::from_millis(100));
        self.last_frame_time = now;
        let mut frame = GpuFrame::new(
            device,
            queue,
            texture,
            view.clone(),
            output_format,
            layer_width,
            layer_height,
            PointerState::default(),
            GestureState::new(),
            elapsed,
            delta,
        );
        self.surface.render(&mut frame);
        let needs_redraw = frame.was_redraw_requested() || self.redraw_handle.take_dirty();
        let corners = [
            point_to_clip(top_left, target_width, target_height),
            point_to_clip(top_right, target_width, target_height),
            point_to_clip(bottom_right, target_width, target_height),
            point_to_clip(bottom_left, target_width, target_height),
        ];

        PreparedGpuSurfaceLayer {
            view,
            uniform_bytes: encode_compositor_uniform(corners),
            needs_redraw,
        }
    }

    fn ensure_setup(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
    ) {
        if self.setup_complete {
            return;
        }

        let ctx = GpuContext {
            adapter: None,
            device,
            queue,
            surface_format,
            msaa_samples: self.surface.get_msaa_max_samples().get(),
            pipeline_cache: None,
            redraw_handle: self.redraw_handle.clone(),
        };
        ready_now_or_panic(
            self.surface.setup(&ctx, &mut self.env),
            "hydrolysis embedded GpuSurface setup",
        );
        self.setup_complete = true;
    }

    fn ensure_output_target(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) {
        let needs_recreate = self.output_texture.is_none()
            || self.output_size != (width, height)
            || self.output_format != format;
        if !needs_recreate {
            return;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_embedded_gpu_surface_target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.output_format = format;
        self.output_size = (width, height);
        self.output_texture = Some(texture);
        self.output_view = Some(view);
    }
}

fn select_embedded_surface_format(
    target_format: wgpu::TextureFormat,
    prefers_hdr_override: Option<bool>,
) -> wgpu::TextureFormat {
    let target_hdr = matches!(
        target_format,
        wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
    );
    let prefers_hdr =
        waterui_graphics::gpu_surface::resolve_surface_hdr_preference(prefers_hdr_override);
    if target_hdr && prefers_hdr {
        return wgpu::TextureFormat::Rgba16Float;
    }
    wgpu::TextureFormat::Rgba8Unorm
}

fn point_to_clip(point: vello::kurbo::Point, width: u32, height: u32) -> [f32; 2] {
    assert!(
        width != 0 && height != 0,
        "hydrolysis compositor target size must be non-zero"
    );

    let clip_x = ((point.x as f32) / (width as f32)) * 2.0 - 1.0;
    let clip_y = 1.0 - ((point.y as f32) / (height as f32)) * 2.0;
    [clip_x, clip_y]
}

fn edge_length_in_pixels(
    start: vello::kurbo::Point,
    end: vello::kurbo::Point,
    target_width: u32,
    target_height: u32,
) -> u32 {
    assert!(
        target_width != 0 && target_height != 0,
        "hydrolysis compositor target size must be non-zero"
    );
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    ((dx * dx + dy * dy).sqrt().round().max(1.0)) as u32
}

fn encode_compositor_uniform(corners: [[f32; 2]; 4]) -> [u8; 64] {
    let uvs = [[0.0f32, 0.0f32], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let mut bytes = [0u8; 64];
    for (index, corner) in corners.iter().enumerate() {
        let base = index * 16;
        write_f32(&mut bytes, base, corner[0]);
        write_f32(&mut bytes, base + 4, corner[1]);
        write_f32(&mut bytes, base + 8, uvs[index][0]);
        write_f32(&mut bytes, base + 12, uvs[index][1]);
    }
    bytes
}

fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}

fn ready_now_or_panic<F>(future: F, scope: &'static str) -> F::Output
where
    F: core::future::Future,
{
    let mut future = future;
    let mut future = unsafe { Pin::new_unchecked(&mut future) };
    let mut cx = Context::from_waker(core::task::Waker::noop());
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("{scope}: future returned Pending in synchronous path"),
    }
}

impl CustomNavigationController for HydroNavigationController {
    fn push_builder(&mut self, content: AnyViewBuilder<NavigationView>) {
        self.entries.borrow_mut().push(content);
        self.rebuild_requested.set(true);
    }

    fn push(&mut self, _content: NavigationView) {
        panic!(
            "hydrolysis NavigationController::push(NavigationView) is unsupported; use NavigationLink or push_builder"
        );
    }

    fn pop(&mut self) {
        if self.entries.borrow_mut().pop().is_some() {
            self.rebuild_requested.set(true);
        }
    }
}

impl PickerMenuSlot {
    fn new() -> Self {
        Self {
            open: Rc::new(Cell::new(false)),
        }
    }
}

impl TextInputModel {
    fn plain_text(&self) -> String {
        match self {
            Self::TextField { value, .. } => value.get().to_plain().to_string(),
            Self::SecureField { value } => value.get().expose().to_owned(),
        }
    }

    fn set_plain_text(&self, text: String) {
        match self {
            Self::TextField { value, .. } => value.set(StyledStr::plain(text)),
            Self::SecureField { value } => {
                let mut next = FormSecure::default();
                next.set(text);
                value.set(next);
            }
        }
    }

    fn line_limit(&self) -> Option<usize> {
        match self {
            Self::TextField { line_limit, .. } => *line_limit,
            Self::SecureField { .. } => Some(1),
        }
    }

    fn is_secure(&self) -> bool {
        matches!(self, Self::SecureField { .. })
    }

    fn custom_selection_menu_items(&self) -> Vec<ResolvedMenuItem> {
        match self {
            Self::TextField { selection_menu, .. } => selection_menu.get(),
            Self::SecureField { .. } => Vec::new(),
        }
    }

    fn layout_index_from_plain_index(&self, plain_index: usize) -> usize {
        match self {
            Self::TextField { .. } => {
                let text = self.plain_text();
                clamp_to_char_boundary(text.as_str(), plain_index)
            }
            Self::SecureField { .. } => {
                let text = self.plain_text();
                byte_index_to_char_offset(text.as_str(), plain_index)
            }
        }
    }

    fn plain_index_from_layout_index(&self, layout_index: usize) -> usize {
        match self {
            Self::TextField { .. } => {
                let text = self.plain_text();
                clamp_to_char_boundary(text.as_str(), layout_index)
            }
            Self::SecureField { .. } => {
                let text = self.plain_text();
                char_offset_to_byte_index(text.as_str(), layout_index)
            }
        }
    }
}

impl HoverController {
    fn begin_rebuild_frame(&mut self) {
        self.cursor = 0;
    }

    fn finish_rebuild_frame(&mut self) {
        self.slots.truncate(self.cursor);
    }

    fn bind(&mut self) -> (HoverSlot, bool) {
        let index = self.cursor;
        self.cursor = self
            .cursor
            .checked_add(1)
            .expect("hover controller cursor overflow");
        if index == self.slots.len() {
            self.slots.push(HoverStateSlot { hovering: false });
        }
        (HoverSlot { index }, self.slots[index].hovering)
    }

    fn set_hovering(&mut self, slot: HoverSlot, hovering: bool) {
        self.slots[slot.index].hovering = hovering;
    }

    fn cursor(&self) -> usize {
        self.cursor
    }

    fn rewind_to(&mut self, cursor: usize) {
        assert!(
            !(cursor > self.cursor),
            "hover controller rewind cursor exceeds current cursor"
        );
        self.cursor = cursor;
        self.slots.truncate(cursor);
    }
}

impl LazyStackController {
    fn begin_rebuild_frame(&mut self) {
        self.cursor = 0;
    }

    fn finish_rebuild_frame(&mut self) {
        self.slots.truncate(self.cursor);
    }

    fn bind(&mut self) -> usize {
        let index = self.cursor;
        self.cursor = self
            .cursor
            .checked_add(1)
            .expect("lazy stack controller cursor overflow");
        if index == self.slots.len() {
            self.slots.push(LazyStackSlot::default());
        }
        index
    }
}

impl LazyListController {
    fn begin_rebuild_frame(&mut self) {
        self.cursor = 0;
    }

    fn finish_rebuild_frame(&mut self) {
        self.slots.truncate(self.cursor);
    }

    fn bind(&mut self) -> usize {
        let index = self.cursor;
        self.cursor = self
            .cursor
            .checked_add(1)
            .expect("lazy list controller cursor overflow");
        if index == self.slots.len() {
            self.slots.push(LazyListSlot::default());
        }
        index
    }
}

impl LazyTableController {
    fn begin_rebuild_frame(&mut self) {
        self.cursor = 0;
    }

    fn finish_rebuild_frame(&mut self) {
        self.slots.truncate(self.cursor);
    }

    fn bind(&mut self) -> usize {
        let index = self.cursor;
        self.cursor = self
            .cursor
            .checked_add(1)
            .expect("lazy table controller cursor overflow");
        if index == self.slots.len() {
            self.slots.push(LazyTableSlot::default());
        }
        index
    }
}

impl LazyStackSlot {
    fn prepare_len(&mut self, len: usize) {
        self.item_extents.resize(len, None);
    }
}

impl LazyListSlot {
    fn prepare_len(&mut self, len: usize) {
        self.row_extents.resize(len, None);
    }
}

impl LazyTableSlot {
    fn prepare_columns(&mut self, len: usize) {
        self.column_widths.resize(len, TABLE_MIN_COLUMN_WIDTH);
    }
}

impl<'a> HydroSubview<'a> {
    fn from_view(view: &'a AnyView, state: &mut HydroState, env: &'a Environment) -> Self {
        Self {
            view,
            state: state as *mut _,
            env,
            stretch_axis: effective_stretch_axis(view),
        }
    }
}

impl SubView for HydroSubview<'_> {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let state = unsafe { &mut *self.state };
        let mut dimensions =
            measure_view_dimensions_with_proposal(self.view, proposal, state, self.env);

        if self.stretch_axis.stretches_horizontal() {
            if let Some(width) = proposal.width {
                dimensions.size.width = width;
            }
        } else if let Some(width) = proposal.width {
            dimensions.size.width = dimensions.size.width.min(width);
        }

        if self.stretch_axis.stretches_vertical() {
            if let Some(height) = proposal.height {
                dimensions.size.height = height;
            }
        } else if let Some(height) = proposal.height {
            dimensions.size.height = dimensions.size.height.min(height);
        }

        dimensions
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch_axis
    }

    fn priority(&self) -> i32 {
        0
    }
}

impl core::fmt::Debug for HydroState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydroState").finish_non_exhaustive()
    }
}

impl core::fmt::Debug for HydrolysisRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HydrolysisRenderer")
            .field("dispatcher", &self.dispatcher)
            .finish_non_exhaustive()
    }
}

trait HydroNativeView: View + Sized + 'static {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment);
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
        _state: &mut HydroState,
        _ctx: RenderContext,
        _view: &Self,
        _env: &Environment,
    ) {
    }
}

#[cfg(feature = "accessibility")]
fn register_native_view<V: HydroNativeView>(
    dispatcher: &mut ViewDispatcher<HydroState, RenderContext, ()>,
) {
    dispatcher.register::<V>(|state, ctx, view, env| {
        let accessibility_is_render_driven = V::accessibility_is_render_driven();
        let hidden_from_accessibility = env
            .get::<AccessibilityHidden>()
            .is_some_and(AccessibilityHidden::is_hidden);
        if hidden_from_accessibility {
            let renderer = unsafe { ctx.renderer() };
            renderer.push_accessibility_suppression();
            if !accessibility_is_render_driven {
                V::accessibility(state, ctx, &view, env);
            }
            V::render(state, ctx, view, env);
            let renderer = unsafe { ctx.renderer() };
            renderer.pop_accessibility_suppression();
            return;
        }
        if !accessibility_is_render_driven {
            V::accessibility(state, ctx, &view, env);
        }
        let suppress_descendants = env
            .get::<AccessibilityChildren>()
            .is_some_and(AccessibilityChildren::excludes_descendants);
        if suppress_descendants && !accessibility_is_render_driven {
            let renderer = unsafe { ctx.renderer() };
            renderer.push_accessibility_suppression();
            V::render(state, ctx, view, env);
            let renderer = unsafe { ctx.renderer() };
            renderer.pop_accessibility_suppression();
            return;
        }
        V::render(state, ctx, view, env);
    });
}

#[cfg(not(feature = "accessibility"))]
fn register_native_view<V: HydroNativeView>(
    dispatcher: &mut ViewDispatcher<HydroState, RenderContext, ()>,
) {
    dispatcher.register::<V>(|state, ctx, view, env| {
        V::accessibility(state, ctx, &view, env);
        V::render(state, ctx, view, env);
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

const TABLE_MIN_COLUMN_WIDTH: f64 = 72.0;
const TABLE_CELL_HORIZONTAL_PADDING: f64 = 18.0;
const TABLE_HEADER_HEIGHT: f64 = 32.0;
const TABLE_ROW_HEIGHT: f64 = 30.0;

const NAVIGATION_BAR_HEIGHT_AUTOMATIC: f64 = 52.0;
const NAVIGATION_BAR_HEIGHT_INLINE: f64 = 44.0;
const NAVIGATION_BAR_HEIGHT_LARGE: f64 = 64.0;
const NAVIGATION_TITLE_HEIGHT_INLINE: f64 = 24.0;
const NAVIGATION_TITLE_HEIGHT_LARGE: f64 = 32.0;
const NAVIGATION_BAR_HORIZONTAL_INSET: f64 = 12.0;
const NAVIGATION_BAR_BOTTOM_INSET: f64 = 8.0;
const NAVIGATION_BAR_ITEM_SPACING: f64 = 8.0;
const NAVIGATION_SEARCH_HEIGHT: f64 = 40.0;
const NAVIGATION_SEARCH_VERTICAL_INSET: f64 = 8.0;
const NAVIGATION_TRANSITION_DURATION: Duration = Duration::from_millis(250);
const NAVIGATION_PUSHPOP_PARALLAX_FACTOR: f64 = 0.35;

const TABS_BAR_MIN_HEIGHT: f64 = 44.0;
const TABS_BAR_MAX_HEIGHT: f64 = 64.0;
const TABS_BUTTON_MIN_WIDTH: f64 = 44.0;
const TABS_BUTTON_HORIZONTAL_INSET: f64 = 8.0;

const LIST_ROW_CONTENT_MIN_HEIGHT: f32 = 28.0;
const LIST_ROW_VERTICAL_PADDING: f64 = 8.0;
const LIST_ROW_HORIZONTAL_PADDING: f64 = 12.0;
const LIST_MOVE_CONTROL_WIDTH: f64 = 20.0;
const LIST_DELETE_CONTROL_WIDTH: f64 = 26.0;
const LIST_TRAILING_CONTROL_SPACING: f64 = 6.0;
const LIST_ESTIMATED_ROW_HEIGHT: f64 =
    LIST_ROW_CONTENT_MIN_HEIGHT as f64 + LIST_ROW_VERTICAL_PADDING * 2.0;
const HIT_TEST_ALPHA_THRESHOLD: f32 = 0.01;

const BUTTON_MIN_WIDTH: f64 = 44.0;
const BUTTON_MIN_HEIGHT: f64 = 28.0;
const BUTTON_LINK_UNDERLINE_BOTTOM_INSET: f64 = 2.0;
const BUTTON_LINK_UNDERLINE_THICKNESS: f64 = 1.0;
const BUTTON_LINK_HORIZONTAL_PADDING: f64 = 4.0;
const BUTTON_LINK_VERTICAL_PADDING: f64 = 2.0;

const SLIDER_HORIZONTAL_INSET: f64 = 12.0;
const SLIDER_HORIZONTAL_SPACING: f64 = 8.0;
const SLIDER_VERTICAL_SPACING: f64 = 6.0;
const SLIDER_MIN_TRACK_WIDTH: f64 = 72.0;
const SLIDER_TRACK_HEIGHT: f64 = 6.0;
const SLIDER_THUMB_RADIUS: f64 = 9.0;

const TOGGLE_SWITCH_WIDTH: f64 = 51.0;
const TOGGLE_SWITCH_HEIGHT: f64 = 31.0;
const TOGGLE_CHECKBOX_SIZE: f64 = 22.0;
const TOGGLE_LABEL_SPACING: f64 = 8.0;
const CONTROL_SPRING_STIFFNESS: f32 = 300.0;
const CONTROL_SPRING_DAMPING: f32 = 20.0;

const STEPPER_BUTTON_MIN_SIZE: f64 = 24.0;
const STEPPER_BUTTON_MAX_SIZE: f64 = 32.0;
const STEPPER_BUTTON_INTRINSIC_SIZE: f64 = 28.0;
const STEPPER_BUTTON_SPACING: f64 = 4.0;
const STEPPER_LABEL_SPACING: f64 = 8.0;

const PROGRESS_LINEAR_LABEL_HEIGHT: f64 = 18.0;
const PROGRESS_LINEAR_BAR_TOP_OFFSET: f64 = 10.0;
const PROGRESS_LINEAR_BAR_HEIGHT: f64 = 8.0;
const PROGRESS_LINEAR_BAR_HORIZONTAL_INSET: f64 = 8.0;
const PROGRESS_LINEAR_VALUE_LABEL_TOP_SPACING: f64 = 6.0;
const PROGRESS_LINEAR_MIN_TRACK_WIDTH: f64 = 72.0;
const PROGRESS_CIRCULAR_DIAMETER: f64 = 32.0;

const INPUT_LABEL_HEIGHT: f64 = 18.0;
const INPUT_FIELD_MIN_WIDTH: f64 = 72.0;
const INPUT_FIELD_MIN_HEIGHT: f64 = 30.0;
const INPUT_FIELD_HORIZONTAL_INSET: f64 = 8.0;
const INPUT_FIELD_VERTICAL_INSET: f64 = 6.0;
const INPUT_CARET_FADE_CYCLE_DURATION: Duration = Duration::from_millis(1060);
const INPUT_CARET_FADE_FRAME_INTERVAL: Duration = Duration::from_millis(530);
const INPUT_CARET_MIN_OPACITY: f32 = 0.2;
const TEXT_SELECTION_MULTI_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const TEXT_SELECTION_MULTI_CLICK_DISTANCE: f64 = 6.0;
const TEXT_SELECTION_FILL_COLOR: [f32; 4] = [0.20, 0.45, 0.90, 0.28];
const TEXT_CONTEXT_MENU_WINDOW_TITLE: &str = "";
const TEXT_CONTEXT_MENU_ROW_HEIGHT: f32 = 28.0;
const TEXT_CONTEXT_MENU_HORIZONTAL_PADDING: f32 = 10.0;
const TEXT_CONTEXT_MENU_VERTICAL_PADDING: f64 = 6.0;
const TEXT_CONTEXT_MENU_MIN_WIDTH: f32 = 140.0;
const TEXT_CONTEXT_MENU_MAX_WIDTH: f32 = 320.0;
const TEXT_CONTEXT_MENU_WIDTH_PER_CHAR: f32 = 8.5;
const TEXT_CONTEXT_MENU_CORNER_RADIUS: f64 = 6.0;

const PICKER_MIN_WIDTH: f64 = 72.0;
const PICKER_MIN_HEIGHT: f64 = 30.0;
const PICKER_HORIZONTAL_INSET: f64 = 8.0;
const PICKER_VERTICAL_INSET: f64 = 6.0;
const PICKER_INDICATOR_SPACE: f64 = 18.0;
const PICKER_RADIO_INDICATOR_SIZE: f64 = 16.0;
const PICKER_RADIO_LABEL_SPACING: f64 = 8.0;
const PICKER_RADIO_ROW_SPACING: f64 = 6.0;
const PICKER_MENU_POPUP_TOP_SPACING: f64 = 4.0;
const PICKER_MENU_POPUP_CORNER_RADIUS: f64 = 6.0;

fn button_padding(style: ButtonStyle) -> (f64, f64) {
    match style {
        ButtonStyle::Automatic => (8.0, 4.0),
        ButtonStyle::Plain => (0.0, 0.0),
        ButtonStyle::Link => (BUTTON_LINK_HORIZONTAL_PADDING, BUTTON_LINK_VERTICAL_PADDING),
        ButtonStyle::Borderless => (4.0, 2.0),
        ButtonStyle::Bordered => (8.0, 4.0),
        ButtonStyle::BorderedProminent => (10.0, 5.0),
        _ => panic!("hydrolysis ButtonStyle variant is not implemented"),
    }
}

fn button_min_size(style: ButtonStyle) -> (f64, f64) {
    match style {
        ButtonStyle::Automatic | ButtonStyle::Bordered | ButtonStyle::BorderedProminent => {
            (BUTTON_MIN_WIDTH, BUTTON_MIN_HEIGHT)
        }
        ButtonStyle::Plain | ButtonStyle::Link | ButtonStyle::Borderless => (0.0, 0.0),
        _ => panic!("hydrolysis ButtonStyle variant is not implemented"),
    }
}

fn input_placeholder_color() -> Color {
    Color::srgb(142, 142, 147).with_opacity(0.8)
}

fn toggle_control_size(style: ToggleStyle) -> (f64, f64) {
    match style {
        ToggleStyle::Automatic | ToggleStyle::Switch => (TOGGLE_SWITCH_WIDTH, TOGGLE_SWITCH_HEIGHT),
        ToggleStyle::Checkbox => (TOGGLE_CHECKBOX_SIZE, TOGGLE_CHECKBOX_SIZE),
        _ => panic!("hydrolysis ToggleStyle variant is not implemented"),
    }
}

fn slider_value_epsilon(span: f64, track_width: f64) -> f64 {
    (span / track_width).abs().max(f64::EPSILON)
}

fn normalized_insert_text(inserted: &str, max_lines: Option<usize>) -> String {
    if max_lines == Some(1) {
        inserted
            .chars()
            .filter(|ch| *ch != '\n' && *ch != '\r')
            .collect()
    } else {
        inserted.chars().filter(|ch| *ch != '\r').collect()
    }
}

fn line_count(value: &str) -> usize {
    value.chars().filter(|ch| *ch == '\n').count() + 1
}

fn exceeds_line_limit(value: &str, max_lines: Option<usize>) -> bool {
    max_lines.is_some_and(|max| line_count(value) > max)
}

fn apply_text_insert(buffer: &mut String, inserted: &str, max_lines: Option<usize>) -> bool {
    let normalized = normalized_insert_text(inserted, max_lines);
    if normalized.is_empty() {
        return false;
    }
    let original_len = buffer.len();
    buffer.push_str(normalized.as_str());
    if exceeds_line_limit(buffer, max_lines) {
        buffer.truncate(original_len);
        return false;
    }
    true
}

fn apply_backspace(buffer: &mut String) -> bool {
    buffer.pop().is_some()
}

fn clamp_to_char_boundary(text: &str, mut index: usize) -> usize {
    if index > text.len() {
        index = text.len();
    }
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn previous_char_boundary(text: &str, index: usize) -> usize {
    let clamped = clamp_to_char_boundary(text, index);
    if clamped == 0 {
        return 0;
    }
    text[..clamped]
        .char_indices()
        .next_back()
        .map_or(0, |(value, _)| value)
}

fn next_char_boundary(text: &str, index: usize) -> usize {
    let clamped = clamp_to_char_boundary(text, index);
    if clamped >= text.len() {
        return text.len();
    }
    let mut chars = text[clamped..].chars();
    let Some(ch) = chars.next() else {
        return text.len();
    };
    clamped + ch.len_utf8()
}

fn byte_index_to_char_offset(text: &str, index: usize) -> usize {
    let clamped = clamp_to_char_boundary(text, index);
    text[..clamped].chars().count()
}

fn char_offset_to_byte_index(text: &str, char_offset: usize) -> usize {
    if char_offset == 0 {
        return 0;
    }
    let mut consumed = 0usize;
    for (index, _) in text.char_indices() {
        if consumed == char_offset {
            return index;
        }
        consumed = consumed
            .checked_add(1)
            .expect("char offset conversion overflow");
    }
    text.len()
}

fn normalized_selection_range(anchor: usize, focus: usize) -> std::ops::Range<usize> {
    anchor.min(focus)..anchor.max(focus)
}

fn replace_text_selection(
    text: &mut String,
    anchor: &mut usize,
    focus: &mut usize,
    inserted: &str,
    line_limit: Option<usize>,
) -> bool {
    let start = clamp_to_char_boundary(text.as_str(), (*anchor).min(*focus));
    let end = clamp_to_char_boundary(text.as_str(), (*anchor).max(*focus));
    let normalized = normalized_insert_text(inserted, line_limit);
    if normalized.is_empty() {
        return false;
    }
    let mut next = text.clone();
    next.replace_range(start..end, normalized.as_str());
    if exceeds_line_limit(next.as_str(), line_limit) {
        return false;
    }
    *text = next;
    let caret = start + normalized.len();
    *anchor = caret;
    *focus = caret;
    true
}

fn delete_backward_in_selection(text: &mut String, anchor: &mut usize, focus: &mut usize) -> bool {
    let start = clamp_to_char_boundary(text.as_str(), (*anchor).min(*focus));
    let end = clamp_to_char_boundary(text.as_str(), (*anchor).max(*focus));
    if start != end {
        text.replace_range(start..end, "");
        *anchor = start;
        *focus = start;
        return true;
    }
    if start == 0 {
        return false;
    }
    let previous = text[..start]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0);
    text.replace_range(previous..start, "");
    *anchor = previous;
    *focus = previous;
    true
}

fn delete_forward_in_selection(text: &mut String, anchor: &mut usize, focus: &mut usize) -> bool {
    let start = clamp_to_char_boundary(text.as_str(), (*anchor).min(*focus));
    let end = clamp_to_char_boundary(text.as_str(), (*anchor).max(*focus));
    if start != end {
        text.replace_range(start..end, "");
        *anchor = start;
        *focus = start;
        return true;
    }
    if start >= text.len() {
        return false;
    }
    let mut iter = text[start..].char_indices();
    let Some((_, ch)) = iter.next() else {
        return false;
    };
    let next = start + ch.len_utf8();
    text.replace_range(start..next, "");
    *anchor = start;
    *focus = start;
    true
}

fn selection_slot_range_for_text(slot: &TextSelectionSlot, text: &str) -> std::ops::Range<usize> {
    let anchor = clamp_to_char_boundary(text, slot.anchor);
    let focus = clamp_to_char_boundary(text, slot.focus);
    normalized_selection_range(anchor, focus)
}

fn selected_text_for_model(model: &TextInputModel, slot: &TextSelectionSlot) -> Option<String> {
    let text = model.plain_text();
    let range = selection_slot_range_for_text(slot, text.as_str());
    if range.is_empty() {
        return None;
    }
    text.as_str().get(range).map(str::to_owned)
}

fn replace_model_selection(
    model: &TextInputModel,
    slot: &mut TextSelectionSlot,
    inserted: &str,
) -> bool {
    let mut text = model.plain_text();
    let mut anchor = clamp_to_char_boundary(text.as_str(), slot.anchor);
    let mut focus = clamp_to_char_boundary(text.as_str(), slot.focus);
    if !replace_text_selection(
        &mut text,
        &mut anchor,
        &mut focus,
        inserted,
        model.line_limit(),
    ) {
        return false;
    }
    model.set_plain_text(text);
    slot.anchor = anchor;
    slot.focus = focus;
    slot.initialized = true;
    true
}

fn delete_model_selection(model: &TextInputModel, slot: &mut TextSelectionSlot) -> bool {
    let mut text = model.plain_text();
    let mut anchor = clamp_to_char_boundary(text.as_str(), slot.anchor);
    let mut focus = clamp_to_char_boundary(text.as_str(), slot.focus);
    let range = normalized_selection_range(anchor, focus);
    if range.is_empty() {
        return false;
    }
    text.replace_range(range.clone(), "");
    model.set_plain_text(text);
    slot.anchor = range.start;
    slot.focus = range.start;
    slot.initialized = true;
    true
}

fn delete_model_backward(model: &TextInputModel, slot: &mut TextSelectionSlot) -> bool {
    let mut text = model.plain_text();
    let mut anchor = clamp_to_char_boundary(text.as_str(), slot.anchor);
    let mut focus = clamp_to_char_boundary(text.as_str(), slot.focus);
    if !delete_backward_in_selection(&mut text, &mut anchor, &mut focus) {
        return false;
    }
    model.set_plain_text(text);
    slot.anchor = anchor;
    slot.focus = focus;
    slot.initialized = true;
    true
}

fn delete_model_forward(model: &TextInputModel, slot: &mut TextSelectionSlot) -> bool {
    let mut text = model.plain_text();
    let mut anchor = clamp_to_char_boundary(text.as_str(), slot.anchor);
    let mut focus = clamp_to_char_boundary(text.as_str(), slot.focus);
    if !delete_forward_in_selection(&mut text, &mut anchor, &mut focus) {
        return false;
    }
    model.set_plain_text(text);
    slot.anchor = anchor;
    slot.focus = focus;
    slot.initialized = true;
    true
}

fn set_model_caret_position(
    model: &TextInputModel,
    slot: &mut TextSelectionSlot,
    index: usize,
) -> bool {
    let text = model.plain_text();
    let index = clamp_to_char_boundary(text.as_str(), index);
    let changed = slot.anchor != index || slot.focus != index || !slot.initialized;
    slot.anchor = index;
    slot.focus = index;
    slot.initialized = true;
    changed
}

fn move_model_caret_horizontal(
    model: &TextInputModel,
    slot: &mut TextSelectionSlot,
    backward: bool,
    extend: bool,
) -> bool {
    let text = model.plain_text();
    let anchor = clamp_to_char_boundary(text.as_str(), slot.anchor);
    let focus = clamp_to_char_boundary(text.as_str(), slot.focus);
    let base = if !extend && anchor != focus {
        if backward {
            anchor.min(focus)
        } else {
            anchor.max(focus)
        }
    } else {
        focus
    };
    let next = if backward {
        previous_char_boundary(text.as_str(), base)
    } else {
        next_char_boundary(text.as_str(), base)
    };
    if extend {
        let changed = slot.focus != next || !slot.initialized;
        slot.anchor = anchor;
        slot.focus = next;
        slot.initialized = true;
        return changed;
    }
    set_model_caret_position(model, slot, next)
}

fn read_clipboard_text_async() -> impl core::future::Future<Output = Option<String>> {
    async {
        let clipboard = match Clipboard::new() {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(
                    target: "waterui::hydrolysis::input",
                    error = %error,
                    "failed to initialize clipboard for paste"
                );
                return None;
            }
        };
        match clipboard.text().await {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(
                    target: "waterui::hydrolysis::input",
                    error = %error,
                    "failed to read clipboard text"
                );
                None
            }
        }
    }
}

fn spawn_clipboard_paste_task(model: TextInputModel, selection: Rc<RefCell<TextSelectionSlot>>) {
    spawn_local(async move {
        let Some(text) = read_clipboard_text_async().await else {
            return;
        };
        if text.is_empty() {
            return;
        }
        let mut slot = selection.borrow_mut();
        let _ = replace_model_selection(&model, &mut slot, text.as_str());
    })
    .detach();
}

fn select_all_model_text(model: &TextInputModel, slot: &mut TextSelectionSlot) -> bool {
    let text = model.plain_text();
    if text.is_empty() {
        let changed = slot.anchor != 0 || slot.focus != 0 || !slot.initialized;
        slot.anchor = 0;
        slot.focus = 0;
        slot.initialized = true;
        return changed;
    }
    let changed = slot.anchor != 0 || slot.focus != text.len() || !slot.initialized;
    slot.anchor = 0;
    slot.focus = text.len();
    slot.initialized = true;
    changed
}

fn write_clipboard_text(text: &str) -> bool {
    let mut clipboard: Clipboard = match Clipboard::new() {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                target: "waterui::hydrolysis::input",
                error = %error,
                "failed to initialize clipboard for copy/cut"
            );
            return false;
        }
    };
    if let Err(error) = clipboard.set_text(text) {
        tracing::warn!(
            target: "waterui::hydrolysis::input",
            error = %error,
            "failed to set clipboard text"
        );
        return false;
    }
    true
}

fn selection_range_contains_index(
    model: &TextInputModel,
    slot: &TextSelectionSlot,
    index: usize,
) -> bool {
    let text = model.plain_text();
    let range = selection_slot_range_for_text(slot, text.as_str());
    !range.is_empty() && range.contains(&index)
}

fn text_context_menu_size(entries: &[TextContextMenuEntry]) -> (f64, f64) {
    let max_label_chars = entries
        .iter()
        .filter_map(|entry| match entry {
            TextContextMenuEntry::Command { label, .. } => Some(label.chars().count()),
            TextContextMenuEntry::Divider => None,
        })
        .max()
        .unwrap_or(0) as f32;
    let width = (TEXT_CONTEXT_MENU_HORIZONTAL_PADDING * 2.0
        + max_label_chars * TEXT_CONTEXT_MENU_WIDTH_PER_CHAR)
        .clamp(TEXT_CONTEXT_MENU_MIN_WIDTH, TEXT_CONTEXT_MENU_MAX_WIDTH);
    let height =
        (entries.len() as f32 * TEXT_CONTEXT_MENU_ROW_HEIGHT).max(TEXT_CONTEXT_MENU_ROW_HEIGHT);
    (f64::from(width), f64::from(height))
}

fn text_context_menu_overlay_bounds(
    anchor: vello::kurbo::Point,
    entries: &[TextContextMenuEntry],
    window_bounds: vello::kurbo::Rect,
) -> vello::kurbo::Rect {
    let (width, height) = text_context_menu_size(entries);
    let preferred_x = anchor.x;
    let preferred_y = anchor.y;
    let fallback_x = anchor.x - width;
    let fallback_y = anchor.y - height;

    let mut x0 = if preferred_x + width <= window_bounds.x1 {
        preferred_x
    } else {
        fallback_x
    };
    let mut y0 = if preferred_y + height <= window_bounds.y1 {
        preferred_y
    } else {
        fallback_y
    };

    if x0 < window_bounds.x0 {
        x0 = window_bounds.x0;
    }
    if x0 + width > window_bounds.x1 {
        x0 = window_bounds.x1 - width;
    }
    if y0 < window_bounds.y0 {
        y0 = window_bounds.y0;
    }
    if y0 + height > window_bounds.y1 {
        y0 = window_bounds.y1 - height;
    }
    vello::kurbo::Rect::new(x0, y0, x0 + width, y0 + height)
}

fn popup_menu_nodes(items: &[ResolvedMenuItem]) -> Vec<PopupMenuNode> {
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
            let text = Text::new(styled);
            let label = if let Some(icon) = command.icon.clone() {
                SemanticLabel::new(text).icon(icon)
            } else {
                SemanticLabel::new(text)
            };
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
            let text = Text::new(styled);
            let label = if let Some(icon) = menu.icon.clone() {
                SemanticLabel::new(text).icon(icon)
            } else {
                SemanticLabel::new(text)
            };
            PopupMenuNode::Menu {
                label,
                plain_label,
                items: popup_menu_nodes(&menu.items.get()),
            }
        }
    }
}

fn popup_menu_size(nodes: &[PopupMenuNode]) -> (f64, f64) {
    let max_label_chars = nodes
        .iter()
        .filter_map(|node| match node {
            PopupMenuNode::Command { plain_label, .. }
            | PopupMenuNode::Menu { plain_label, .. } => Some(plain_label.chars().count()),
            PopupMenuNode::Divider => None,
        })
        .max()
        .unwrap_or(0) as f32;
    let width = (TEXT_CONTEXT_MENU_HORIZONTAL_PADDING * 2.0
        + max_label_chars * TEXT_CONTEXT_MENU_WIDTH_PER_CHAR)
        .clamp(TEXT_CONTEXT_MENU_MIN_WIDTH, TEXT_CONTEXT_MENU_MAX_WIDTH);
    let height =
        (nodes.len() as f32 * TEXT_CONTEXT_MENU_ROW_HEIGHT).max(TEXT_CONTEXT_MENU_ROW_HEIGHT);
    (f64::from(width), f64::from(height))
}

fn popup_menu_window(
    nodes: Vec<PopupMenuNode>,
    origin: LayoutPoint,
    group: PopupMenuStateGroup,
    depth: usize,
) -> (Window, Binding<WindowState>) {
    let state = Binding::container(WindowState::Normal);
    let (width, height) = popup_menu_size(&nodes);
    let popup_origin_x = origin.x;
    let popup_origin_y = origin.y;
    let group_for_content = group.clone();
    let state_for_content = state.clone();
    let nodes_for_content = nodes.clone();
    let popup_content = move || {
        let mut rows = Vec::with_capacity(nodes_for_content.len());
        for (index, node) in nodes_for_content.clone().into_iter().enumerate() {
            match node {
                PopupMenuNode::Command {
                    label,
                    action,
                    disabled,
                    ..
                } => {
                    let button = Button::new(label)
                        .style(ButtonStyle::Borderless)
                        .extract::<PopupMenuStateGroup>()
                        .extract::<Environment>()
                        .action(move |(group, env)| {
                            if disabled {
                                return;
                            }
                            group.close_all();
                            action.call(&env);
                        });
                    rows.push(AnyView::new(button));
                }
                PopupMenuNode::Divider => rows.push(AnyView::new(Divider)),
                PopupMenuNode::Menu { label, items, .. } => {
                    let next_depth = depth + 1;
                    let child_origin = LayoutPoint::new(
                        popup_origin_x + width as f32,
                        popup_origin_y + TEXT_CONTEXT_MENU_ROW_HEIGHT * index as f32,
                    );
                    let button = Button::new(label)
                        .style(ButtonStyle::Borderless)
                        .extract::<PopupMenuStateGroup>()
                        .extract::<Environment>()
                        .action(move |(group, env)| {
                            if items.is_empty() {
                                return;
                            }
                            group.truncate(next_depth);
                            let (window, child_state) = popup_menu_window(
                                items.clone(),
                                child_origin,
                                group.clone(),
                                next_depth,
                            );
                            group.push(child_state);
                            env.get::<WindowManager>()
                                .expect(
                                    "hydrolysis popup menus require WindowManager in environment",
                                )
                                .show(window);
                        });
                    rows.push(AnyView::new(button));
                }
            }
        }
        let menu_content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
            rows.into_iter().collect();
        AnyView::new(
            menu_content
                .alignment(HorizontalAlignment::Leading)
                .spacing(0.0)
                .with(group_for_content.clone()),
        )
    };
    let mut popup = Window::new(TEXT_CONTEXT_MENU_WINDOW_TITLE, popup_content)
        .style(WindowStyle::Borderless)
        .resizable(false)
        .with_state(state_for_content);
    popup.closable = false;
    popup.frame.set(LayoutRect::new(
        origin,
        LayoutSize::new(width as f32, height as f32),
    ));
    (popup, state)
}

fn execute_text_context_menu_action(
    action: &TextContextMenuAction,
    model: &TextInputModel,
    selection: &Rc<RefCell<TextSelectionSlot>>,
    env: &Environment,
) -> bool {
    match action {
        TextContextMenuAction::Copy => {
            let slot = selection.borrow();
            let Some(value) = selected_text_for_model(model, &slot) else {
                return false;
            };
            write_clipboard_text(value.as_str())
        }
        TextContextMenuAction::Cut => {
            let mut slot = selection.borrow_mut();
            let Some(value) = selected_text_for_model(model, &slot) else {
                return false;
            };
            write_clipboard_text(value.as_str()) && delete_model_selection(model, &mut slot)
        }
        TextContextMenuAction::Paste => {
            spawn_clipboard_paste_task(model.clone(), Rc::clone(selection));
            true
        }
        TextContextMenuAction::SelectAll => {
            let mut slot = selection.borrow_mut();
            select_all_model_text(model, &mut slot)
        }
        TextContextMenuAction::Custom(command) => {
            command.action.call(env);
            true
        }
    }
}

struct TableMetrics {
    column_widths: Vec<f64>,
    table_width: f64,
    table_height: f64,
}

fn table_header_cell_rect(
    origin_x: f64,
    origin_y: f64,
    x_offset: f64,
    width: f64,
) -> vello::kurbo::Rect {
    vello::kurbo::Rect::new(
        origin_x + x_offset,
        origin_y,
        origin_x + x_offset + width,
        origin_y + TABLE_HEADER_HEIGHT,
    )
}

fn table_data_cell_rect(
    origin_x: f64,
    origin_y: f64,
    x_offset: f64,
    width: f64,
    row_index: usize,
) -> vello::kurbo::Rect {
    let y0 = origin_y + TABLE_HEADER_HEIGHT + TABLE_ROW_HEIGHT * row_index as f64;
    vello::kurbo::Rect::new(
        origin_x + x_offset,
        y0,
        origin_x + x_offset + width,
        y0 + TABLE_ROW_HEIGHT,
    )
}

fn lazy_stack_axis_config(layout: &dyn Layout) -> LazyStackAxisConfig {
    let layout_any = layout as &dyn core::any::Any;
    if let Some(vstack) = layout_any.downcast_ref::<VStackLayout>() {
        return LazyStackAxisConfig::Vertical {
            spacing: f64::from(vstack.spacing),
            alignment: vstack.alignment,
        };
    }
    if let Some(hstack) = layout_any.downcast_ref::<HStackLayout>() {
        return LazyStackAxisConfig::Horizontal {
            spacing: f64::from(hstack.spacing),
            alignment: hstack.alignment,
        };
    }
    panic!("hydrolysis LazyContainer supports VStackLayout and HStackLayout only");
}

fn sum_cached_or_estimated(extents: &[Option<f64>], estimate: f64) -> f64 {
    extents
        .iter()
        .map(|extent| extent.unwrap_or(estimate))
        .sum::<f64>()
}

fn resolve_visible_index_window(
    count: usize,
    start_offset: f64,
    end_offset: f64,
    mut extent_at: impl FnMut(usize) -> f64,
) -> VisibleIndexWindow {
    if count == 0 {
        return VisibleIndexWindow {
            start: 0,
            end: 0,
            leading_offset: 0.0,
        };
    }

    let clamped_start = start_offset.max(0.0);
    let clamped_end = end_offset.max(clamped_start);
    let mut index = 0usize;
    let mut offset = 0.0;
    while index < count {
        let extent = extent_at(index);
        if offset + extent > clamped_start {
            break;
        }
        offset += extent;
        index += 1;
    }
    let start = index.min(count);
    let leading_offset = offset;
    while index < count && offset < clamped_end {
        offset += extent_at(index);
        index += 1;
    }
    VisibleIndexWindow {
        start,
        end: index.min(count),
        leading_offset,
    }
}

fn resolve_visible_column_window(
    widths: &[f64],
    start_offset: f64,
    end_offset: f64,
) -> VisibleColumnWindow {
    let count = widths.len();
    if count == 0 {
        return VisibleColumnWindow {
            start: 0,
            end: 0,
            leading_offset: 0.0,
        };
    }
    let clamped_start = start_offset.max(0.0);
    let clamped_end = end_offset.max(clamped_start);
    let mut index = 0usize;
    let mut offset = 0.0;
    while index < count {
        let width = widths[index];
        if offset + width > clamped_start {
            break;
        }
        offset += width;
        index += 1;
    }
    let start = index.min(count);
    let leading_offset = offset;
    while index < count && offset < clamped_end {
        offset += widths[index];
        index += 1;
    }
    VisibleColumnWindow {
        start,
        end: index.min(count),
        leading_offset,
    }
}

fn navigation_bar_height(view: &NavigationView) -> f64 {
    if view.bar.hidden.get() {
        0.0
    } else {
        let base = match view.bar.display_mode {
            waterui::navigation::NavigationTitleDisplayMode::Automatic => {
                NAVIGATION_BAR_HEIGHT_AUTOMATIC
            }
            waterui::navigation::NavigationTitleDisplayMode::Inline => NAVIGATION_BAR_HEIGHT_INLINE,
            waterui::navigation::NavigationTitleDisplayMode::Large => NAVIGATION_BAR_HEIGHT_LARGE,
        };
        let search_extra = if view.bar.search.is_some() {
            NAVIGATION_SEARCH_HEIGHT + NAVIGATION_SEARCH_VERTICAL_INSET * 2.0
        } else {
            0.0
        };
        base + search_extra
    }
}

fn navigation_base_bar_height_for_display_mode(
    display_mode: waterui::navigation::NavigationTitleDisplayMode,
) -> f64 {
    match display_mode {
        waterui::navigation::NavigationTitleDisplayMode::Automatic => {
            NAVIGATION_BAR_HEIGHT_AUTOMATIC
        }
        waterui::navigation::NavigationTitleDisplayMode::Inline => NAVIGATION_BAR_HEIGHT_INLINE,
        waterui::navigation::NavigationTitleDisplayMode::Large => NAVIGATION_BAR_HEIGHT_LARGE,
    }
}

fn split_compact_threshold(sidebar_width: f64) -> f64 {
    sidebar_width + 360.0
}

fn measure_view_intrinsic(view: &AnyView, state: &mut HydroState, env: &Environment) -> LayoutSize {
    measure_view_dimensions(view, state, env).size
}

fn measure_view_dimensions(
    view: &AnyView,
    state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    measure_view_dimensions_with_proposal(view, ProposalSize::UNSPECIFIED, state, env)
}

fn measure_view_dimensions_with_proposal(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    if let Some(metadata) = view.downcast_ref::<Metadata<Environment>>() {
        return measure_view_dimensions_with_proposal(
            &metadata.content,
            proposal,
            state,
            &metadata.value,
        );
    }

    if let Some(content) = passthrough_content(view) {
        return measure_view_dimensions_with_proposal(content, proposal, state, env);
    }

    if view.downcast_ref::<()>().is_some() || view.downcast_ref::<Canvas>().is_some() {
        return ViewDimensions::new(LayoutSize::zero());
    }

    if let Some(text) = view.downcast_ref::<Str>() {
        return HydrolysisRenderer::measure_text_dimensions(
            state,
            StyledStr::plain(text.clone()),
            HorizontalAlignment::Leading,
            env,
            proposal.width,
            None,
        );
    }
    if let Some(text) = view.downcast_ref::<&'static str>() {
        return HydrolysisRenderer::measure_text_dimensions(
            state,
            StyledStr::plain(*text),
            HorizontalAlignment::Leading,
            env,
            proposal.width,
            None,
        );
    }
    if let Some(text) = view.downcast_ref::<String>() {
        return HydrolysisRenderer::measure_text_dimensions(
            state,
            StyledStr::plain(text.clone()),
            HorizontalAlignment::Leading,
            env,
            proposal.width,
            None,
        );
    }
    if let Some(text) = view.downcast_ref::<Cow<'static, str>>() {
        return HydrolysisRenderer::measure_text_dimensions(
            state,
            StyledStr::plain(text.clone()),
            HorizontalAlignment::Leading,
            env,
            proposal.width,
            None,
        );
    }
    if let Some(text) = view.downcast_ref::<Text>() {
        return HydrolysisRenderer::measure_text_dimensions(
            state,
            text.content().get(),
            text.paragraph_alignment().get(),
            env,
            proposal.width,
            None,
        );
    }

    if let Some(dimensions) = dimensions_for_known_native_views(view, proposal, state, env) {
        return dimensions;
    }

    if view.downcast_ref::<Divider>().is_some() {
        return ViewDimensions::new(LayoutSize::new(1.0, 1.0));
    }

    panic!(
        "hydrolysis dimensions estimation encountered unsupported view type {}",
        view.name()
    );
}

fn measure_layout_dimensions<'a>(
    layout: &dyn Layout,
    children: impl IntoIterator<Item = &'a AnyView>,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
) -> ViewDimensions {
    let mut subviews = Vec::new();
    for child in children {
        subviews.push(HydroSubview::from_view(child, state, env));
    }
    let refs: Vec<&dyn SubView> = subviews.iter().map(|view| view as &dyn SubView).collect();
    let size = layout.size_that_fits(proposal, &refs);
    let bounds = LayoutRect::from_size(size);
    let child_rects = layout.place(bounds, &refs);
    let placed_subviews: Vec<PlacedSubview<'_>> = subviews
        .iter()
        .zip(child_rects.iter().copied())
        .map(|(view, frame)| PlacedSubview::new(view as &dyn SubView, frame))
        .collect();

    let mut dimensions = ViewDimensions::new(size);
    let mut horizontal_keys = Vec::new();
    let mut vertical_keys = Vec::new();

    for alignment in layout.explicit_horizontal_alignments() {
        if !horizontal_keys.contains(&alignment) {
            horizontal_keys.push(alignment);
        }
    }
    for alignment in layout.explicit_vertical_alignments() {
        if !vertical_keys.contains(&alignment) {
            vertical_keys.push(alignment);
        }
    }

    for child in &placed_subviews {
        let child_dimensions = child.dimensions();
        for (alignment, _) in child_dimensions.explicit_horizontal_guides() {
            if !horizontal_keys.contains(&alignment) {
                horizontal_keys.push(alignment);
            }
        }
        for (alignment, _) in child_dimensions.explicit_vertical_guides() {
            if !vertical_keys.contains(&alignment) {
                vertical_keys.push(alignment);
            }
        }
    }

    for alignment in horizontal_keys {
        if let Some(value) = layout.explicit_horizontal(alignment, bounds, &placed_subviews) {
            dimensions.set_horizontal(alignment, value);
        }
    }
    for alignment in vertical_keys {
        if let Some(value) = layout.explicit_vertical(alignment, bounds, &placed_subviews) {
            dimensions.set_vertical(alignment, value);
        }
    }

    dimensions
}

fn measure_navigation_view_intrinsic(
    navigation: &NavigationView,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let bar_height = navigation_bar_height(navigation);
    let title_size = if bar_height > 0.0 {
        measure_view_intrinsic(&navigation.bar.title, state, env)
    } else {
        LayoutSize::zero()
    };
    let leading_size = if !navigation.bar.leading.is::<()>() {
        measure_view_intrinsic(&navigation.bar.leading, state, env)
    } else {
        LayoutSize::zero()
    };
    let trailing_size = if !navigation.bar.trailing.is::<()>() {
        measure_view_intrinsic(&navigation.bar.trailing, state, env)
    } else {
        LayoutSize::zero()
    };
    let search_size = if let Some(search) = navigation.bar.search.as_ref() {
        measure_view_intrinsic(
            &AnyView::new(TextField::new(&search.text).prompt(search.prompt.clone())),
            state,
            env,
        )
    } else {
        LayoutSize::zero()
    };
    let content_size = measure_view_intrinsic(&navigation.content, state, env);
    let width = f64::from(content_size.width)
        .max(
            f64::from(leading_size.width)
                + f64::from(title_size.width)
                + f64::from(trailing_size.width)
                + NAVIGATION_BAR_HORIZONTAL_INSET * 2.0
                + NAVIGATION_BAR_ITEM_SPACING * 2.0,
        )
        .max(f64::from(search_size.width) + NAVIGATION_BAR_HORIZONTAL_INSET * 2.0);
    let height = f64::from(content_size.height) + bar_height;
    LayoutSize::new(width as f32, height as f32)
}

fn measure_tabs_intrinsic(tabs: &Tabs, state: &mut HydroState, env: &Environment) -> LayoutSize {
    assert!(
        !(tabs.tabs.is_empty()),
        "hydrolysis Tabs requires at least one tab"
    );

    let mut max_content_width: f64 = 0.0;
    let mut max_content_height: f64 = 0.0;
    let mut bar_width = 0.0;
    for tab in &tabs.tabs {
        let label_size = measure_view_intrinsic(&tab.label.content, state, env);
        bar_width += (f64::from(label_size.width) + TABS_BUTTON_HORIZONTAL_INSET * 2.0)
            .max(TABS_BUTTON_MIN_WIDTH);

        let content = normalize_layout_view(AnyView::new(tab.content.build()), env);
        let content_size = measure_view_intrinsic(&content, state, env);
        max_content_width = max_content_width.max(f64::from(content_size.width));
        max_content_height = max_content_height.max(f64::from(content_size.height));
    }

    let width = max_content_width.max(bar_width);
    let height = max_content_height + TABS_BAR_MIN_HEIGHT;
    LayoutSize::new(width as f32, height as f32)
}

fn tabs_bar_and_content_rect(
    bounds: vello::kurbo::Rect,
    position: TabPosition,
) -> (vello::kurbo::Rect, vello::kurbo::Rect) {
    let bar_height = (bounds.height() * 0.12).clamp(TABS_BAR_MIN_HEIGHT, TABS_BAR_MAX_HEIGHT);
    match position {
        TabPosition::Top => (
            vello::kurbo::Rect::new(
                bounds.x0,
                bounds.y0,
                bounds.x1,
                (bounds.y0 + bar_height).min(bounds.y1),
            ),
            vello::kurbo::Rect::new(
                bounds.x0,
                (bounds.y0 + bar_height).min(bounds.y1),
                bounds.x1,
                bounds.y1,
            ),
        ),
        TabPosition::Bottom => (
            vello::kurbo::Rect::new(
                bounds.x0,
                (bounds.y1 - bar_height).max(bounds.y0),
                bounds.x1,
                bounds.y1,
            ),
            vello::kurbo::Rect::new(
                bounds.x0,
                bounds.y0,
                bounds.x1,
                (bounds.y1 - bar_height).max(bounds.y0),
            ),
        ),
    }
}

fn tabs_button_rect(
    bar_rect: vello::kurbo::Rect,
    tab_count: usize,
    index: usize,
) -> vello::kurbo::Rect {
    let button_width = bar_rect.width() / tab_count as f64;
    let x0 = bar_rect.x0 + button_width * index as f64;
    vello::kurbo::Rect::new(x0, bar_rect.y0, x0 + button_width, bar_rect.y1)
}

fn navigation_back_button_rect(bounds: vello::kurbo::Rect) -> vello::kurbo::Rect {
    vello::kurbo::Rect::new(
        bounds.x0 + 8.0,
        bounds.y0 + 8.0,
        bounds.x0 + 38.0,
        bounds.y0 + 36.0,
    )
}

fn menu_picker_row_height(field_bounds: vello::kurbo::Rect, max_item_text_height: f64) -> f64 {
    field_bounds
        .height()
        .max(PICKER_MIN_HEIGHT)
        .max(max_item_text_height + PICKER_VERTICAL_INSET * 2.0)
}

fn menu_picker_popup_rect(
    field_bounds: vello::kurbo::Rect,
    row_height: f64,
    item_count: usize,
) -> vello::kurbo::Rect {
    let y0 = field_bounds.y1 + PICKER_MENU_POPUP_TOP_SPACING;
    let y1 = y0 + row_height * item_count as f64;
    vello::kurbo::Rect::new(field_bounds.x0, y0, field_bounds.x1, y1)
}

fn menu_picker_option_rect(
    popup_rect: vello::kurbo::Rect,
    row_height: f64,
    index: usize,
) -> vello::kurbo::Rect {
    let y0 = popup_rect.y0 + row_height * index as f64;
    vello::kurbo::Rect::new(popup_rect.x0, y0, popup_rect.x1, y0 + row_height)
}

fn measure_list_intrinsic(
    list: &ListConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let row_count = list.contents.len().get();
    if row_count == 0 {
        return LayoutSize::zero();
    }
    let editing = list.editing.get();
    let mut first_item = list
        .contents
        .get_view(0)
        .unwrap_or_else(|| panic!("ListConfig failed to materialize item at index 0"));
    first_item.content = normalize_layout_view(first_item.content, env);
    let content_size = measure_view_intrinsic(&first_item.content, state, env);
    let row_height = f64::from(content_size.height.max(LIST_ROW_CONTENT_MIN_HEIGHT))
        + LIST_ROW_VERTICAL_PADDING * 2.0;

    let mut row_width = f64::from(content_size.width) + LIST_ROW_HORIZONTAL_PADDING * 2.0;
    if editing && list.on_move.is_some() {
        row_width += LIST_MOVE_CONTROL_WIDTH + LIST_TRAILING_CONTROL_SPACING;
    }
    if editing && list.on_delete.is_some() {
        row_width += LIST_DELETE_CONTROL_WIDTH + LIST_TRAILING_CONTROL_SPACING;
    }
    let total_height = row_height * row_count as f64;
    let max_width = row_width.max(LIST_ROW_HORIZONTAL_PADDING * 2.0);

    LayoutSize::new(max_width as f32, total_height as f32)
}

fn materialize_list_item(
    contents: &impl Views<View = ListItem>,
    index: usize,
    env: &Environment,
) -> ListItem {
    let mut item = contents
        .get_view(index)
        .unwrap_or_else(|| panic!("ListConfig failed to materialize item at index {index}"));
    item.content = normalize_layout_view(item.content, env);
    item
}

fn measure_list_item_row_height(item: &ListItem, state: &mut HydroState, env: &Environment) -> f64 {
    let intrinsic = measure_view_intrinsic(&item.content, state, env);
    f64::from(intrinsic.height.max(LIST_ROW_CONTENT_MIN_HEIGHT)) + LIST_ROW_VERTICAL_PADDING * 2.0
}

fn materialize_list_row(
    contents: &impl Views<View = ListItem>,
    index: usize,
    state: &mut HydroState,
    env: &Environment,
) -> (ListItem, f64) {
    let item = materialize_list_item(contents, index, env);
    let row_height = measure_list_item_row_height(&item, state, env);
    (item, row_height)
}

fn measure_button_intrinsic(
    button: &ButtonConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let label_size = measure_view_intrinsic(&button.label, state, env);
    let (padding_x, padding_y) = button_padding(button.style);
    let content_width = f64::from(label_size.width) + padding_x * 2.0;
    let content_height = f64::from(label_size.height) + padding_y * 2.0;
    let (min_width, min_height) = button_min_size(button.style);
    LayoutSize::new(
        content_width.max(min_width) as f32,
        content_height.max(min_height) as f32,
    )
}

fn measure_menu_intrinsic(
    menu: &ResolvedMenu,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let label_size = measure_view_intrinsic(&menu.label, state, env);
    let (padding_x, padding_y) = button_padding(ButtonStyle::Borderless);
    let content_width = f64::from(label_size.width) + padding_x * 2.0;
    let content_height = f64::from(label_size.height) + padding_y * 2.0;
    let (min_width, min_height) = button_min_size(ButtonStyle::Borderless);
    LayoutSize::new(
        content_width.max(min_width) as f32,
        content_height.max(min_height) as f32,
    )
}

fn measure_toggle_intrinsic(
    toggle: &ToggleConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let (control_width, control_height) = toggle_control_size(toggle.style);
    let label_size = measure_view_intrinsic(&toggle.label, state, env);
    let label_width = f64::from(label_size.width);
    let width = if label_width > 0.0 {
        label_width + TOGGLE_LABEL_SPACING + control_width
    } else {
        control_width
    };
    let height = f64::from(label_size.height).max(control_height);
    LayoutSize::new(width as f32, height as f32)
}

fn measure_stepper_intrinsic(
    stepper: &StepperConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let label_size = measure_view_intrinsic(&stepper.label, state, env);
    let controls_width = STEPPER_BUTTON_INTRINSIC_SIZE * 2.0 + STEPPER_BUTTON_SPACING;
    let label_width = f64::from(label_size.width);
    let width = if label_width > 0.0 {
        label_width + STEPPER_LABEL_SPACING + controls_width
    } else {
        controls_width
    };
    let height = f64::from(label_size.height).max(STEPPER_BUTTON_INTRINSIC_SIZE);
    LayoutSize::new(width as f32, height as f32)
}

fn measure_progress_intrinsic(
    progress: &ProgressConfig,
    _state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    match progress.style {
        ProgressStyle::Linear => {
            let label_height =
                f64::from(waterui_text::font::Font::default().resolve(env).get().size)
                    .max(PROGRESS_LINEAR_LABEL_HEIGHT);
            let width =
                PROGRESS_LINEAR_MIN_TRACK_WIDTH + PROGRESS_LINEAR_BAR_HORIZONTAL_INSET * 2.0;
            let height = label_height
                + PROGRESS_LINEAR_BAR_TOP_OFFSET
                + PROGRESS_LINEAR_BAR_HEIGHT
                + PROGRESS_LINEAR_VALUE_LABEL_TOP_SPACING
                + label_height;
            LayoutSize::new(width as f32, height as f32)
        }
        ProgressStyle::Circular => LayoutSize::new(
            PROGRESS_CIRCULAR_DIAMETER as f32,
            PROGRESS_CIRCULAR_DIAMETER as f32,
        ),
        _ => panic!("hydrolysis ProgressStyle variant is not implemented"),
    }
}

fn measure_text_field_intrinsic(
    text_field: &ResolvedTextFieldConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let label_size = measure_view_intrinsic(&text_field.label, state, env);
    let has_label = label_size.width > 0.0 || label_size.height > 0.0;
    let label_height = f64::from(label_size.height).max(INPUT_LABEL_HEIGHT);
    let line_limit = text_field.line_limit.map(NonZeroUsize::get);
    let prompt = text_field.prompt.content.get();
    let value = text_field.value.get();
    let prompt_size = HydrolysisRenderer::measure_text_intrinsic_size_with_line_limit(
        state, prompt, env, line_limit,
    );
    let value_size = HydrolysisRenderer::measure_text_intrinsic_size_with_line_limit(
        state, value, env, line_limit,
    );
    let content_width =
        f64::from(prompt_size.width.max(value_size.width)) + INPUT_FIELD_HORIZONTAL_INSET * 2.0;
    let content_height =
        f64::from(prompt_size.height.max(value_size.height)) + INPUT_FIELD_VERTICAL_INSET * 2.0;

    let field_width = content_width.max(INPUT_FIELD_MIN_WIDTH);
    let field_height = content_height.max(INPUT_FIELD_MIN_HEIGHT);
    let width = f64::from(label_size.width).max(field_width);
    let height = if has_label {
        label_height + field_height
    } else {
        field_height
    };
    LayoutSize::new(width as f32, height as f32)
}

fn measure_secure_field_intrinsic(
    secure_field: &SecureFieldConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let label_size = measure_view_intrinsic(&secure_field.label, state, env);
    let has_label = label_size.width > 0.0 || label_size.height > 0.0;
    let label_height = f64::from(label_size.height).max(INPUT_LABEL_HEIGHT);
    let secure_len = secure_field.value.get().expose().chars().count();
    let masked = if secure_len == 0 {
        StyledStr::plain("")
    } else {
        StyledStr::plain("*".repeat(secure_len))
    };
    let value_size = HydrolysisRenderer::measure_text_intrinsic_size(state, masked, env);
    let field_width = (f64::from(value_size.width) + INPUT_FIELD_HORIZONTAL_INSET * 2.0)
        .max(INPUT_FIELD_MIN_WIDTH);
    let field_height = (f64::from(value_size.height) + INPUT_FIELD_VERTICAL_INSET * 2.0)
        .max(INPUT_FIELD_MIN_HEIGHT);
    let width = f64::from(label_size.width).max(field_width);
    let height = if has_label {
        label_height + field_height
    } else {
        field_height
    };
    LayoutSize::new(width as f32, height as f32)
}

fn measure_table_metrics(
    columns: &[TableColumn],
    state: &mut HydroState,
    env: &Environment,
) -> TableMetrics {
    let mut column_widths = Vec::with_capacity(columns.len());
    let mut max_rows = 0usize;
    for column in columns {
        let mut width = TABLE_MIN_COLUMN_WIDTH;
        let label_view = normalize_layout_view(AnyView::new(column.label()), env);
        let label_size = measure_view_intrinsic(&label_view, state, env);
        width = width.max(f64::from(label_size.width) + TABLE_CELL_HORIZONTAL_PADDING);

        let rows = column.rows();
        max_rows = max_rows.max(rows.len().get());
        column_widths.push(width);
    }

    let table_width: f64 = column_widths.iter().sum();
    let table_height = TABLE_HEADER_HEIGHT + TABLE_ROW_HEIGHT * max_rows as f64;
    TableMetrics {
        column_widths,
        table_width,
        table_height,
    }
}

fn resolve_table_visible_rows(
    offset_y: f64,
    viewport_height: f64,
    max_rows: usize,
) -> VisibleIndexWindow {
    let data_start = (offset_y - TABLE_HEADER_HEIGHT).max(0.0);
    let data_end = (offset_y + viewport_height - TABLE_HEADER_HEIGHT).max(0.0);
    let start = ((data_start / TABLE_ROW_HEIGHT).floor() as usize).min(max_rows);
    let end = ((data_end / TABLE_ROW_HEIGHT).ceil() as usize).min(max_rows);
    VisibleIndexWindow {
        start,
        end: end.max(start),
        leading_offset: start as f64 * TABLE_ROW_HEIGHT,
    }
}

fn refresh_table_slot_baseline(
    columns: &[TableColumn],
    slot: &mut LazyTableSlot,
    state: &mut HydroState,
    env: &Environment,
) {
    slot.prepare_columns(columns.len());
    slot.max_rows = 0;
    for (index, column) in columns.iter().enumerate() {
        let label_view = normalize_layout_view(AnyView::new(column.label()), env);
        let label_size = measure_view_intrinsic(&label_view, state, env);
        let width = (f64::from(label_size.width) + TABLE_CELL_HORIZONTAL_PADDING)
            .max(TABLE_MIN_COLUMN_WIDTH);
        if slot.column_widths[index] < width {
            slot.column_widths[index] = width;
        }
        slot.max_rows = slot.max_rows.max(column.rows().len().get());
    }
}

fn update_table_slot_visible_cell_widths(
    columns: &[TableColumn],
    slot: &mut LazyTableSlot,
    row_window: VisibleIndexWindow,
    col_window: VisibleColumnWindow,
    state: &mut HydroState,
    env: &Environment,
) {
    for column_index in col_window.start..col_window.end {
        let column = &columns[column_index];
        let rows = column.rows();
        for row_index in row_window.start..row_window.end {
            if let Some(cell) = rows.get_view(row_index) {
                let cell_view = normalize_layout_view(AnyView::new(cell), env);
                let size = measure_view_intrinsic(&cell_view, state, env);
                let width = (f64::from(size.width) + TABLE_CELL_HORIZONTAL_PADDING)
                    .max(TABLE_MIN_COLUMN_WIDTH);
                if slot.column_widths[column_index] < width {
                    slot.column_widths[column_index] = width;
                }
            }
        }
    }
}

fn table_metrics_from_slot(slot: &LazyTableSlot) -> TableMetrics {
    TableMetrics {
        column_widths: slot.column_widths.clone(),
        table_width: slot.column_widths.iter().sum(),
        table_height: TABLE_HEADER_HEIGHT + TABLE_ROW_HEIGHT * slot.max_rows as f64,
    }
}

fn measure_slider_intrinsic(
    slider: &SliderConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let label_size = measure_view_intrinsic(&slider.label, state, env);
    let min_label_size = measure_view_intrinsic(&slider.min_value_label, state, env);
    let max_label_size = measure_view_intrinsic(&slider.max_value_label, state, env);

    let control_row_height = (SLIDER_THUMB_RADIUS * 2.0)
        .max(f64::from(min_label_size.height))
        .max(f64::from(max_label_size.height));
    let label_height = f64::from(label_size.height);
    let intrinsic_height = if label_height > 0.0 {
        label_height + SLIDER_VERTICAL_SPACING + control_row_height
    } else {
        control_row_height
    };

    let min_width = f64::from(label_size.width).max(
        f64::from(min_label_size.width)
            + SLIDER_HORIZONTAL_SPACING
            + SLIDER_MIN_TRACK_WIDTH
            + SLIDER_HORIZONTAL_SPACING
            + f64::from(max_label_size.width)
            + SLIDER_HORIZONTAL_INSET * 2.0,
    );
    LayoutSize::new(min_width as f32, intrinsic_height as f32)
}

fn measure_picker_intrinsic(
    picker: &PickerConfig,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let items = picker.items.get();
    assert!(
        !(items.is_empty()),
        "hydrolysis picker requires at least one item"
    );
    let item_count = items.len();

    match picker.style {
        PickerStyle::Automatic | PickerStyle::Menu => {
            let mut max_item_width: f64 = 0.0;
            let mut max_item_height: f64 = 0.0;
            for item in &items {
                let styled = item.content.content().get();
                let size = HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env);
                max_item_width = max_item_width.max(f64::from(size.width));
                max_item_height = max_item_height.max(f64::from(size.height));
            }

            let width = (max_item_width + PICKER_HORIZONTAL_INSET * 2.0 + PICKER_INDICATOR_SPACE)
                .max(PICKER_MIN_WIDTH);
            let height = (max_item_height + PICKER_VERTICAL_INSET * 2.0).max(PICKER_MIN_HEIGHT);
            LayoutSize::new(width as f32, height as f32)
        }
        PickerStyle::Radio => {
            let mut max_item_width: f64 = 0.0;
            let mut total_height = 0.0;
            for (index, item) in items.iter().enumerate() {
                let styled = item.content.content().get();
                let size = HydrolysisRenderer::measure_text_intrinsic_size(state, styled, env);
                max_item_width = max_item_width.max(f64::from(size.width));
                total_height += f64::from(size.height).max(PICKER_RADIO_INDICATOR_SIZE);
                if index + 1 < item_count {
                    total_height += PICKER_RADIO_ROW_SPACING;
                }
            }
            let width = (PICKER_HORIZONTAL_INSET * 2.0
                + PICKER_RADIO_INDICATOR_SIZE
                + PICKER_RADIO_LABEL_SPACING
                + max_item_width)
                .max(PICKER_MIN_WIDTH);
            let height = (PICKER_VERTICAL_INSET * 2.0 + total_height).max(PICKER_MIN_HEIGHT);
            LayoutSize::new(width as f32, height as f32)
        }
        _ => panic!("hydrolysis PickerStyle variant is not implemented"),
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
        $macro!(Native<ListConfig>);
        $macro!(Native<TableConfig>);
        $macro!(Native<ButtonConfig>);
        $macro!(Native<ResolvedMenu>);
        $macro!(Native<ToggleConfig>);
        $macro!(Native<SliderConfig>);
        $macro!(Native<StepperConfig>);
        $macro!(Native<ProgressConfig>);
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
    };
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

impl HydroNativeView for Native<()> {
    fn render(_state: &mut HydroState, _ctx: RenderContext, _view: Self, _env: &Environment) {}

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }
}

impl HydroNativeView for Native<Spacer> {
    fn render(_state: &mut HydroState, _ctx: RenderContext, _view: Self, _env: &Environment) {}

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }
}

impl HydroNativeView for Native<TextConfig> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_text_config(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        HydrolysisRenderer::measure_text_dimensions(
            state,
            view.as_inner().content.get(),
            view.as_inner().paragraph_alignment.get(),
            env,
            None,
            None,
        )
        .size
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        HydrolysisRenderer::measure_text_dimensions(
            state,
            view.as_inner().content.get(),
            view.as_inner().paragraph_alignment.get(),
            env,
            proposal.width,
            None,
        )
    }

    fn accessibility(_state: &mut HydroState, ctx: RenderContext, view: &Self, env: &Environment) {
        #[cfg(feature = "accessibility")]
        {
            let text = view.as_inner();
            let renderer = unsafe { ctx.renderer() };
            let plain = renderer.read_signal(&text.content).to_plain().to_string();
            let default_label = (!plain.is_empty()).then_some(plain);
            let label = renderer.resolve_accessibility_label(env, default_label);
            let Some(label) = label else {
                return;
            };
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

impl HydroNativeView for Native<FixedContainer> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_fixed_container(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let (layout, children) = view.as_inner().as_parts();
        estimate_layout_intrinsic(layout, children.iter(), state, env)
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let (layout, children) = view.as_inner().as_parts();
        measure_layout_dimensions(layout, children.iter(), proposal, state, env)
    }
}

impl HydroNativeView for Native<LazyContainer> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_lazy_container(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let (layout, children) = view.as_inner().as_parts();
        let child_count = children.len().get();
        if child_count == 0 {
            return LayoutSize::zero();
        }
        let sample = children
            .get_view(0)
            .map(|view| normalize_layout_view(view, env))
            .map(|view| measure_view_intrinsic(&view, state, env))
            .unwrap_or_else(|| panic!("LazyContainer failed to materialize child at index 0"));
        let count = child_count as f64;
        match lazy_stack_axis_config(layout) {
            LazyStackAxisConfig::Vertical { spacing, .. } => {
                let width = f64::from(sample.width);
                let height = f64::from(sample.height) * count + spacing * (count - 1.0).max(0.0);
                LayoutSize::new(width as f32, height as f32)
            }
            LazyStackAxisConfig::Horizontal { spacing, .. } => {
                let width = f64::from(sample.width) * count + spacing * (count - 1.0).max(0.0);
                let height = f64::from(sample.height);
                LayoutSize::new(width as f32, height as f32)
            }
        }
    }
}

impl HydroNativeView for Native<ScrollView> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_scroll_view(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let (_axis, content) = view.as_inner().as_parts();
        measure_view_intrinsic(content, state, env)
    }

    fn accessibility(state: &mut HydroState, ctx: RenderContext, view: &Self, env: &Environment) {
        let (axis, content) = view.as_inner().as_parts();
        let viewport = ctx.bounds;
        let intrinsic = measure_view_intrinsic(content, state, env);
        let (content_width, content_height) = match axis {
            ScrollAxis::Horizontal => (
                f64::from(intrinsic.width).max(viewport.width()),
                viewport.height(),
            ),
            ScrollAxis::Vertical => (
                viewport.width(),
                f64::from(intrinsic.height).max(viewport.height()),
            ),
            ScrollAxis::All => (
                f64::from(intrinsic.width).max(viewport.width()),
                f64::from(intrinsic.height).max(viewport.height()),
            ),
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        };
        let handle = {
            let renderer = unsafe { ctx.renderer() };
            let handle = renderer.scroll_controller.bind(
                axis,
                viewport.width(),
                viewport.height(),
                content_width,
                content_height,
            );
            renderer.push_pending_scroll_handle(handle.clone());
            handle
        };
        #[cfg(feature = "accessibility")]
        {
            let metrics = handle.metrics();
            let renderer = unsafe { ctx.renderer() };
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::ScrollView),
            );
            let label = renderer.resolve_accessibility_label(env, None);
            if let Some(label) = label {
                node.set_label(label);
            }
            node.set_scroll_x(metrics.offset_x);
            node.set_scroll_x_min(0.0);
            node.set_scroll_x_max(metrics.max_x);
            node.set_scroll_y(metrics.offset_y);
            node.set_scroll_y_min(0.0);
            node.set_scroll_y_max(metrics.max_y);
            match axis {
                ScrollAxis::Horizontal => {
                    node.add_action(AccessibilityAction::ScrollLeft);
                    node.add_action(AccessibilityAction::ScrollRight);
                }
                ScrollAxis::Vertical => {
                    node.add_action(AccessibilityAction::ScrollUp);
                    node.add_action(AccessibilityAction::ScrollDown);
                }
                ScrollAxis::All => {
                    node.add_action(AccessibilityAction::ScrollLeft);
                    node.add_action(AccessibilityAction::ScrollRight);
                    node.add_action(AccessibilityAction::ScrollUp);
                    node.add_action(AccessibilityAction::ScrollDown);
                }
                _ => panic!("scroll axis variant is not supported by hydrolysis"),
            }
            let _ = renderer.register_accessibility_node(
                node,
                transformed_rect(ctx.hit_transform, viewport),
                env,
                Some(AccessibilityActionTarget::Scroll {
                    handle: handle.clone(),
                    axis,
                }),
            );
        }
    }
}

impl HydroNativeView for Native<NavigationView> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_navigation_view(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_navigation_view_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(_state: &mut HydroState, ctx: RenderContext, view: &Self, env: &Environment) {
        #[cfg(feature = "accessibility")]
        {
            let navigation = view.as_inner();
            let bar = &navigation.bar;
            let renderer = unsafe { ctx.renderer() };
            let bar_hidden = renderer.read_signal(&bar.hidden);
            if bar_hidden {
                return;
            }
            let bar_height = match bar.display_mode {
                waterui::navigation::NavigationTitleDisplayMode::Automatic => {
                    NAVIGATION_BAR_HEIGHT_AUTOMATIC
                }
                waterui::navigation::NavigationTitleDisplayMode::Inline => {
                    NAVIGATION_BAR_HEIGHT_INLINE
                }
                waterui::navigation::NavigationTitleDisplayMode::Large => {
                    NAVIGATION_BAR_HEIGHT_LARGE
                }
            };
            let bar_rect = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                (ctx.bounds.y0 + bar_height).min(ctx.bounds.y1),
            );
            let mut bar_node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Navigation),
            );
            let bar_label = renderer.resolve_accessibility_label(env, None);
            if let Some(label) = bar_label {
                bar_node.set_label(label);
            }
            let title_height = if matches!(
                bar.display_mode,
                waterui::navigation::NavigationTitleDisplayMode::Large
            ) {
                NAVIGATION_TITLE_HEIGHT_LARGE
            } else {
                NAVIGATION_TITLE_HEIGHT_INLINE
            };
            let title_rect = vello::kurbo::Rect::new(
                bar_rect.x0 + NAVIGATION_BAR_HORIZONTAL_INSET,
                bar_rect.y1 - title_height - NAVIGATION_BAR_BOTTOM_INSET,
                bar_rect.x1 - NAVIGATION_BAR_HORIZONTAL_INSET,
                bar_rect.y1 - NAVIGATION_BAR_BOTTOM_INSET,
            );
            if title_rect.width() > 0.0 && title_rect.height() > 0.0 {
                let mut title_node = AccessibilityNode::new(
                    renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Header),
                );
                let default_title_label = renderer.accessibility_label_from_view(&bar.title, env);
                let title_label = renderer.resolve_accessibility_label(env, default_title_label);
                if let Some(label) = title_label {
                    title_node.set_label(label);
                }
                if let Some(title_node_id) = renderer.register_accessibility_child_node(
                    title_node,
                    transformed_rect(ctx.hit_transform, title_rect),
                    env,
                    None,
                ) {
                    bar_node.push_child(title_node_id);
                }
            }
            let _ = renderer.register_accessibility_node(
                bar_node,
                transformed_rect(ctx.hit_transform, bar_rect),
                env,
                None,
            );
        }
    }
}

impl HydroNativeView for Native<NavigationSplitLayout> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_navigation_split_layout(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let split = view.as_inner();
        let sidebar = measure_view_intrinsic(&split.sidebar, state, env);
        let detail = if let Some(detail) = split.detail.as_ref() {
            measure_navigation_view_intrinsic(detail, state, env)
        } else {
            measure_view_intrinsic(&split.placeholder, state, env)
        };
        LayoutSize::new(
            (f64::from(split.sidebar_width) + f64::from(detail.width)) as f32,
            f64::from(sidebar.height.max(detail.height)) as f32,
        )
    }

    fn accessibility(
        _state: &mut HydroState,
        _ctx: RenderContext,
        _view: &Self,
        _env: &Environment,
    ) {
    }
}

impl HydroNativeView for Native<NavigationStack<(), ()>> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_navigation_stack(state, ctx, view, env);
    }

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }

    fn accessibility(
        _state: &mut HydroState,
        ctx: RenderContext,
        _view: &Self,
        _env: &Environment,
    ) {
        let entries = {
            let renderer = unsafe { ctx.renderer() };
            let (slot_index, entries) = renderer.bind_navigation_entries();
            renderer.push_pending_navigation_entries(slot_index, Rc::clone(&entries));
            entries
        };
        let depth = entries.borrow().len();
        #[cfg(feature = "accessibility")]
        {
            if depth == 0 {
                return;
            }
            let renderer = unsafe { ctx.renderer() };
            let mut back_node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(_env, AccessibilityNodeRole::Button),
            );
            back_node.set_label("Back".to_owned());
            back_node.add_action(AccessibilityAction::Focus);
            back_node.add_action(AccessibilityAction::Click);
            let back_bounds =
                transformed_rect(ctx.hit_transform, navigation_back_button_rect(ctx.bounds));
            let _ = renderer.register_accessibility_node(
                back_node,
                back_bounds,
                _env,
                Some(AccessibilityActionTarget::PointerPrimaryClick {
                    point: accessibility_activation_point(back_bounds),
                }),
            );
        }
    }
}

impl HydroNativeView for Native<Tabs> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_tabs(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_tabs_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(_state: &mut HydroState, ctx: RenderContext, view: &Self, env: &Environment) {
        #[cfg(feature = "accessibility")]
        {
            let tabs = view.as_inner();
            assert!(
                !(tabs.tabs.is_empty()),
                "hydrolysis Tabs requires at least one tab"
            );
            let renderer = unsafe { ctx.renderer() };
            let selected_id = renderer.read_signal(&tabs.selection);
            let selected_index = tabs
                .tabs
                .iter()
                .position(|tab| tab.label.tag == selected_id)
                .unwrap_or_else(|| panic!("hydrolysis Tabs selection is not present in tabs"));
            let (bar_rect, _content_rect) = tabs_bar_and_content_rect(ctx.bounds, tabs.position);
            let mut tab_list = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::TabList),
            );
            let tab_list_label = renderer.resolve_accessibility_label(env, None);
            if let Some(label) = tab_list_label {
                tab_list.set_label(label);
            }
            for (index, tab) in tabs.tabs.iter().enumerate() {
                let mut tab_node = AccessibilityNode::new(
                    renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Tab),
                );
                let default_label = renderer.accessibility_label_from_view(&tab.label.content, env);
                let label = renderer.resolve_accessibility_label(env, default_label);
                if let Some(label) = label {
                    tab_node.set_label(label);
                }
                let is_selected = index == selected_index;
                tab_node.set_selected(is_selected);
                tab_node.add_action(AccessibilityAction::Focus);
                tab_node.add_action(AccessibilityAction::Click);
                let tab_bounds = transformed_rect(
                    ctx.hit_transform,
                    tabs_button_rect(bar_rect, tabs.tabs.len(), index),
                );
                if let Some(tab_node_id) = renderer.register_accessibility_child_node(
                    tab_node,
                    tab_bounds,
                    env,
                    Some(AccessibilityActionTarget::PickerSelect {
                        selection: tabs.selection.clone(),
                        target: tab.label.tag,
                    }),
                ) {
                    tab_list.push_child(tab_node_id);
                }
            }
            let _ = renderer.register_accessibility_node(
                tab_list,
                transformed_rect(ctx.hit_transform, bar_rect),
                env,
                None,
            );
        }
    }
}

impl HydroNativeView for Native<ListConfig> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_list(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_list_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(state: &mut HydroState, ctx: RenderContext, view: &Self, env: &Environment) {
        let list = view.as_inner();
        let row_count_signal = list.contents.len();
        let row_count = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&row_count_signal)
        };
        let slot_index = {
            let renderer = unsafe { ctx.renderer() };
            let index = renderer.lazy_list_controller.bind();
            renderer.lazy_list_controller.slots[index].prepare_len(row_count);
            index
        };
        let viewport = ctx.bounds;
        let content_height = {
            let renderer = unsafe { ctx.renderer() };
            sum_cached_or_estimated(
                &renderer.lazy_list_controller.slots[slot_index].row_extents,
                LIST_ESTIMATED_ROW_HEIGHT,
            )
            .max(viewport.height())
        };
        let handle = {
            let renderer = unsafe { ctx.renderer() };
            let handle = renderer.scroll_controller.bind(
                ScrollAxis::Vertical,
                viewport.width(),
                viewport.height(),
                viewport.width(),
                content_height,
            );
            renderer.push_pending_scroll_handle(handle.clone());
            handle
        };
        #[cfg(feature = "accessibility")]
        {
            let metrics = handle.metrics();
            let window = resolve_visible_index_window(
                row_count,
                metrics.offset_y,
                metrics.offset_y + viewport.height(),
                |index| {
                    let cached_extent = {
                        let renderer = unsafe { ctx.renderer() };
                        renderer.lazy_list_controller.slots[slot_index].row_extents[index]
                    };
                    if let Some(extent) = cached_extent {
                        return extent;
                    }
                    let (_item, extent) = materialize_list_row(&list.contents, index, state, env);
                    {
                        let renderer = unsafe { ctx.renderer() };
                        renderer.lazy_list_controller.slots[slot_index].row_extents[index] =
                            Some(extent);
                    }
                    extent
                },
            );
            let renderer = unsafe { ctx.renderer() };
            let mut list_node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::List),
            );
            let list_label = renderer.resolve_accessibility_label(env, None);
            if let Some(label) = list_label {
                list_node.set_label(label);
            }
            list_node.set_scroll_y(metrics.offset_y);
            list_node.set_scroll_y_min(0.0);
            list_node.set_scroll_y_max(metrics.max_y);
            list_node.add_action(AccessibilityAction::ScrollUp);
            list_node.add_action(AccessibilityAction::ScrollDown);
            let mut y = viewport.y0 - metrics.offset_y + window.leading_offset;
            for index in window.start..window.end {
                let item = materialize_list_item(&list.contents, index, env);
                let row_height = {
                    let cached_extent =
                        renderer.lazy_list_controller.slots[slot_index].row_extents[index];
                    if let Some(extent) = cached_extent {
                        extent
                    } else {
                        let extent = measure_list_item_row_height(&item, state, env);
                        renderer.lazy_list_controller.slots[slot_index].row_extents[index] =
                            Some(extent);
                        extent
                    }
                };
                let row_rect = vello::kurbo::Rect::new(viewport.x0, y, viewport.x1, y + row_height);
                y += row_height;
                if row_rect.y1 <= viewport.y0 || row_rect.y0 >= viewport.y1 {
                    continue;
                }
                let mut row_node = AccessibilityNode::new(
                    renderer.resolve_accessibility_role(env, AccessibilityNodeRole::ListItem),
                );
                let default_label = renderer.accessibility_label_from_view(&item.content, env);
                let label = renderer.resolve_accessibility_label(env, default_label);
                if let Some(label) = label {
                    row_node.set_label(label);
                }
                row_node.add_action(AccessibilityAction::Focus);
                if let Some(row_node_id) = renderer.register_accessibility_child_node(
                    row_node,
                    transformed_rect(ctx.hit_transform, row_rect),
                    env,
                    None,
                ) {
                    list_node.push_child(row_node_id);
                }
            }
            let _ = renderer.register_accessibility_node(
                list_node,
                transformed_rect(ctx.hit_transform, viewport),
                env,
                Some(AccessibilityActionTarget::Scroll {
                    handle: handle.clone(),
                    axis: ScrollAxis::Vertical,
                }),
            );
        }
    }
}

impl HydroNativeView for Native<ButtonConfig> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_button(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_button_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(_state: &mut HydroState, ctx: RenderContext, view: &Self, env: &Environment) {
        #[cfg(feature = "accessibility")]
        {
            let button = view.as_inner();
            let renderer = unsafe { ctx.renderer() };
            let mut node = AccessibilityNode::new(renderer.resolve_accessibility_role(
                env,
                match button.style {
                    ButtonStyle::Link => AccessibilityNodeRole::Link,
                    _ => AccessibilityNodeRole::Button,
                },
            ));
            let default_label = renderer.accessibility_label_from_view(&button.label, env);
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
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
    }
}

impl HydroNativeView for Native<ResolvedMenu> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_menu(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_menu_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(_state: &mut HydroState, ctx: RenderContext, view: &Self, env: &Environment) {
        #[cfg(feature = "accessibility")]
        {
            let menu = view.as_inner();
            let renderer = unsafe { ctx.renderer() };
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Button),
            );
            let default_label = Some(
                renderer
                    .read_signal(&menu.accessibility_label)
                    .to_plain()
                    .to_string(),
            );
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
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
    }
}

impl HydroNativeView for Native<ToggleConfig> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_toggle(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_toggle_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(_state: &mut HydroState, ctx: RenderContext, view: &Self, env: &Environment) {
        #[cfg(feature = "accessibility")]
        {
            let toggle = view.as_inner();
            let renderer = unsafe { ctx.renderer() };
            let mut node = AccessibilityNode::new(renderer.resolve_accessibility_role(
                env,
                match toggle.style {
                    ToggleStyle::Automatic | ToggleStyle::Switch => AccessibilityNodeRole::Switch,
                    ToggleStyle::Checkbox => AccessibilityNodeRole::CheckBox,
                    _ => panic!("hydrolysis ToggleStyle variant is not implemented"),
                },
            ));
            let default_label = renderer.accessibility_label_from_view(&toggle.label, env);
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            let checked = renderer.read_signal(&toggle.toggle);
            node.set_toggled(AccessibilityToggled::from(checked));
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            let _ = renderer.register_accessibility_node(
                node,
                bounds,
                env,
                Some(AccessibilityActionTarget::Toggle {
                    binding: toggle.toggle.clone(),
                }),
            );
        }
    }
}

impl HydroNativeView for Native<StepperConfig> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_stepper(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_stepper_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(_state: &mut HydroState, ctx: RenderContext, view: &Self, env: &Environment) {
        #[cfg(feature = "accessibility")]
        {
            let stepper = view.as_inner();
            let renderer = unsafe { ctx.renderer() };
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::SpinButton),
            );
            let default_label = renderer.accessibility_label_from_view(&stepper.label, env);
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            let start = *stepper.range.start();
            let end = *stepper.range.end();
            assert!(
                !(start > end),
                "hydrolysis stepper requires an ordered range"
            );
            let current = stepper.value.get().clamp(start, end);
            node.set_numeric_value(f64::from(current));
            node.set_min_numeric_value(f64::from(start));
            node.set_max_numeric_value(f64::from(end));
            let step = stepper.step.get();
            assert!(!(step <= 0), "hydrolysis stepper requires positive step");
            node.set_numeric_value_step(f64::from(step));
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Increment);
            node.add_action(AccessibilityAction::Decrement);
            node.add_action(AccessibilityAction::SetValue);
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            let _ = renderer.register_accessibility_node(
                node,
                bounds,
                env,
                Some(AccessibilityActionTarget::Stepper {
                    value: stepper.value.clone(),
                    step: stepper.step.clone(),
                    range: stepper.range.clone(),
                }),
            );
        }
    }
}

impl HydroNativeView for Native<ProgressConfig> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_progress(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_progress_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(_state: &mut HydroState, ctx: RenderContext, view: &Self, env: &Environment) {
        #[cfg(feature = "accessibility")]
        {
            let progress = view.as_inner();
            let renderer = unsafe { ctx.renderer() };
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::ProgressIndicator),
            );
            let default_label = renderer.accessibility_label_from_view(&progress.label, env);
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            let value = renderer.read_signal(&progress.value).clamp(0.0, 1.0);
            node.set_numeric_value(value);
            node.set_min_numeric_value(0.0);
            node.set_max_numeric_value(1.0);
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            let _ = renderer.register_accessibility_node(node, bounds, env, None);
        }
    }
}

impl HydroNativeView for Native<ResolvedTextFieldConfig> {
    fn accessibility_is_render_driven() -> bool {
        true
    }

    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_text_field(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_text_field_intrinsic(view.as_inner(), state, env)
    }
    fn accessibility(_state: &mut HydroState, ctx: RenderContext, view: &Self, env: &Environment) {
        #[cfg(feature = "accessibility")]
        {
            let text_field = view.as_inner();
            let renderer = unsafe { ctx.renderer() };
            let line_limit = text_field.line_limit.map(NonZeroUsize::get);
            let mut node = AccessibilityNode::new(renderer.resolve_accessibility_role(
                env,
                if line_limit == Some(1) {
                    AccessibilityNodeRole::TextInput
                } else {
                    AccessibilityNodeRole::MultilineTextInput
                },
            ));
            let prompt_signal = text_field.prompt.content.clone();
            let prompt = renderer.read_signal(&prompt_signal).to_plain().to_string();
            let default_label = renderer
                .accessibility_label_from_view(&text_field.label, env)
                .or_else(|| (!prompt.is_empty()).then_some(prompt.clone()));
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            if !prompt.is_empty() {
                node.set_placeholder(prompt);
            }
            let value = renderer
                .read_signal(&text_field.value)
                .to_plain()
                .to_string();
            if !value.is_empty() {
                node.set_value(value.clone());
            }
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            if let Some(text_run_node_id) =
                register_accessibility_text_run_node(renderer, &value, bounds, env)
            {
                node.set_children(vec![text_run_node_id]);
                node.set_text_selection(collapsed_accessibility_text_selection(
                    text_run_node_id,
                    value.chars().count(),
                ));
            }
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            node.add_action(AccessibilityAction::SetValue);
            let _ = renderer.register_accessibility_node(
                node,
                bounds,
                env,
                Some(AccessibilityActionTarget::TextField {
                    value: text_field.value.clone(),
                    line_limit,
                }),
            );
        }
    }
}

impl HydroNativeView for Native<SecureFieldConfig> {
    fn accessibility_is_render_driven() -> bool {
        true
    }

    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_secure_field(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_secure_field_intrinsic(view.as_inner(), state, env)
    }
    fn accessibility(_state: &mut HydroState, ctx: RenderContext, view: &Self, env: &Environment) {
        #[cfg(feature = "accessibility")]
        {
            let secure_field = view.as_inner();
            let renderer = unsafe { ctx.renderer() };
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::PasswordInput),
            );
            let default_label = renderer.accessibility_label_from_view(&secure_field.label, env);
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            let secure_len = renderer
                .read_signal(&secure_field.value)
                .expose()
                .chars()
                .count();
            let masked_value = "*".repeat(secure_len);
            node.set_value(masked_value.clone());
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            if let Some(text_run_node_id) =
                register_accessibility_text_run_node(renderer, &masked_value, bounds, env)
            {
                node.set_children(vec![text_run_node_id]);
                node.set_text_selection(collapsed_accessibility_text_selection(
                    text_run_node_id,
                    masked_value.chars().count(),
                ));
            }
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            node.add_action(AccessibilityAction::SetValue);
            let _ = renderer.register_accessibility_node(
                node,
                bounds,
                env,
                Some(AccessibilityActionTarget::SecureField {
                    value: secure_field.value.clone(),
                }),
            );
        }
    }
}

impl HydroNativeView for Native<Dynamic> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_dynamic(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let dynamic = view.as_inner();
        let identity = dynamic.identity();
        if let Some(dimensions) = state.dynamic_intrinsic_cache.get(&identity) {
            return dimensions.size;
        }
        let initial = dynamic.with_unconnected_view_mut(|slot| {
            slot.take().map(|content| {
                let normalized = normalize_layout_view(content, env);
                let dimensions = measure_view_dimensions(&normalized, state, env);
                *slot = Some(normalized);
                dimensions
            })
        });
        let dimensions = match initial {
            Some(Some(dimensions)) => dimensions,
            Some(None) => {
                panic!("hydrolysis Dynamic intrinsic requires an initial view before layout")
            }
            None => state
                .dynamic_intrinsic_cache
                .get(&identity)
                .cloned()
                .unwrap_or_else(|| {
                    panic!("hydrolysis Dynamic intrinsic cache miss for connected dynamic node")
                }),
        };
        state
            .dynamic_intrinsic_cache
            .insert(identity, dimensions.clone());
        dimensions.size
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let dynamic = view.as_inner();
        let identity = dynamic.identity();
        if proposal == ProposalSize::UNSPECIFIED
            && let Some(dimensions) = state.dynamic_intrinsic_cache.get(&identity)
        {
            return dimensions.clone();
        }

        let initial = dynamic.with_unconnected_view_mut(|slot| {
            slot.take().map(|content| {
                let normalized = normalize_layout_view(content, env);
                let dimensions =
                    measure_view_dimensions_with_proposal(&normalized, proposal, state, env);
                *slot = Some(normalized);
                dimensions
            })
        });
        let dimensions = match initial {
            Some(Some(dimensions)) => dimensions,
            Some(None) => {
                panic!("hydrolysis Dynamic dimensions requires an initial view before layout")
            }
            None => state
                .dynamic_intrinsic_cache
                .get(&identity)
                .cloned()
                .unwrap_or_else(|| {
                    panic!("hydrolysis Dynamic dimensions cache miss for connected dynamic node")
                }),
        };
        state
            .dynamic_intrinsic_cache
            .insert(identity, dimensions.clone());
        dimensions
    }
}

impl HydroNativeView for Native<SystemIcon> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_system_icon(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        HydrolysisRenderer::measure_text_intrinsic_size(
            state,
            StyledStr::plain(view.as_inner().name.clone()),
            env,
        )
    }

    fn accessibility(
        _state: &mut HydroState,
        _ctx: RenderContext,
        _view: &Self,
        _env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let icon = _view.as_inner();
            let renderer = unsafe { _ctx.renderer() };
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(_env, AccessibilityNodeRole::Image),
            );
            let label =
                renderer.resolve_accessibility_label(_env, Some(icon.name.as_str().to_owned()));
            if let Some(label) = label {
                node.set_label(label);
            }
            let _ = renderer.register_accessibility_node(
                node,
                transformed_rect(_ctx.hit_transform, _ctx.bounds),
                _env,
                None,
            );
        }
    }
}

impl HydroNativeView for Native<TableConfig> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_table(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let columns = view.as_inner().columns.get();
        if columns.is_empty() {
            return LayoutSize::zero();
        }
        let metrics = measure_table_metrics(&columns, state, env);
        LayoutSize::new(metrics.table_width as f32, metrics.table_height as f32)
    }

    fn accessibility(state: &mut HydroState, ctx: RenderContext, view: &Self, env: &Environment) {
        let columns = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&view.as_inner().columns)
        };
        if columns.is_empty() {
            return;
        }
        let slot_index = {
            let renderer = unsafe { ctx.renderer() };
            let index = renderer.lazy_table_controller.bind();
            index
        };
        {
            let renderer = unsafe { ctx.renderer() };
            let slot = &mut renderer.lazy_table_controller.slots[slot_index];
            refresh_table_slot_baseline(&columns, slot, state, env);
        }
        let viewport = ctx.bounds;
        let table_metrics = {
            let renderer = unsafe { ctx.renderer() };
            let slot = &renderer.lazy_table_controller.slots[slot_index];
            table_metrics_from_slot(slot)
        };
        let handle = {
            let renderer = unsafe { ctx.renderer() };
            let handle = renderer.scroll_controller.bind(
                ScrollAxis::All,
                viewport.width(),
                viewport.height(),
                table_metrics.table_width.max(viewport.width()),
                table_metrics.table_height.max(viewport.height()),
            );
            renderer.push_pending_scroll_handle(handle.clone());
            handle
        };
        #[cfg(feature = "accessibility")]
        {
            let scroll_metrics = handle.metrics();
            let row_window = {
                let renderer = unsafe { ctx.renderer() };
                let slot = &renderer.lazy_table_controller.slots[slot_index];
                resolve_table_visible_rows(
                    scroll_metrics.offset_y,
                    viewport.height(),
                    slot.max_rows,
                )
            };
            let mut column_window = {
                let renderer = unsafe { ctx.renderer() };
                let slot = &renderer.lazy_table_controller.slots[slot_index];
                resolve_visible_column_window(
                    &slot.column_widths,
                    scroll_metrics.offset_x,
                    scroll_metrics.offset_x + viewport.width(),
                )
            };
            {
                let renderer = unsafe { ctx.renderer() };
                let slot = &mut renderer.lazy_table_controller.slots[slot_index];
                update_table_slot_visible_cell_widths(
                    &columns,
                    slot,
                    row_window,
                    column_window,
                    state,
                    env,
                );
                column_window = resolve_visible_column_window(
                    &slot.column_widths,
                    scroll_metrics.offset_x,
                    scroll_metrics.offset_x + viewport.width(),
                );
            }
            let renderer = unsafe { ctx.renderer() };
            let mut table_node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Table),
            );
            let table_label = renderer.resolve_accessibility_label(env, None);
            if let Some(label) = table_label {
                table_node.set_label(label);
            }
            table_node.set_scroll_x(scroll_metrics.offset_x);
            table_node.set_scroll_x_min(0.0);
            table_node.set_scroll_x_max(scroll_metrics.max_x);
            table_node.set_scroll_y(scroll_metrics.offset_y);
            table_node.set_scroll_y_min(0.0);
            table_node.set_scroll_y_max(scroll_metrics.max_y);
            table_node.add_action(AccessibilityAction::ScrollLeft);
            table_node.add_action(AccessibilityAction::ScrollRight);
            table_node.add_action(AccessibilityAction::ScrollUp);
            table_node.add_action(AccessibilityAction::ScrollDown);

            let origin_x = viewport.x0 - scroll_metrics.offset_x;
            let origin_y = viewport.y0 - scroll_metrics.offset_y;
            let mut x_offset = column_window.leading_offset;
            for column_index in column_window.start..column_window.end {
                let column = &columns[column_index];
                let width =
                    renderer.lazy_table_controller.slots[slot_index].column_widths[column_index];
                let header_cell = table_header_cell_rect(origin_x, origin_y, x_offset, width);
                let header_view = AnyView::new(column.label());
                let mut header_node = AccessibilityNode::new(
                    renderer.resolve_accessibility_role(env, AccessibilityNodeRole::ColumnHeader),
                );
                let default_label = renderer.accessibility_label_from_view(&header_view, env);
                let label = renderer.resolve_accessibility_label(env, default_label);
                if let Some(label) = label {
                    header_node.set_label(label);
                }
                header_node.add_action(AccessibilityAction::Focus);
                if let Some(header_node_id) = renderer.register_accessibility_child_node(
                    header_node,
                    transformed_rect(ctx.hit_transform, header_cell),
                    env,
                    None,
                ) {
                    table_node.push_child(header_node_id);
                }
                let rows = column.rows();
                for row_index in row_window.start..row_window.end {
                    let cell_rect =
                        table_data_cell_rect(origin_x, origin_y, x_offset, width, row_index);
                    if let Some(cell) = rows.get_view(row_index) {
                        let cell_view = AnyView::new(cell);
                        let mut cell_node = AccessibilityNode::new(
                            renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Cell),
                        );
                        let default_label = renderer.accessibility_label_from_view(&cell_view, env);
                        let label = renderer.resolve_accessibility_label(env, default_label);
                        if let Some(label) = label {
                            cell_node.set_label(label);
                        }
                        cell_node.add_action(AccessibilityAction::Focus);
                        if let Some(cell_node_id) = renderer.register_accessibility_child_node(
                            cell_node,
                            transformed_rect(ctx.hit_transform, cell_rect),
                            env,
                            None,
                        ) {
                            table_node.push_child(cell_node_id);
                        }
                    }
                }
                x_offset += width;
            }

            let _ = renderer.register_accessibility_node(
                table_node,
                transformed_rect(ctx.hit_transform, viewport),
                env,
                Some(AccessibilityActionTarget::Scroll {
                    handle: handle.clone(),
                    axis: ScrollAxis::All,
                }),
            );
        }
    }
}

impl HydroNativeView for Native<SliderConfig> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_slider(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_slider_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(_state: &mut HydroState, ctx: RenderContext, view: &Self, env: &Environment) {
        #[cfg(feature = "accessibility")]
        {
            let slider = view.as_inner();
            let renderer = unsafe { ctx.renderer() };
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Slider),
            );
            let default_label = renderer.accessibility_label_from_view(&slider.label, env);
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            let start = *slider.range.start();
            let end = *slider.range.end();
            assert!(
                !(start >= end),
                "hydrolysis slider requires range start < end"
            );
            let current = renderer.read_signal(&slider.value).clamp(start, end);
            node.set_numeric_value(current);
            node.set_min_numeric_value(start);
            node.set_max_numeric_value(end);
            node.set_numeric_value_step(slider_step_for_range(slider.range.clone()));
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Increment);
            node.add_action(AccessibilityAction::Decrement);
            node.add_action(AccessibilityAction::SetValue);
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            let _ = renderer.register_accessibility_node(
                node,
                bounds,
                env,
                Some(AccessibilityActionTarget::Slider {
                    value: slider.value.clone(),
                    range: slider.range.clone(),
                    step: slider_step_for_range(slider.range.clone()),
                }),
            );
        }
    }
}

impl HydroNativeView for Native<PickerConfig> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_picker(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_picker_intrinsic(view.as_inner(), state, env)
    }

    fn accessibility(state: &mut HydroState, ctx: RenderContext, view: &Self, env: &Environment) {
        #[cfg(feature = "accessibility")]
        {
            let picker = view.as_inner();
            let renderer = unsafe { ctx.renderer() };
            let items = renderer.read_signal(&picker.items);
            assert!(
                !(items.is_empty()),
                "hydrolysis picker requires at least one item"
            );
            match picker.style {
                PickerStyle::Automatic | PickerStyle::Menu => {
                    let selected = renderer.read_signal(&picker.selection);
                    let selected_index = items
                        .iter()
                        .position(|item| item.tag == selected)
                        .unwrap_or_else(|| {
                            panic!("hydrolysis picker selection is not present in picker items")
                        });
                    let mut option_labels = Vec::with_capacity(items.len());
                    let mut max_item_text_height: f64 = 0.0;
                    for item in &items {
                        let label_signal = item.content.content();
                        let label = renderer.read_signal(&label_signal).to_plain();
                        let label_size = HydrolysisRenderer::measure_text_intrinsic_size(
                            state,
                            StyledStr::plain(label.clone()),
                            env,
                        );
                        max_item_text_height =
                            max_item_text_height.max(f64::from(label_size.height));
                        option_labels.push(label);
                    }
                    let selected_text = option_labels[selected_index].clone();
                    let mut node = AccessibilityNode::new(
                        renderer.resolve_accessibility_role(env, AccessibilityNodeRole::ComboBox),
                    );
                    let label = renderer
                        .resolve_accessibility_label(env, Some(selected_text.as_str().to_owned()));
                    if let Some(label) = label {
                        node.set_label(label);
                    }
                    node.set_value(selected_text.as_str().to_owned());
                    node.add_action(AccessibilityAction::Focus);
                    node.add_action(AccessibilityAction::Click);
                    let row_height = menu_picker_row_height(ctx.bounds, max_item_text_height);
                    let popup_rect = menu_picker_popup_rect(ctx.bounds, row_height, items.len());
                    for (index, item) in items.iter().enumerate() {
                        let mut option =
                            AccessibilityNode::new(renderer.resolve_accessibility_role(
                                env,
                                AccessibilityNodeRole::ListBoxOption,
                            ));
                        option.set_label(option_labels[index].as_str().to_owned());
                        let is_selected = item.tag == selected;
                        option.set_selected(is_selected);
                        option.set_toggled(AccessibilityToggled::from(is_selected));
                        option.add_action(AccessibilityAction::Focus);
                        option.add_action(AccessibilityAction::Click);
                        let option_bounds = transformed_rect(
                            ctx.hit_transform,
                            menu_picker_option_rect(popup_rect, row_height, index),
                        );
                        if let Some(option_id) = renderer.register_accessibility_child_node(
                            option,
                            option_bounds,
                            env,
                            Some(AccessibilityActionTarget::PickerSelect {
                                selection: picker.selection.clone(),
                                target: item.tag,
                            }),
                        ) {
                            node.push_child(option_id);
                        }
                    }
                    let ids = items.iter().map(|item| item.tag).collect::<Vec<_>>();
                    let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
                    let _ = renderer.register_accessibility_node(
                        node,
                        bounds,
                        env,
                        Some(AccessibilityActionTarget::PickerCycle {
                            selection: picker.selection.clone(),
                            ids,
                        }),
                    );
                }
                PickerStyle::Radio => {
                    let mut group = AccessibilityNode::new(
                        renderer.resolve_accessibility_role(env, AccessibilityNodeRole::Group),
                    );
                    let group_label = renderer.resolve_accessibility_label(env, None);
                    if let Some(label) = group_label {
                        group.set_label(label);
                    }
                    let group_bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
                    let mut row_y = ctx.bounds.y0 + PICKER_VERTICAL_INSET;
                    let selected = renderer.read_signal(&picker.selection);
                    for item in &items {
                        let label_signal = item.content.content();
                        let label = renderer.read_signal(&label_signal).to_plain().to_string();
                        let label_size = HydrolysisRenderer::measure_text_intrinsic_size(
                            state,
                            StyledStr::plain(label.clone()),
                            env,
                        );
                        let row_height =
                            f64::from(label_size.height).max(PICKER_RADIO_INDICATOR_SIZE);
                        let row_rect = vello::kurbo::Rect::new(
                            ctx.bounds.x0,
                            row_y,
                            ctx.bounds.x1,
                            (row_y + row_height).min(ctx.bounds.y1),
                        );
                        if row_rect.height() <= 0.0 {
                            break;
                        }
                        row_y = row_rect.y1 + PICKER_RADIO_ROW_SPACING;
                        let mut option =
                            AccessibilityNode::new(renderer.resolve_accessibility_role(
                                env,
                                AccessibilityNodeRole::RadioButton,
                            ));
                        option.set_label(label);
                        let is_selected = item.tag == selected;
                        option.set_toggled(AccessibilityToggled::from(is_selected));
                        option.set_selected(is_selected);
                        option.add_action(AccessibilityAction::Focus);
                        option.add_action(AccessibilityAction::Click);
                        let row_bounds = transformed_rect(ctx.hit_transform, row_rect);
                        if let Some(child_id) = renderer.register_accessibility_child_node(
                            option,
                            row_bounds,
                            env,
                            Some(AccessibilityActionTarget::PickerSelect {
                                selection: picker.selection.clone(),
                                target: item.tag,
                            }),
                        ) {
                            group.push_child(child_id);
                        }
                    }
                    let _ = renderer.register_accessibility_node(group, group_bounds, env, None);
                }
                _ => panic!("hydrolysis PickerStyle variant is not implemented"),
            }
        }
    }
}

impl HydroNativeView for Native<GpuSurface> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_gpu_surface(state, ctx, view, env);
    }

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }

    fn accessibility(
        _state: &mut HydroState,
        _ctx: RenderContext,
        _view: &Self,
        _env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let renderer = unsafe { _ctx.renderer() };
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(_env, AccessibilityNodeRole::Image),
            );
            if let Some(label) = renderer.resolve_accessibility_label(_env, None) {
                node.set_label(label);
            }
            let _ = renderer.register_accessibility_node(
                node,
                transformed_rect(_ctx.hit_transform, _ctx.bounds),
                _env,
                None,
            );
        }
    }
}

impl HydroNativeView for Native<SceneView> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_scene_view(state, ctx, view, env);
    }

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }

    fn accessibility(
        _state: &mut HydroState,
        _ctx: RenderContext,
        _view: &Self,
        _env: &Environment,
    ) {
        #[cfg(feature = "accessibility")]
        {
            let renderer = unsafe { _ctx.renderer() };
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(_env, AccessibilityNodeRole::Image),
            );
            if let Some(label) = renderer.resolve_accessibility_label(_env, None) {
                node.set_label(label);
            }
            let _ = renderer.register_accessibility_node(
                node,
                transformed_rect(_ctx.hit_transform, _ctx.bounds),
                _env,
                None,
            );
        }
    }
}

impl HydroNativeView for Native<ViewEffectErased> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_view_effect(state, ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_view_intrinsic(view.as_inner().content(), state, env)
    }
}

impl HydroNativeView for Native<ResolvedColor> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_resolved_color(state, ctx, view, env);
    }

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }
}

impl HydroNativeView for Native<ResolvedGradient> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_resolved_gradient(state, ctx, view, env);
    }

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }
}

impl HydroNativeView for Native<ResolvedShape> {
    fn render(state: &mut HydroState, ctx: RenderContext, view: Self, env: &Environment) {
        HydrolysisRenderer::render_resolved_shape(state, ctx, view, env);
    }

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }
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
        let mut dispatcher = ViewDispatcher::with_state(HydroState::default());
        Self::register_core_handlers(&mut dispatcher);

        let vello_renderer =
            vello::Renderer::new(device, options).expect("failed to create hydrolysis renderer");
        Self {
            dispatcher,
            vello_renderer,
            scene: vello::Scene::new(),
            vello_layer_surface: None,
            gpu_surface_compositor: None,
            gpu_surface_slots: Vec::new(),
            gpu_surface_cursor: 0,
            render_layers: Vec::new(),
            active_scene_layers: Vec::new(),
            active_filter_images: Vec::new(),
            pointer_targets: Vec::new(),
            active_pointer_drag_target: None,
            active_pointer_drag_signature: None,
            gesture_engine: GestureEngine::default(),
            gesture_group_ids: BTreeMap::new(),
            next_gesture_group_id: 0,
            cursor_targets: Vec::new(),
            hover_targets: Vec::new(),
            context_menu_targets: Vec::new(),
            text_input_targets: Vec::new(),
            text_selection_slots: Vec::new(),
            text_selection_cursor: 0,
            active_text_selection_drag: None,
            last_text_selection_click: None,
            active_text_context_menu: None,
            active_popup_menu_group: None,
            scroll_targets: Vec::new(),
            hit_test_opacity: 1.0,
            render_depth: 0,
            hit_test_order: 0,
            window_bounds: vello::kurbo::Rect::ZERO,
            focused_text_input: Cell::new(None),
            ime_preedit: None,
            text_caret_fade_started_at: None,
            text_caret_next_frame_at: None,
            lifecycle_disappear_previous: BTreeMap::new(),
            lifecycle_disappear_current: BTreeMap::new(),
            lifecycle_disappear_slot: 0,
            redraw_requested: Rc::new(Cell::new(false)),
            rebuild_requested: Rc::new(Cell::new(false)),
            dynamic_nodes: BTreeMap::new(),
            animation_controller: AnimationController::default(),
            scroll_controller: ScrollController::default(),
            hover_controller: HoverController::default(),
            lazy_stack_controller: LazyStackController::default(),
            lazy_list_controller: LazyListController::default(),
            lazy_table_controller: LazyTableController::default(),
            lazy_viewport_stack: Vec::new(),
            pending_scroll_handles: Vec::new(),
            navigation_slots: Vec::new(),
            pending_navigation_entries: Vec::new(),
            navigation_cursor: 0,
            dynamic_identities_current_frame: Vec::new(),
            picker_menu_slots: Vec::new(),
            picker_menu_cursor: 0,
            local_state_registry: Rc::new(RefCell::new(LocalStateRegistry::default())),
            current_frame_retain: Vec::new(),
            previous_frame_retain: Vec::new(),
            #[cfg(feature = "accessibility")]
            accessibility_nodes: Vec::new(),
            #[cfg(feature = "accessibility")]
            accessibility_root_children: Vec::new(),
            #[cfg(feature = "accessibility")]
            accessibility_actions: BTreeMap::new(),
            #[cfg(feature = "accessibility")]
            accessibility_next_node_id: ACCESSIBILITY_FIRST_NODE_ID,
            #[cfg(feature = "accessibility")]
            accessibility_root_bounds: vello::kurbo::Rect::ZERO,
            #[cfg(feature = "accessibility")]
            accessibility_root_label: String::from("WaterUI Window"),
            #[cfg(feature = "accessibility")]
            accessibility_focus: ACCESSIBILITY_ROOT_NODE_ID,
            #[cfg(feature = "accessibility")]
            pending_text_input_accessibility_nodes: VecDeque::new(),
            #[cfg(feature = "accessibility")]
            accessibility_suppression_depth: 0,
            #[cfg(feature = "accessibility")]
            pending_accessibility_tree_update: None,
        }
    }

    fn register_core_handlers(dispatcher: &mut ViewDispatcher<HydroState, RenderContext, ()>) {
        dispatcher.register::<Str>(Self::render_str);
        dispatcher.register::<Divider>(Self::render_divider);
        macro_rules! register_native {
            ($ty:ty) => {
                register_native_view::<$ty>(dispatcher);
            };
        }
        hydro_native_view_types!(register_native);

        dispatcher.register::<Metadata<Environment>>(Self::render_environment_metadata);
        dispatcher.register::<Metadata<Retain>>(Self::render_retain_metadata);
        dispatcher.register::<Metadata<Opacity>>(Self::render_opacity_metadata);
        dispatcher.register::<Metadata<AppliedFilter>>(Self::render_applied_filter_metadata);
        dispatcher.register::<Metadata<Scale>>(Self::render_scale_metadata);
        dispatcher.register::<Metadata<Rotation>>(Self::render_rotation_metadata);
        dispatcher.register::<Metadata<Offset>>(Self::render_offset_metadata);
        dispatcher.register::<Metadata<ClipShape>>(Self::render_clip_shape_metadata);
        dispatcher.register::<Metadata<Border>>(Self::render_border_metadata);
        dispatcher.register::<Metadata<Shadow>>(Self::render_shadow_metadata);
        dispatcher.register::<Metadata<Focused>>(Self::render_focused_metadata);
        dispatcher.register::<Metadata<Hittable>>(Self::render_hittable_metadata);
        dispatcher.register::<Metadata<Cursor>>(Self::render_cursor_metadata);
        dispatcher.register::<Metadata<GestureObserver>>(Self::render_gesture_observer_metadata);
        dispatcher.register::<Metadata<LifeCycleHook>>(Self::render_lifecycle_hook_metadata);
        dispatcher.register::<Metadata<OnEvent>>(Self::render_on_event_metadata);

        Self::register_passthrough_metadata::<Secure>(dispatcher);
        Self::register_passthrough_metadata::<StandardDynamicRange>(dispatcher);
        Self::register_passthrough_metadata::<HighDynamicRange>(dispatcher);
        Self::register_passthrough_metadata::<IgnoreSafeArea>(dispatcher);
        Self::register_passthrough_metadata::<ContextMenu>(dispatcher);
        dispatcher.register::<Metadata<ResolvedContextMenu>>(Self::render_context_menu_metadata);
        Self::register_passthrough_metadata::<Draggable>(dispatcher);
        Self::register_passthrough_metadata::<DropDestination>(dispatcher);
        Self::register_passthrough_metadata::<Background>(dispatcher);

        Self::register_passthrough_ignorable_metadata::<MaterialBackground>(dispatcher);
        dispatcher.register::<IgnorableMetadata<AccessibilityLabel>>(
            Self::render_accessibility_label_metadata,
        );
        dispatcher.register::<IgnorableMetadata<AccessibilityRole>>(
            Self::render_accessibility_role_metadata,
        );
        dispatcher.register::<IgnorableMetadata<AccessibilityHidden>>(
            Self::render_accessibility_hidden_metadata,
        );
        dispatcher.register::<IgnorableMetadata<AccessibilityChildren>>(
            Self::render_accessibility_children_metadata,
        );
        dispatcher.register::<IgnorableMetadata<AccessibilityState>>(
            Self::render_accessibility_state_metadata,
        );
    }

    fn register_passthrough_metadata<T: MetadataKey>(
        dispatcher: &mut ViewDispatcher<HydroState, RenderContext, ()>,
    ) {
        dispatcher.register::<Metadata<T>>(Self::render_passthrough_metadata::<T>);
    }

    fn register_passthrough_ignorable_metadata<T: MetadataKey>(
        dispatcher: &mut ViewDispatcher<HydroState, RenderContext, ()>,
    ) {
        dispatcher
            .register::<IgnorableMetadata<T>>(Self::render_passthrough_ignorable_metadata::<T>);
    }

    fn reset_text_caret_animation(&mut self, now: Instant) {
        self.text_caret_fade_started_at = Some(now);
        self.text_caret_next_frame_at = Some(
            now.checked_add(INPUT_CARET_FADE_FRAME_INTERVAL)
                .expect("hydrolysis text caret frame timestamp overflow"),
        );
    }

    fn clear_text_caret_animation(&mut self) {
        self.text_caret_fade_started_at = None;
        self.text_caret_next_frame_at = None;
    }

    fn advance_text_caret_animation(&mut self, now: Instant) -> bool {
        if self.focused_text_input.get().is_none() {
            return false;
        }
        let mut next = self.text_caret_next_frame_at.unwrap_or_else(|| {
            self.reset_text_caret_animation(now);
            self.text_caret_next_frame_at
                .expect("hydrolysis text caret animation state missing next frame timestamp")
        });
        if now < next {
            return false;
        }
        while now >= next {
            next = next
                .checked_add(INPUT_CARET_FADE_FRAME_INTERVAL)
                .expect("hydrolysis text caret frame timestamp overflow");
        }
        self.text_caret_next_frame_at = Some(next);
        true
    }

    fn text_caret_opacity(&self, now: Instant) -> f32 {
        if self.focused_text_input.get().is_none() {
            return 0.0;
        }
        let started = self.text_caret_fade_started_at.unwrap_or(now);
        let elapsed = now.saturating_duration_since(started);
        let cycle_secs = INPUT_CARET_FADE_CYCLE_DURATION.as_secs_f32();
        assert!(
            !(cycle_secs <= 0.0),
            "hydrolysis text caret fade cycle duration must be > 0"
        );
        let phase = (elapsed.as_secs_f32() / cycle_secs).fract();
        let wave = ((core::f32::consts::TAU * phase).cos() + 1.0) * 0.5;
        INPUT_CARET_MIN_OPACITY + (1.0 - INPUT_CARET_MIN_OPACITY) * wave
    }

    fn set_focused_text_input(&mut self, focused: Option<usize>) -> bool {
        let previous = self.focused_text_input.get();
        if previous == focused {
            return false;
        }
        let previous_binding = previous
            .and_then(|index| self.text_input_targets.as_slice().get(index))
            .and_then(|target| target.focus_binding.clone());
        let next_binding = focused
            .and_then(|index| self.text_input_targets.as_slice().get(index))
            .and_then(|target| target.focus_binding.clone());
        if let Some(binding) = previous_binding {
            binding.set(false);
        }
        self.focused_text_input.set(focused);
        if let Some(binding) = next_binding {
            binding.set(true);
        }
        self.active_text_selection_drag = None;
        self.ime_preedit = None;
        if focused.is_some() {
            self.reset_text_caret_animation(Instant::now());
        } else {
            self.clear_text_caret_animation();
            self.dismiss_active_text_context_menu();
            self.dismiss_active_popup_menu();
        }
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            previous_focus = ?previous,
            next_focus = ?focused,
            "text input focus changed"
        );
        true
    }

    fn target_hit_priority(depth: usize, order: usize, index: usize) -> (usize, usize, usize) {
        (order, depth, index)
    }

    fn topmost_text_input_index_at_point(&self, point: vello::kurbo::Point) -> Option<usize> {
        self.text_input_targets
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

    fn dismiss_active_text_context_menu(&mut self) {
        if let Some(menu) = self.active_text_context_menu.take() {
            if let ActiveTextContextMenu::NativeWindow { state, .. } = menu {
                state.set(WindowState::Closed);
            }
        }
    }

    fn dismiss_active_popup_menu(&mut self) {
        if let Some(group) = self.active_popup_menu_group.take() {
            group.close_all();
        }
    }

    fn topmost_context_menu_target_at_point(
        &self,
        point: vello::kurbo::Point,
    ) -> Option<ContextMenuTarget> {
        self.context_menu_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.bounds.contains(point))
            .max_by(|(left_index, left), (right_index, right)| {
                Self::target_hit_priority(left.depth, left.order, *left_index).cmp(
                    &Self::target_hit_priority(right.depth, right.order, *right_index),
                )
            })
            .map(|(_, target)| target.clone())
    }

    fn show_popup_menu_nodes(
        &mut self,
        nodes: Vec<PopupMenuNode>,
        origin: LayoutPoint,
        env: &Environment,
    ) -> bool {
        if nodes.is_empty() {
            return false;
        }
        self.dismiss_active_popup_menu();
        let group = PopupMenuStateGroup::new();
        let (window, state) = popup_menu_window(nodes, origin, group.clone(), 0);
        group.push(state);
        env.get::<WindowManager>()
            .expect("hydrolysis popup menus require WindowManager in environment")
            .show(window);
        self.active_popup_menu_group = Some(group);
        true
    }

    fn register_context_menu_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        items: nami::Computed<Vec<ResolvedMenuItem>>,
    ) {
        if self.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let order = self.next_hit_test_order();
        self.context_menu_targets.push(ContextMenuTarget {
            bounds,
            depth: self.render_depth,
            order,
            items,
        });
    }

    fn active_text_context_menu_target(&self) -> Option<usize> {
        self.active_text_context_menu
            .as_ref()
            .map(|menu| match menu {
                ActiveTextContextMenu::Overlay { index, .. }
                | ActiveTextContextMenu::NativeWindow { index, .. } => *index,
            })
    }

    pub(crate) fn set_window_bounds(&mut self, bounds: vello::kurbo::Rect) {
        self.window_bounds = bounds;
    }

    pub(crate) fn render_active_text_context_menu_overlay(
        &mut self,
        env: &Environment,
        transform: vello::kurbo::Affine,
    ) {
        let Some(ActiveTextContextMenu::Overlay { overlay, .. }) =
            self.active_text_context_menu.clone()
        else {
            return;
        };

        let renderer_ptr = self as *mut HydrolysisRenderer;
        let scene = &mut self.scene;
        let panel =
            vello::kurbo::RoundedRect::from_rect(overlay.bounds, TEXT_CONTEXT_MENU_CORNER_RADIUS);
        scene.fill(
            vello::peniko::Fill::NonZero,
            transform,
            vello::peniko::Color::new([0.98, 0.98, 0.99, 1.0]),
            None,
            &panel,
        );
        scene.stroke(
            &vello::kurbo::Stroke::new(1.0),
            transform,
            vello::peniko::Color::new([0.8, 0.82, 0.86, 1.0]),
            None,
            &panel,
        );
        for (index, row) in overlay.rows.iter().enumerate() {
            let next_is_divider = overlay
                .rows
                .as_slice()
                .get(index + 1)
                .is_some_and(|next| matches!(next.entry, TextContextMenuEntry::Divider));
            if index + 1 < overlay.rows.len()
                && !matches!(row.entry, TextContextMenuEntry::Divider)
                && !next_is_divider
            {
                let separator = vello::kurbo::Rect::new(
                    row.bounds.x0 + 6.0,
                    row.bounds.y1 - 1.0,
                    row.bounds.x1 - 6.0,
                    row.bounds.y1,
                );
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    transform,
                    vello::peniko::Color::new([0.9, 0.91, 0.94, 1.0]),
                    None,
                    &separator,
                );
            }

            match &row.entry {
                TextContextMenuEntry::Command { label, .. } => {
                    let text_rect = inset_rect(
                        row.bounds,
                        f64::from(TEXT_CONTEXT_MENU_HORIZONTAL_PADDING),
                        TEXT_CONTEXT_MENU_VERTICAL_PADDING,
                    );
                    let ctx = RenderContext {
                        renderer_ptr,
                        transform,
                        hit_transform: vello::kurbo::Affine::IDENTITY,
                        bounds: overlay.bounds,
                    }
                    .child(
                        vello::kurbo::Affine::translate((text_rect.x0, text_rect.y0)),
                        vello::kurbo::Rect::new(0.0, 0.0, text_rect.width(), text_rect.height()),
                    );
                    Self::render_styled_text(
                        self.dispatcher.state_mut(),
                        ctx,
                        StyledStr::plain(label.clone()),
                        HorizontalAlignment::Leading,
                        env,
                    );
                }
                TextContextMenuEntry::Divider => {
                    let separator = vello::kurbo::Rect::new(
                        row.bounds.x0 + 6.0,
                        row.bounds.y0 + row.bounds.height() * 0.5 - 0.5,
                        row.bounds.x1 - 6.0,
                        row.bounds.y0 + row.bounds.height() * 0.5 + 0.5,
                    );
                    scene.fill(
                        vello::peniko::Fill::NonZero,
                        transform,
                        vello::peniko::Color::new([0.86, 0.87, 0.9, 1.0]),
                        None,
                        &separator,
                    );
                }
            }
        }
    }

    fn handle_text_context_menu_overlay_pointer_down(
        &mut self,
        point: vello::kurbo::Point,
    ) -> bool {
        let Some(ActiveTextContextMenu::Overlay { overlay, .. }) =
            self.active_text_context_menu.clone()
        else {
            return false;
        };
        if !overlay.bounds.contains(point) {
            self.dismiss_active_text_context_menu();
            return false;
        }
        for row in &overlay.rows {
            if !row.bounds.contains(point) {
                continue;
            }
            match &row.entry {
                TextContextMenuEntry::Command { action, .. } => {
                    let changed = execute_text_context_menu_action(
                        action,
                        &overlay.model,
                        &overlay.selection,
                        &overlay.env,
                    );
                    self.dismiss_active_text_context_menu();
                    return changed;
                }
                TextContextMenuEntry::Divider => return false,
            }
        }
        true
    }

    fn focused_text_target_data(
        &mut self,
    ) -> Option<(usize, TextInputModel, Rc<RefCell<TextSelectionSlot>>)> {
        let index = self.focused_text_input.get()?;
        let Some(target) = self.text_input_targets.as_slice().get(index) else {
            self.set_focused_text_input(None);
            return None;
        };
        Some((index, target.model.clone(), Rc::clone(&target.selection)))
    }

    fn text_selection_index_from_point(
        target: &TextInputTarget,
        point: vello::kurbo::Point,
    ) -> usize {
        let local_x = (point.x - target.text_bounds.x0) as f32;
        let local_y = (point.y - target.text_bounds.y0) as f32;
        let selection =
            parley::Selection::from_point(&target.layout, local_x, local_y).refresh(&target.layout);
        target
            .model
            .plain_index_from_layout_index(selection.focus().index())
    }

    fn text_selection_range_from_point_with_click_count(
        target: &TextInputTarget,
        point: vello::kurbo::Point,
        click_count: u8,
    ) -> (usize, usize) {
        let local_x = (point.x - target.text_bounds.x0) as f32;
        let local_y = (point.y - target.text_bounds.y0) as f32;
        let selection = match click_count {
            2 => parley::Selection::word_from_point(&target.layout, local_x, local_y),
            3.. => parley::Selection::line_from_point(&target.layout, local_x, local_y),
            _ => parley::Selection::from_point(&target.layout, local_x, local_y),
        }
        .refresh(&target.layout);
        (
            target
                .model
                .plain_index_from_layout_index(selection.anchor().index()),
            target
                .model
                .plain_index_from_layout_index(selection.focus().index()),
        )
    }

    fn next_text_selection_click_count(
        &mut self,
        target_index: usize,
        point: vello::kurbo::Point,
        at: Instant,
    ) -> u8 {
        let count = if let Some(previous) = self.last_text_selection_click {
            if previous.target_index == target_index
                && at.saturating_duration_since(previous.at) <= TEXT_SELECTION_MULTI_CLICK_INTERVAL
                && previous.point.distance(point) <= TEXT_SELECTION_MULTI_CLICK_DISTANCE
            {
                previous.count.saturating_add(1).min(3)
            } else {
                1
            }
        } else {
            1
        };
        self.last_text_selection_click = Some(TextSelectionClickState {
            target_index,
            point,
            at,
            count,
        });
        count
    }

    fn apply_text_selection_click_gesture(
        &mut self,
        index: usize,
        point: vello::kurbo::Point,
        click_count: u8,
    ) -> bool {
        let Some(target) = self.text_input_targets.as_slice().get(index) else {
            self.active_text_selection_drag = None;
            return false;
        };
        let (anchor, focus) =
            Self::text_selection_range_from_point_with_click_count(target, point, click_count);
        let mut slot = target.selection.borrow_mut();
        let changed = slot.anchor != anchor || slot.focus != focus || !slot.initialized;
        slot.anchor = anchor;
        slot.focus = focus;
        slot.initialized = true;
        changed
    }

    fn update_text_selection_from_pointer(
        &mut self,
        index: usize,
        point: vello::kurbo::Point,
        extend: bool,
    ) -> bool {
        let Some(target) = self.text_input_targets.as_slice().get(index) else {
            self.active_text_selection_drag = None;
            return false;
        };
        let next_index = Self::text_selection_index_from_point(target, point);
        let mut slot = target.selection.borrow_mut();
        if !extend || !slot.initialized {
            let changed =
                slot.anchor != next_index || slot.focus != next_index || !slot.initialized;
            slot.anchor = next_index;
            slot.focus = next_index;
            slot.initialized = true;
            return changed;
        }
        let changed = slot.focus != next_index || !slot.initialized;
        slot.focus = next_index;
        slot.initialized = true;
        changed
    }

    fn insert_text_into_focused_target(&mut self, text: &str) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        let mut slot = selection.borrow_mut();
        replace_model_selection(&model, &mut slot, text)
    }

    fn delete_backward_in_focused_target(&mut self) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        let mut slot = selection.borrow_mut();
        delete_model_backward(&model, &mut slot)
    }

    fn delete_forward_in_focused_target(&mut self) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        let mut slot = selection.borrow_mut();
        delete_model_forward(&model, &mut slot)
    }

    fn select_all_in_focused_target(&mut self) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        let mut slot = selection.borrow_mut();
        select_all_model_text(&model, &mut slot)
    }

    fn copy_selection_in_focused_target(&mut self) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        if model.is_secure() {
            return false;
        }
        let slot = selection.borrow();
        let Some(selected) = selected_text_for_model(&model, &slot) else {
            return false;
        };
        write_clipboard_text(selected.as_str())
    }

    fn cut_selection_in_focused_target(&mut self) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        if model.is_secure() {
            return false;
        }
        let mut slot = selection.borrow_mut();
        let Some(selected) = selected_text_for_model(&model, &slot) else {
            return false;
        };
        if !write_clipboard_text(selected.as_str()) {
            return false;
        }
        delete_model_selection(&model, &mut slot)
    }

    fn paste_clipboard_into_focused_target(&mut self) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        spawn_clipboard_paste_task(model, selection);
        true
    }

    fn move_focused_caret_horizontal(&mut self, backward: bool, extend: bool) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        let mut slot = selection.borrow_mut();
        move_model_caret_horizontal(&model, &mut slot, backward, extend)
    }

    fn move_focused_caret_to_boundary(&mut self, end: bool, extend: bool) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        let mut slot = selection.borrow_mut();
        let text = model.plain_text();
        let next_index = if end { text.len() } else { 0 };
        if extend {
            let next_index = clamp_to_char_boundary(text.as_str(), next_index);
            let changed = slot.focus != next_index || !slot.initialized;
            slot.focus = next_index;
            slot.initialized = true;
            return changed;
        }
        set_model_caret_position(&model, &mut slot, next_index)
    }

    fn build_text_context_menu_entries(target: &TextInputTarget) -> Vec<TextContextMenuEntry> {
        let has_selection = {
            let slot = target.selection.borrow();
            selected_text_for_model(&target.model, &slot).is_some()
        };
        let has_text = !target.model.plain_text().is_empty();
        let mut entries = Vec::new();
        if has_selection && !target.model.is_secure() {
            entries.push(TextContextMenuEntry::Command {
                label: "Copy".to_owned(),
                action: TextContextMenuAction::Copy,
            });
            entries.push(TextContextMenuEntry::Command {
                label: "Cut".to_owned(),
                action: TextContextMenuAction::Cut,
            });
        }
        entries.push(TextContextMenuEntry::Command {
            label: "Paste".to_owned(),
            action: TextContextMenuAction::Paste,
        });
        if has_text {
            entries.push(TextContextMenuEntry::Command {
                label: "Select All".to_owned(),
                action: TextContextMenuAction::SelectAll,
            });
        }
        if has_selection {
            for item in target.model.custom_selection_menu_items() {
                match item {
                    ResolvedMenuItem::Command(command) => {
                        entries.push(TextContextMenuEntry::Command {
                            label: command.label.content.get().to_plain().to_string(),
                            action: TextContextMenuAction::Custom(command),
                        });
                    }
                    ResolvedMenuItem::Divider => entries.push(TextContextMenuEntry::Divider),
                    ResolvedMenuItem::Menu(_) => {
                        panic!("hydrolysis text selection menus do not support nested menus yet")
                    }
                }
            }
        }
        entries
    }

    fn show_text_context_menu(
        &mut self,
        index: usize,
        point: vello::kurbo::Point,
        env: &Environment,
    ) -> bool {
        let Some(target) = self.text_input_targets.as_slice().get(index).cloned() else {
            return false;
        };
        let entries = Self::build_text_context_menu_entries(&target);
        if entries.is_empty() {
            self.dismiss_active_text_context_menu();
            return false;
        }

        self.dismiss_active_text_context_menu();
        let mode = env
            .get::<HydrolysisTextContextMenuMode>()
            .copied()
            .unwrap_or(HydrolysisTextContextMenuMode::NativeWindow);

        if mode == HydrolysisTextContextMenuMode::Overlay {
            let bounds = text_context_menu_overlay_bounds(point, &entries, self.window_bounds);
            let mut rows = Vec::with_capacity(entries.len());
            for (index, entry) in entries.into_iter().enumerate() {
                let y0 = bounds.y0 + f64::from(TEXT_CONTEXT_MENU_ROW_HEIGHT) * index as f64;
                let row_bounds = vello::kurbo::Rect::new(
                    bounds.x0,
                    y0,
                    bounds.x1,
                    y0 + f64::from(TEXT_CONTEXT_MENU_ROW_HEIGHT),
                );
                rows.push(TextContextMenuOverlayRow {
                    bounds: row_bounds,
                    entry,
                });
            }
            self.active_text_context_menu = Some(ActiveTextContextMenu::Overlay {
                index,
                overlay: TextContextMenuOverlay {
                    bounds,
                    rows,
                    model: target.model,
                    selection: target.selection,
                    env: env.clone(),
                },
            });
            return true;
        }

        let menu_state = nami::Binding::container(WindowState::Normal);
        let (width, height) = text_context_menu_size(&entries);
        let origin = env
            .get::<HydrolysisWindowOrigin>()
            .copied()
            .expect("hydrolysis text context menu requires HydrolysisWindowOrigin in environment");

        let entries_for_popup = entries.clone();
        let model = target.model.clone();
        let selection = Rc::clone(&target.selection);
        let action_env = env.clone();
        let menu_state_for_content = menu_state.clone();
        let popup_content = move || {
            let mut rows = Vec::with_capacity(entries_for_popup.len());
            for entry in entries_for_popup.clone() {
                let state_binding = menu_state_for_content.clone();
                let model = model.clone();
                let selection = Rc::clone(&selection);
                let action_env = action_env.clone();
                match entry {
                    TextContextMenuEntry::Command { label, action } => {
                        let button =
                            Button::new(label)
                                .style(ButtonStyle::Borderless)
                                .action(move || {
                                    state_binding.set(WindowState::Closed);
                                    let _ = execute_text_context_menu_action(
                                        &action,
                                        &model,
                                        &selection,
                                        &action_env,
                                    );
                                });
                        rows.push(AnyView::new(button));
                    }
                    TextContextMenuEntry::Divider => rows.push(AnyView::new(Divider)),
                }
            }
            let menu_content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
                rows.into_iter().collect();
            AnyView::new(
                menu_content
                    .alignment(HorizontalAlignment::Leading)
                    .spacing(0.0),
            )
        };
        let mut popup = Window::new(TEXT_CONTEXT_MENU_WINDOW_TITLE, popup_content)
            .style(WindowStyle::Borderless)
            .resizable(false)
            .with_state(menu_state.clone());
        popup.closable = false;
        popup.frame.set(LayoutRect::new(
            LayoutPoint::new(origin.x + point.x as f32, origin.y + point.y as f32),
            LayoutSize::new(width as f32, height as f32),
        ));
        popup.show(env);
        self.active_text_context_menu = Some(ActiveTextContextMenu::NativeWindow {
            index,
            state: menu_state,
        });
        true
    }

    #[cfg(feature = "accessibility")]
    fn focused_text_input_accessibility_node(&self) -> Option<AccessibilityNodeId> {
        let index = self.focused_text_input.get()?;
        let target = self.text_input_targets.as_slice().get(index)?;
        target.accessibility_node_id
    }

    #[cfg(feature = "accessibility")]
    fn focus_text_input_for_accessibility_node(&mut self, node_id: AccessibilityNodeId) -> bool {
        let focused = self
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
        let order = self.hit_test_order;
        self.hit_test_order = self
            .hit_test_order
            .checked_add(1)
            .expect("hydrolysis hit-test order overflow");
        order
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
        self.push_render_depth();
        self.dispatcher.dispatch(view, env, ctx);
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

    fn dispatch_any(ctx: RenderContext, env: &Environment, content: AnyView) {
        let renderer = unsafe { ctx.renderer() };
        renderer.dispatch_with_render_depth(content, env, ctx);
    }

    fn dispatch_any_without_accessibility(ctx: RenderContext, env: &Environment, content: AnyView) {
        #[cfg(feature = "accessibility")]
        {
            let renderer = unsafe { ctx.renderer() };
            renderer.push_accessibility_suppression();
            renderer.dispatch_with_render_depth(content, env, ctx);
            renderer.pop_accessibility_suppression();
            return;
        }
        #[cfg(not(feature = "accessibility"))]
        Self::dispatch_any(ctx, env, content);
    }

    fn dispatch_in_rect(
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
        Self::dispatch_any(ctx.child(child_transform, child_bounds), env, content);
    }

    fn dispatch_in_rect_without_accessibility(
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
            ctx.child(child_transform, child_bounds),
            env,
            content,
        );
    }

    fn render_subtree_scene(
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
    ) -> vello::Scene {
        let renderer = unsafe { ctx.renderer() };
        let mut subtree_scene = vello::Scene::new();
        core::mem::swap(&mut renderer.scene, &mut subtree_scene);
        renderer.dispatch_with_render_depth(content, env, ctx);
        core::mem::swap(&mut renderer.scene, &mut subtree_scene);
        subtree_scene
    }

    fn render_dynamic_subtree(
        ctx: RenderContext,
        env: &Environment,
        content: AnyView,
    ) -> DynamicSubtree {
        let renderer = unsafe { ctx.renderer() };
        let subtree_depth_base = renderer.render_depth;
        let mut subtree_scene = vello::Scene::new();
        let mut subtree_pointer_targets = Vec::new();
        let mut subtree_gesture_targets = Vec::new();
        let mut subtree_cursor_targets = Vec::new();
        let mut subtree_hover_targets = Vec::new();
        let mut subtree_text_input_targets = Vec::new();
        let mut subtree_scroll_targets = Vec::new();
        let mut subtree_hover_controller = HoverController::default();
        #[cfg(feature = "accessibility")]
        let mut subtree_accessibility_nodes = Vec::new();
        #[cfg(feature = "accessibility")]
        let mut subtree_accessibility_root_children = Vec::new();
        #[cfg(feature = "accessibility")]
        let mut subtree_accessibility_actions = BTreeMap::new();

        core::mem::swap(&mut renderer.scene, &mut subtree_scene);
        core::mem::swap(&mut renderer.pointer_targets, &mut subtree_pointer_targets);
        renderer
            .gesture_engine
            .swap_targets(&mut subtree_gesture_targets);
        core::mem::swap(&mut renderer.cursor_targets, &mut subtree_cursor_targets);
        core::mem::swap(&mut renderer.hover_targets, &mut subtree_hover_targets);
        core::mem::swap(
            &mut renderer.hover_controller,
            &mut subtree_hover_controller,
        );
        core::mem::swap(
            &mut renderer.text_input_targets,
            &mut subtree_text_input_targets,
        );
        core::mem::swap(&mut renderer.scroll_targets, &mut subtree_scroll_targets);
        #[cfg(feature = "accessibility")]
        {
            core::mem::swap(
                &mut renderer.accessibility_nodes,
                &mut subtree_accessibility_nodes,
            );
            core::mem::swap(
                &mut renderer.accessibility_root_children,
                &mut subtree_accessibility_root_children,
            );
            core::mem::swap(
                &mut renderer.accessibility_actions,
                &mut subtree_accessibility_actions,
            );

            let previous_accessibility_next_node_id = renderer.accessibility_next_node_id;
            let previous_accessibility_suppression_depth = renderer.accessibility_suppression_depth;
            renderer.accessibility_next_node_id = ACCESSIBILITY_FIRST_NODE_ID;
            renderer.accessibility_suppression_depth = 0;
            renderer.dispatch_with_render_depth(content, env, ctx);
            renderer.accessibility_suppression_depth = previous_accessibility_suppression_depth;
            renderer.accessibility_next_node_id = previous_accessibility_next_node_id;

            core::mem::swap(
                &mut renderer.accessibility_actions,
                &mut subtree_accessibility_actions,
            );
            core::mem::swap(
                &mut renderer.accessibility_root_children,
                &mut subtree_accessibility_root_children,
            );
            core::mem::swap(
                &mut renderer.accessibility_nodes,
                &mut subtree_accessibility_nodes,
            );
        }
        #[cfg(not(feature = "accessibility"))]
        renderer.dispatch_with_render_depth(content, env, ctx);

        core::mem::swap(&mut renderer.scroll_targets, &mut subtree_scroll_targets);
        core::mem::swap(
            &mut renderer.text_input_targets,
            &mut subtree_text_input_targets,
        );
        core::mem::swap(
            &mut renderer.hover_controller,
            &mut subtree_hover_controller,
        );
        core::mem::swap(&mut renderer.hover_targets, &mut subtree_hover_targets);
        core::mem::swap(&mut renderer.cursor_targets, &mut subtree_cursor_targets);
        renderer
            .gesture_engine
            .swap_targets(&mut subtree_gesture_targets);
        core::mem::swap(&mut renderer.pointer_targets, &mut subtree_pointer_targets);
        core::mem::swap(&mut renderer.scene, &mut subtree_scene);

        DynamicSubtree {
            scene: subtree_scene,
            depth_base: subtree_depth_base,
            pointer_targets: subtree_pointer_targets,
            gesture_targets: subtree_gesture_targets,
            cursor_targets: subtree_cursor_targets,
            hover_targets: subtree_hover_targets,
            text_input_targets: subtree_text_input_targets,
            scroll_targets: subtree_scroll_targets,
            #[cfg(feature = "accessibility")]
            accessibility: DynamicAccessibilitySubtree {
                nodes: subtree_accessibility_nodes,
                root_children: subtree_accessibility_root_children,
                actions: subtree_accessibility_actions,
            },
        }
    }

    fn replay_dynamic_subtree(&mut self, ctx: RenderContext, subtree: &DynamicSubtree) {
        self.scene.append(&subtree.scene, Some(ctx.transform));
        let mut gesture_group_remap = BTreeMap::new();

        for target in &subtree.pointer_targets {
            let depth = self.replay_target_depth(subtree.depth_base, target.depth);
            self.register_pointer_target_action(
                transformed_rect(ctx.hit_transform, target.bounds),
                target.captures_drag,
                Rc::clone(&target.action),
                depth,
            );
        }
        for target in &subtree.gesture_targets {
            let depth = self.replay_target_depth(subtree.depth_base, target.depth);
            let group_id = *gesture_group_remap
                .entry(target.group_id)
                .or_insert_with(|| self.allocate_gesture_group_id());
            self.register_gesture_target_recognizer(
                transformed_rect(ctx.hit_transform, target.bounds),
                target.clone(),
                depth,
                group_id,
            );
        }
        for target in &subtree.cursor_targets {
            self.register_cursor_target_style(
                transformed_rect(ctx.hit_transform, target.bounds),
                target.style,
            );
        }
        for target in &subtree.hover_targets {
            let bounds = transformed_rect(ctx.hit_transform, target.bounds);
            self.register_hover_target(
                bounds,
                target.on_enter.clone(),
                target.on_move.clone(),
                target.on_exit.clone(),
            );
        }
        #[cfg(feature = "accessibility")]
        let accessibility_node_map =
            self.replay_dynamic_accessibility_subtree(ctx.hit_transform, &subtree.accessibility);
        for target in &subtree.text_input_targets {
            let depth = self.replay_target_depth(subtree.depth_base, target.depth);
            #[cfg(feature = "accessibility")]
            let accessibility_node_id = target.accessibility_node_id.map(|node_id| {
                *accessibility_node_map.get(&node_id).unwrap_or_else(|| {
                    panic!(
                        "hydrolysis dynamic text input accessibility node {:?} is missing remap entry",
                        node_id
                    )
                })
            });
            self.register_text_input_target_data(
                transformed_rect(ctx.hit_transform, target.bounds),
                transformed_rect(ctx.hit_transform, target.cursor_area),
                transformed_rect(ctx.hit_transform, target.text_bounds),
                target.layout.clone(),
                target.purpose,
                target.model.clone(),
                Rc::clone(&target.selection),
                depth,
                target.focus_binding.clone(),
                #[cfg(feature = "accessibility")]
                accessibility_node_id,
            );
        }
        for target in &subtree.scroll_targets {
            self.register_scroll_target_action(
                transformed_rect(ctx.hit_transform, target.bounds),
                Rc::clone(&target.action),
            );
        }
    }

    #[cfg(feature = "accessibility")]
    fn next_accessibility_node_id(&mut self) -> AccessibilityNodeId {
        let node_id = AccessibilityNodeId(self.accessibility_next_node_id);
        self.accessibility_next_node_id = self
            .accessibility_next_node_id
            .checked_add(1)
            .expect("hydrolysis accessibility node ID overflow");
        node_id
    }

    #[cfg(feature = "accessibility")]
    fn replay_dynamic_accessibility_subtree(
        &mut self,
        transform: vello::kurbo::Affine,
        subtree: &DynamicAccessibilitySubtree,
    ) -> BTreeMap<AccessibilityNodeId, AccessibilityNodeId> {
        if self.accessibility_suppression_depth > 0 {
            return BTreeMap::new();
        }

        let mut id_map = BTreeMap::new();
        for (local_id, _) in &subtree.nodes {
            id_map.insert(*local_id, self.next_accessibility_node_id());
        }

        for (local_id, node) in &subtree.nodes {
            let mapped_id = *id_map
                .get(local_id)
                .expect("hydrolysis dynamic accessibility node mapping missing local id");
            let mut mapped_node = node.clone();
            remap_accessibility_node_references(&mut mapped_node, &id_map);
            if let Some(bounds) = mapped_node.bounds() {
                let bounds = transformed_rect(transform, accesskit_rect_to_kurbo_rect(bounds));
                mapped_node.set_bounds(kurbo_rect_to_accesskit_rect(bounds));
            }
            if subtree
                .root_children
                .iter()
                .any(|root_id| root_id == local_id)
            {
                self.accessibility_root_children.push(mapped_id);
            }
            self.accessibility_nodes.push((mapped_id, mapped_node));
            if let Some(action_target) = subtree.actions.get(local_id) {
                self.accessibility_actions.insert(
                    mapped_id,
                    transform_accessibility_action_target(action_target, transform),
                );
            }
        }
        id_map
    }

    fn watch_signal<S>(&mut self, signal: &S)
    where
        S: Signal + Clone + 'static,
    {
        let rebuild_requested = Rc::clone(&self.rebuild_requested);
        let guard = signal.watch(move |_| rebuild_requested.set(true));
        self.current_frame_retain.push(Retain::new(guard));
    }

    fn read_signal<S>(&mut self, signal: &S) -> S::Output
    where
        S: Signal + Clone + 'static,
    {
        self.watch_signal(signal);
        signal.get()
    }

    fn bind_navigation_entries(&mut self) -> (usize, NavigationEntries) {
        let index = self.navigation_cursor;
        self.navigation_cursor = self
            .navigation_cursor
            .checked_add(1)
            .expect("navigation slot cursor overflow");

        if index == self.navigation_slots.len() {
            self.navigation_slots.push(NavigationSlot::new());
        }

        (index, Rc::clone(&self.navigation_slots[index].entries))
    }

    fn push_pending_navigation_entries(&mut self, slot_index: usize, entries: NavigationEntries) {
        self.pending_navigation_entries.push((slot_index, entries));
    }

    fn take_pending_navigation_entries(
        &mut self,
        caller: &'static str,
    ) -> (usize, NavigationEntries) {
        self.pending_navigation_entries
            .pop()
            .unwrap_or_else(|| panic!("hydrolysis {caller} requires prebound navigation entries"))
    }

    fn push_pending_scroll_handle(&mut self, handle: ScrollHandle) {
        self.pending_scroll_handles.push(handle);
    }

    fn take_pending_scroll_handle(&mut self, caller: &'static str) -> ScrollHandle {
        self.pending_scroll_handles
            .pop()
            .unwrap_or_else(|| panic!("hydrolysis {caller} requires prebound scroll handle"))
    }

    fn bind_picker_menu_state(&mut self) -> Rc<Cell<bool>> {
        let index = self.picker_menu_cursor;
        self.picker_menu_cursor = self
            .picker_menu_cursor
            .checked_add(1)
            .expect("picker menu slot cursor overflow");
        if index == self.picker_menu_slots.len() {
            self.picker_menu_slots.push(PickerMenuSlot::new());
        }
        Rc::clone(&self.picker_menu_slots[index].open)
    }

    fn bind_text_selection_slot(&mut self) -> Rc<RefCell<TextSelectionSlot>> {
        let index = self.text_selection_cursor;
        self.text_selection_cursor = self
            .text_selection_cursor
            .checked_add(1)
            .expect("text selection slot cursor overflow");
        if index == self.text_selection_slots.len() {
            self.text_selection_slots
                .push(Rc::new(RefCell::new(TextSelectionSlot::default())));
        }
        Rc::clone(&self.text_selection_slots[index])
    }

    fn resolve_animated_scalar<S>(&mut self, signal: &S) -> f32
    where
        S: Signal<Output = f32> + Clone + 'static,
    {
        let now = Instant::now();
        let handle = self.animation_controller.bind_scalar(signal.get());
        let watcher_handle = handle.clone();
        let rebuild_requested = Rc::clone(&self.rebuild_requested);
        let guard = signal.watch(move |update| {
            watcher_handle.apply_update_from_context(update, Instant::now());
            rebuild_requested.set(true);
        });
        self.current_frame_retain.push(Retain::new(guard));
        handle.sample(now)
    }

    fn resolve_toggle_progress<S>(&mut self, signal: &S) -> f32
    where
        S: Signal<Output = bool> + Clone + 'static,
    {
        let now = Instant::now();
        let handle = self
            .animation_controller
            .bind_scalar(if signal.get() { 1.0 } else { 0.0 });
        let watcher_handle = handle.clone();
        let rebuild_requested = Rc::clone(&self.rebuild_requested);
        let default_animation = Animation::spring(CONTROL_SPRING_STIFFNESS, CONTROL_SPRING_DAMPING);
        let guard = signal.watch(move |update| {
            let target = if *update.value() { 1.0 } else { 0.0 };
            let animation = update
                .metadata()
                .try_get::<Animation>()
                .unwrap_or_else(|| default_animation.clone());
            watcher_handle.apply_target(target, Some(animation), Instant::now());
            rebuild_requested.set(true);
        });
        self.current_frame_retain.push(Retain::new(guard));
        handle.sample(now).clamp(0.0, 1.0)
    }

    fn render_layout_container(
        state: &mut HydroState,
        ctx: RenderContext,
        layout: Box<dyn Layout>,
        children: Vec<AnyView>,
        env: &Environment,
    ) {
        let mut resolved_children = Vec::with_capacity(children.len());
        for child in children {
            resolved_children.push(normalize_layout_view(child, env));
        }

        let mut subviews = Vec::with_capacity(resolved_children.len());
        for child in &resolved_children {
            subviews.push(HydroSubview::from_view(child, state, env));
        }
        let refs: Vec<&dyn SubView> = subviews.iter().map(|view| view as &dyn SubView).collect();

        let proposal = ProposalSize::new(
            Some(ctx.bounds.width() as f32),
            Some(ctx.bounds.height() as f32),
        );
        let _ = layout.size_that_fits(proposal, &refs);
        let bounds = LayoutRect::from_size(LayoutSize::new(
            ctx.bounds.width() as f32,
            ctx.bounds.height() as f32,
        ));
        let child_rects = layout.place(bounds, &refs);

        for (child, rect) in resolved_children.into_iter().zip(child_rects) {
            let child_transform =
                vello::kurbo::Affine::translate((f64::from(rect.x()), f64::from(rect.y())));
            let child_bounds = vello::kurbo::Rect::new(
                0.0,
                0.0,
                f64::from(rect.width()),
                f64::from(rect.height()),
            );
            Self::dispatch_any(ctx.child(child_transform, child_bounds), env, child);
        }
    }

    fn render_fixed_container(
        state: &mut HydroState,
        ctx: RenderContext,
        container: Native<FixedContainer>,
        env: &Environment,
    ) {
        let (layout, children) = container.into_inner().into_inner();
        Self::render_layout_container(state, ctx, layout, children, env);
    }

    fn render_lazy_container(
        state: &mut HydroState,
        ctx: RenderContext,
        container: Native<LazyContainer>,
        env: &Environment,
    ) {
        let (layout, children) = container.into_inner().into_inner();
        let axis_config = lazy_stack_axis_config(layout.as_ref());
        let count = children.len().get();
        if count == 0 {
            return;
        }
        let visible_bounds = {
            let renderer = unsafe { ctx.renderer() };
            renderer
                .lazy_viewport_stack
                .last()
                .copied()
                .unwrap_or(ctx.bounds)
        };
        let slot_index = {
            let renderer = unsafe { ctx.renderer() };
            let index = renderer.lazy_stack_controller.bind();
            renderer.lazy_stack_controller.slots[index].prepare_len(count);
            index
        };
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
                let renderer = unsafe { ctx.renderer() };
                renderer.lazy_stack_controller.slots[slot_index].item_extents[index]
            };
            let extent = if let Some(extent) = cached_extent {
                extent
            } else {
                let child = children.get_view(index).unwrap_or_else(|| {
                    panic!("LazyContainer failed to materialize child at index {index}")
                });
                let child = normalize_layout_view(child, env);
                let subview = HydroSubview::from_view(&child, state, env);
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
                let renderer = unsafe { ctx.renderer() };
                renderer.lazy_stack_controller.slots[slot_index].item_extents[index] = Some(extent);
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
            let child = normalize_layout_view(child, env);
            let subview = HydroSubview::from_view(&child, state, env);
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
                let renderer = unsafe { ctx.renderer() };
                renderer.lazy_stack_controller.slots[slot_index].item_extents[index] = Some(extent);
            }
            Self::dispatch_any(
                ctx.child(
                    vello::kurbo::Affine::translate((child_rect.x0, child_rect.y0)),
                    vello::kurbo::Rect::new(0.0, 0.0, child_rect.width(), child_rect.height()),
                ),
                env,
                child,
            );
            cursor += extent;
            if index + 1 < count {
                cursor += spacing;
            }
        }
    }

    fn render_scroll_view(
        state: &mut HydroState,
        ctx: RenderContext,
        scroll: Native<ScrollView>,
        env: &Environment,
    ) {
        let (axis, content) = scroll.into_inner().into_inner();
        let content = normalize_layout_view(content, env);
        let viewport = ctx.bounds;
        let intrinsic = measure_view_intrinsic(&content, state, env);
        let (content_width, content_height) = match axis {
            ScrollAxis::Horizontal => (
                f64::from(intrinsic.width).max(viewport.width()),
                viewport.height(),
            ),
            ScrollAxis::Vertical => (
                viewport.width(),
                f64::from(intrinsic.height).max(viewport.height()),
            ),
            ScrollAxis::All => (
                f64::from(intrinsic.width).max(viewport.width()),
                f64::from(intrinsic.height).max(viewport.height()),
            ),
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        };

        let handle = {
            let renderer = unsafe { ctx.renderer() };
            renderer.take_pending_scroll_handle("render_scroll_view")
        };
        let metrics = handle.metrics();

        let content_transform =
            vello::kurbo::Affine::translate((-metrics.offset_x, -metrics.offset_y));
        let content_bounds = vello::kurbo::Rect::new(0.0, 0.0, content_width, content_height);
        let lazy_viewport = vello::kurbo::Rect::new(
            metrics.offset_x,
            metrics.offset_y,
            metrics.offset_x + viewport.width(),
            metrics.offset_y + viewport.height(),
        );
        {
            let renderer = unsafe { ctx.renderer() };
            renderer.push_layer_rect(1.0, ctx.transform, viewport);
        }
        {
            let renderer = unsafe { ctx.renderer() };
            renderer.lazy_viewport_stack.push(lazy_viewport);
        }
        Self::dispatch_any(ctx.child(content_transform, content_bounds), env, content);
        {
            let renderer = unsafe { ctx.renderer() };
            renderer
                .lazy_viewport_stack
                .pop()
                .expect("lazy viewport stack underflow in render_scroll_view");
        }
        {
            let renderer = unsafe { ctx.renderer() };
            renderer.pop_layer();
        }

        {
            let renderer = unsafe { ctx.renderer() };
            let target_handle = handle.clone();
            renderer.register_scroll_target(
                transformed_rect(ctx.hit_transform, viewport),
                move |dx, dy, is_line_delta| {
                    target_handle.apply_scroll_delta(dx, dy, is_line_delta)
                },
            );
        }

        let scene = unsafe { ctx.scene() };
        Self::draw_scroll_indicators(scene, ctx.transform, viewport, metrics, axis);
    }

    fn render_navigation_view(
        _state: &mut HydroState,
        ctx: RenderContext,
        navigation: Native<NavigationView>,
        env: &Environment,
    ) {
        let navigation = navigation.into_inner();
        let NavigationView { bar, content } = navigation;
        let waterui::navigation::Bar {
            title,
            leading,
            trailing,
            search,
            color,
            hidden,
            display_mode,
        } = bar;
        let bar_height = if {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&hidden)
        } {
            0.0
        } else {
            let base = match display_mode {
                waterui::navigation::NavigationTitleDisplayMode::Automatic => {
                    NAVIGATION_BAR_HEIGHT_AUTOMATIC
                }
                waterui::navigation::NavigationTitleDisplayMode::Inline => {
                    NAVIGATION_BAR_HEIGHT_INLINE
                }
                waterui::navigation::NavigationTitleDisplayMode::Large => {
                    NAVIGATION_BAR_HEIGHT_LARGE
                }
            };
            let search_extra = if search.is_some() {
                NAVIGATION_SEARCH_HEIGHT + NAVIGATION_SEARCH_VERTICAL_INSET * 2.0
            } else {
                0.0
            };
            base + search_extra
        };

        if bar_height > 0.0 {
            let base_bar_height = navigation_base_bar_height_for_display_mode(display_mode);
            let bar_rect = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                (ctx.bounds.y0 + bar_height).min(ctx.bounds.y1),
            );
            let bar_color = {
                let renderer = unsafe { ctx.renderer() };
                let color = renderer.read_signal(&color);
                resolved_color_to_peniko(color.resolve(env).get())
            };
            {
                let scene = unsafe { ctx.scene() };
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    bar_color,
                    None,
                    &bar_rect,
                );
                let separator = vello::kurbo::Rect::new(
                    bar_rect.x0,
                    (bar_rect.y1 - 1.0).max(bar_rect.y0),
                    bar_rect.x1,
                    bar_rect.y1,
                );
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    vello::peniko::Color::new([0.8, 0.8, 0.82, 1.0]),
                    None,
                    &separator,
                );
            }

            let leading_width = if !leading.is::<()>() {
                f64::from(measure_view_intrinsic(&leading, _state, env).width)
            } else {
                0.0
            };
            let trailing_width = if !trailing.is::<()>() {
                f64::from(measure_view_intrinsic(&trailing, _state, env).width)
            } else {
                0.0
            };
            let leading_rect = vello::kurbo::Rect::new(
                bar_rect.x0 + NAVIGATION_BAR_HORIZONTAL_INSET,
                bar_rect.y0 + (base_bar_height - NAVIGATION_TITLE_HEIGHT_INLINE) * 0.5,
                (bar_rect.x0 + NAVIGATION_BAR_HORIZONTAL_INSET + leading_width).min(bar_rect.x1),
                bar_rect.y0 + (base_bar_height + NAVIGATION_TITLE_HEIGHT_INLINE) * 0.5,
            );
            let trailing_rect = vello::kurbo::Rect::new(
                (bar_rect.x1 - NAVIGATION_BAR_HORIZONTAL_INSET - trailing_width).max(bar_rect.x0),
                bar_rect.y0 + (base_bar_height - NAVIGATION_TITLE_HEIGHT_INLINE) * 0.5,
                bar_rect.x1 - NAVIGATION_BAR_HORIZONTAL_INSET,
                bar_rect.y0 + (base_bar_height + NAVIGATION_TITLE_HEIGHT_INLINE) * 0.5,
            );
            if !leading.is::<()>() && leading_rect.width() > 0.0 && leading_rect.height() > 0.0 {
                Self::dispatch_in_rect(ctx, env, leading, leading_rect);
            }
            if !trailing.is::<()>() && trailing_rect.width() > 0.0 && trailing_rect.height() > 0.0 {
                Self::dispatch_in_rect(ctx, env, trailing, trailing_rect);
            }

            let title_height = if matches!(
                display_mode,
                waterui::navigation::NavigationTitleDisplayMode::Large
            ) {
                NAVIGATION_TITLE_HEIGHT_LARGE
            } else {
                NAVIGATION_TITLE_HEIGHT_INLINE
            };
            let title_rect = vello::kurbo::Rect::new(
                (bar_rect.x0
                    + NAVIGATION_BAR_HORIZONTAL_INSET
                    + leading_width
                    + NAVIGATION_BAR_ITEM_SPACING)
                    .min(bar_rect.x1),
                bar_rect.y0 + (base_bar_height - title_height) * 0.5,
                (bar_rect.x1
                    - NAVIGATION_BAR_HORIZONTAL_INSET
                    - trailing_width
                    - NAVIGATION_BAR_ITEM_SPACING)
                    .max(bar_rect.x0),
                bar_rect.y0 + (base_bar_height + title_height) * 0.5,
            );
            if title_rect.width() > 0.0 && title_rect.height() > 0.0 {
                Self::dispatch_in_rect_without_accessibility(ctx, env, title, title_rect);
            }

            if let Some(search) = search.as_ref() {
                let search_rect = vello::kurbo::Rect::new(
                    bar_rect.x0 + NAVIGATION_BAR_HORIZONTAL_INSET,
                    bar_rect.y0 + base_bar_height + NAVIGATION_SEARCH_VERTICAL_INSET,
                    bar_rect.x1 - NAVIGATION_BAR_HORIZONTAL_INSET,
                    (bar_rect.y0
                        + base_bar_height
                        + NAVIGATION_SEARCH_VERTICAL_INSET
                        + NAVIGATION_SEARCH_HEIGHT)
                        .min(bar_rect.y1 - NAVIGATION_SEARCH_VERTICAL_INSET),
                );
                if search_rect.width() > 0.0 && search_rect.height() > 0.0 {
                    Self::dispatch_in_rect(
                        ctx,
                        env,
                        AnyView::new(TextField::new(&search.text).prompt(search.prompt.clone())),
                        search_rect,
                    );
                }
            }
        }

        let content_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            (ctx.bounds.y0 + bar_height).min(ctx.bounds.y1),
            ctx.bounds.x1,
            ctx.bounds.y1,
        );
        if content_rect.width() > 0.0 && content_rect.height() > 0.0 {
            Self::dispatch_in_rect(ctx, env, content, content_rect);
        }
    }

    fn render_navigation_split_layout(
        _state: &mut HydroState,
        ctx: RenderContext,
        split: Native<NavigationSplitLayout>,
        env: &Environment,
    ) {
        let split = split.into_inner();
        let compact = ctx.bounds.width() < split_compact_threshold(f64::from(split.sidebar_width));

        if compact && let Some(detail) = split.detail {
            Self::dispatch_in_rect(ctx, env, AnyView::new(detail), ctx.bounds);
            let back_button_rect = navigation_back_button_rect(ctx.bounds);
            {
                let scene = unsafe { ctx.scene() };
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    vello::peniko::Color::new([0.92, 0.92, 0.94, 1.0]),
                    None,
                    &vello::kurbo::RoundedRect::from_rect(back_button_rect, 6.0),
                );
            }
            let clear_selection = Rc::new(RefCell::new(split.clear_selection));
            let env = env.clone();
            let renderer = unsafe { ctx.renderer() };
            renderer.register_pointer_target(
                transformed_rect(ctx.hit_transform, back_button_rect),
                move |_point, _| {
                    let mut action = clear_selection.borrow_mut();
                    (action)(&env);
                    true
                },
            );
            return;
        }

        let sidebar_width = f64::from(split.sidebar_width).min(ctx.bounds.width() * 0.5);
        let sidebar_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            ctx.bounds.x0 + sidebar_width,
            ctx.bounds.y1,
        );
        let detail_rect =
            vello::kurbo::Rect::new(sidebar_rect.x1, ctx.bounds.y0, ctx.bounds.x1, ctx.bounds.y1);
        Self::dispatch_in_rect(ctx, env, split.sidebar, sidebar_rect);
        if let Some(detail) = split.detail {
            Self::dispatch_in_rect(ctx, env, AnyView::new(detail), detail_rect);
        } else {
            Self::dispatch_in_rect(ctx, env, split.placeholder, detail_rect);
        }
    }

    fn render_navigation_stack(
        _state: &mut HydroState,
        ctx: RenderContext,
        stack: Native<NavigationStack<(), ()>>,
        env: &Environment,
    ) {
        let stack = stack.into_inner();
        let transition_style = stack.transition_style();
        let root = stack.into_inner();
        let (slot_index, entries) = {
            let renderer = unsafe { ctx.renderer() };
            renderer.take_pending_navigation_entries("render_navigation_stack")
        };
        let mut local_env = env.clone();
        local_env.insert(NavigationController::new(HydroNavigationController {
            entries: Rc::clone(&entries),
            rebuild_requested: {
                let renderer = unsafe { ctx.renderer() };
                Rc::clone(&renderer.rebuild_requested)
            },
        }));

        let (active, depth) = {
            let entries_ref = entries.borrow();
            let active = entries_ref
                .last()
                .map_or_else(|| root, |builder| AnyView::new(builder.build()));
            (active, entries_ref.len())
        };
        let local_ctx = RenderContext {
            renderer_ptr: ctx.renderer_ptr,
            transform: vello::kurbo::Affine::IDENTITY,
            hit_transform: vello::kurbo::Affine::IDENTITY,
            bounds: ctx.bounds,
        };
        let active_scene = Self::render_subtree_scene(local_ctx, &local_env, active);
        let now = Instant::now();
        let transition_frame = {
            let renderer = unsafe { ctx.renderer() };
            let slot = renderer
                .navigation_slots
                .get_mut(slot_index)
                .expect("hydrolysis navigation slot missing");
            if depth != slot.last_depth {
                if transition_style == NavigationTransition::None || slot.last_scene.is_none() {
                    slot.transition = None;
                } else {
                    let direction = if depth > slot.last_depth {
                        NavigationTransitionDirection::Push
                    } else {
                        NavigationTransitionDirection::Pop
                    };
                    let from_scene = slot
                        .last_scene
                        .take()
                        .expect("hydrolysis navigation transition requires previous scene");
                    slot.transition = Some(NavigationTransitionState::new(
                        transition_style,
                        direction,
                        from_scene,
                        active_scene.clone(),
                        now,
                    ));
                }
                slot.last_depth = depth;
            } else if let Some(transition) = slot.transition.as_ref()
                && !transition.is_active(now)
            {
                slot.transition = None;
            }
            let frame = slot.transition.as_ref().map(|transition| {
                (
                    transition.style,
                    transition.direction,
                    transition.progress(now),
                    transition.from_scene.clone(),
                    transition.to_scene.clone(),
                )
            });
            slot.last_scene = Some(active_scene.clone());
            frame
        };
        let scene = unsafe { ctx.scene() };
        if let Some((style, direction, progress, from_scene, to_scene)) = transition_frame {
            draw_navigation_transition(
                scene,
                ctx.transform,
                ctx.bounds,
                style,
                direction,
                progress,
                &from_scene,
                &to_scene,
            );
        } else {
            scene.append(&active_scene, Some(ctx.transform));
        }

        if depth == 0 {
            return;
        }

        let back_button_rect = navigation_back_button_rect(ctx.bounds);
        {
            let scene = unsafe { ctx.scene() };
            scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                vello::peniko::Color::new([0.92, 0.92, 0.94, 1.0]),
                None,
                &vello::kurbo::RoundedRect::from_rect(back_button_rect, 6.0),
            );
            let chevron = vello::kurbo::BezPath::from_vec(vec![
                vello::kurbo::PathEl::MoveTo(vello::kurbo::Point::new(
                    back_button_rect.x0 + 19.0,
                    back_button_rect.y0 + 10.0,
                )),
                vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(
                    back_button_rect.x0 + 12.0,
                    back_button_rect.y0 + 15.0,
                )),
                vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(
                    back_button_rect.x0 + 19.0,
                    back_button_rect.y0 + 20.0,
                )),
            ]);
            scene.stroke(
                &vello::kurbo::Stroke::new(2.0),
                ctx.transform,
                vello::peniko::Color::new([0.25, 0.25, 0.28, 1.0]),
                None,
                &chevron,
            );
        }

        let entries_for_pop = Rc::clone(&entries);
        let rebuild_requested = {
            let renderer = unsafe { ctx.renderer() };
            Rc::clone(&renderer.rebuild_requested)
        };
        let renderer = unsafe { ctx.renderer() };
        renderer.register_pointer_target(
            transformed_rect(ctx.hit_transform, back_button_rect),
            move |_point, _env| {
                if entries_for_pop.borrow_mut().pop().is_some() {
                    rebuild_requested.set(true);
                    return true;
                }
                false
            },
        );
    }

    fn render_tabs(
        _state: &mut HydroState,
        ctx: RenderContext,
        tabs: Native<Tabs>,
        env: &Environment,
    ) {
        let tabs = tabs.into_inner();
        assert!(
            !(tabs.tabs.is_empty()),
            "hydrolysis Tabs requires at least one tab"
        );

        let tab_count = tabs.tabs.len();
        let position = tabs.position;
        let selection = tabs.selection;
        let selected_id = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&selection)
        };
        let selected_index = tabs
            .tabs
            .iter()
            .position(|tab| tab.label.tag == selected_id)
            .unwrap_or_else(|| panic!("hydrolysis Tabs selection is not present in tabs"));

        let (bar_rect, content_rect) = tabs_bar_and_content_rect(ctx.bounds, position);

        {
            let scene = unsafe { ctx.scene() };
            scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                vello::peniko::Color::new([0.95, 0.95, 0.97, 1.0]),
                None,
                &bar_rect,
            );
            let separator = if matches!(position, TabPosition::Top) {
                vello::kurbo::Rect::new(bar_rect.x0, bar_rect.y1 - 1.0, bar_rect.x1, bar_rect.y1)
            } else {
                vello::kurbo::Rect::new(bar_rect.x0, bar_rect.y0, bar_rect.x1, bar_rect.y0 + 1.0)
            };
            scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                vello::peniko::Color::new([0.82, 0.82, 0.85, 1.0]),
                None,
                &separator,
            );
        }

        let mut selected_content = None;
        for (index, tab) in tabs.tabs.into_iter().enumerate() {
            if index == selected_index {
                selected_content = Some(AnyView::new(tab.content.build()));
            }

            let button_rect = tabs_button_rect(bar_rect, tab_count, index);
            {
                let scene = unsafe { ctx.scene() };
                if index == selected_index {
                    let highlight = inset_rect(button_rect, 4.0, 6.0);
                    scene.fill(
                        vello::peniko::Fill::NonZero,
                        ctx.transform,
                        vello::peniko::Color::new([0.84, 0.9, 1.0, 1.0]),
                        None,
                        &vello::kurbo::RoundedRect::from_rect(highlight, 8.0),
                    );
                }
            }
            let label_rect = inset_rect(button_rect, TABS_BUTTON_HORIZONTAL_INSET, 8.0);
            let tab_id = tab.label.tag;
            if label_rect.width() > 0.0 && label_rect.height() > 0.0 {
                Self::dispatch_in_rect_without_accessibility(
                    ctx,
                    env,
                    tab.label.content,
                    label_rect,
                );
            }

            let selection_binding = selection.clone();
            let renderer = unsafe { ctx.renderer() };
            renderer.register_pointer_target(
                transformed_rect(ctx.hit_transform, button_rect),
                move |_point, _env| {
                    if selection_binding.get() != tab_id {
                        selection_binding.set(tab_id);
                    }
                    true
                },
            );
        }

        if let Some(content) = selected_content {
            if content_rect.width() > 0.0 && content_rect.height() > 0.0 {
                Self::dispatch_in_rect(ctx, env, content, content_rect);
            }
        }
    }

    fn render_list(
        state: &mut HydroState,
        ctx: RenderContext,
        list: Native<ListConfig>,
        env: &Environment,
    ) {
        let list = list.into_inner();
        let editing = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&list.editing)
        };
        let row_count_signal = list.contents.len();
        let row_count = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&row_count_signal)
        };
        let contents = list.contents.clone();
        let slot_index = {
            let renderer = unsafe { ctx.renderer() };
            let index = renderer.lazy_list_controller.bind();
            renderer.lazy_list_controller.slots[index].prepare_len(row_count);
            index
        };

        let viewport = ctx.bounds;
        let handle = {
            let renderer = unsafe { ctx.renderer() };
            renderer.take_pending_scroll_handle("render_list")
        };
        let metrics = handle.metrics();
        {
            let renderer = unsafe { ctx.renderer() };
            renderer.push_layer_rect(1.0, ctx.transform, viewport);
        }

        let window = resolve_visible_index_window(
            row_count,
            metrics.offset_y,
            metrics.offset_y + viewport.height(),
            |index| {
                let cached_extent = {
                    let renderer = unsafe { ctx.renderer() };
                    renderer.lazy_list_controller.slots[slot_index].row_extents[index]
                };
                if let Some(extent) = cached_extent {
                    return extent;
                }
                let (_item, extent) = materialize_list_row(&contents, index, state, env);
                {
                    let renderer = unsafe { ctx.renderer() };
                    renderer.lazy_list_controller.slots[slot_index].row_extents[index] =
                        Some(extent);
                }
                extent
            },
        );
        let delete_action = list.on_delete.map(Rc::new);
        let move_action = list.on_move.map(Rc::new);
        let total_rows = row_count;
        let mut y = viewport.y0 - metrics.offset_y + window.leading_offset;
        for index in window.start..window.end {
            let item = materialize_list_item(&contents, index, env);
            let row_height = {
                let cached_extent = {
                    let renderer = unsafe { ctx.renderer() };
                    renderer.lazy_list_controller.slots[slot_index].row_extents[index]
                };
                if let Some(extent) = cached_extent {
                    extent
                } else {
                    let extent = measure_list_item_row_height(&item, state, env);
                    let renderer = unsafe { ctx.renderer() };
                    renderer.lazy_list_controller.slots[slot_index].row_extents[index] =
                        Some(extent);
                    extent
                }
            };
            let row_rect = vello::kurbo::Rect::new(viewport.x0, y, viewport.x1, y + row_height);
            y += row_height;
            if row_rect.y1 <= viewport.y0 || row_rect.y0 >= viewport.y1 {
                continue;
            }
            {
                let scene = unsafe { ctx.scene() };
                let row_color = if index % 2 == 0 {
                    vello::peniko::Color::new([1.0, 1.0, 1.0, 1.0])
                } else {
                    vello::peniko::Color::new([0.985, 0.985, 0.99, 1.0])
                };
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    row_color,
                    None,
                    &row_rect,
                );
            }

            let deletable = {
                let renderer = unsafe { ctx.renderer() };
                renderer.read_signal(&item.deletable)
            };
            let mut content_rect = inset_rect(
                row_rect,
                LIST_ROW_HORIZONTAL_PADDING,
                LIST_ROW_VERTICAL_PADDING,
            );
            let mut trailing_x = row_rect.x1 - 8.0;

            if editing && move_action.is_some() {
                let control_width = LIST_MOVE_CONTROL_WIDTH;
                let control_height = (row_height - 12.0).max(12.0);
                let control_rect = vello::kurbo::Rect::new(
                    trailing_x - control_width,
                    row_rect.y0 + 6.0,
                    trailing_x,
                    row_rect.y0 + 6.0 + control_height,
                );
                trailing_x -= control_width + LIST_TRAILING_CONTROL_SPACING;
                {
                    let scene = unsafe { ctx.scene() };
                    draw_stepper_button(scene, ctx.transform, control_rect);
                    let split = control_rect.y0 + control_rect.height() / 2.0;
                    let separator = vello::kurbo::Line::new(
                        (control_rect.x0 + 3.0, split),
                        (control_rect.x1 - 3.0, split),
                    );
                    scene.stroke(
                        &vello::kurbo::Stroke::new(1.0),
                        ctx.transform,
                        vello::peniko::Color::new([0.65, 0.65, 0.68, 1.0]),
                        None,
                        &separator,
                    );
                }

                if index > 0 {
                    let up_rect = vello::kurbo::Rect::new(
                        control_rect.x0,
                        control_rect.y0,
                        control_rect.x1,
                        control_rect.y0 + control_rect.height() / 2.0,
                    );
                    let action = Rc::clone(move_action.as_ref().expect("move action missing"));
                    let renderer = unsafe { ctx.renderer() };
                    renderer.register_pointer_target(
                        transformed_rect(ctx.hit_transform, up_rect),
                        move |_point, env| {
                            (action.as_ref())(env, Move::new(index, index - 1));
                            true
                        },
                    );
                }
                if index + 1 < total_rows {
                    let down_rect = vello::kurbo::Rect::new(
                        control_rect.x0,
                        control_rect.y0 + control_rect.height() / 2.0,
                        control_rect.x1,
                        control_rect.y1,
                    );
                    let action = Rc::clone(move_action.as_ref().expect("move action missing"));
                    let renderer = unsafe { ctx.renderer() };
                    renderer.register_pointer_target(
                        transformed_rect(ctx.hit_transform, down_rect),
                        move |_point, env| {
                            (action.as_ref())(env, Move::new(index, index + 1));
                            true
                        },
                    );
                }
            }

            if editing && deletable && delete_action.is_some() {
                let delete_rect = vello::kurbo::Rect::new(
                    trailing_x - LIST_DELETE_CONTROL_WIDTH,
                    row_rect.y0 + 6.0,
                    trailing_x,
                    row_rect.y1 - 6.0,
                );
                trailing_x = delete_rect.x0 - LIST_TRAILING_CONTROL_SPACING;
                {
                    let scene = unsafe { ctx.scene() };
                    scene.fill(
                        vello::peniko::Fill::NonZero,
                        ctx.transform,
                        vello::peniko::Color::new([0.91, 0.25, 0.2, 1.0]),
                        None,
                        &vello::kurbo::RoundedRect::from_rect(delete_rect, 5.0),
                    );
                }
                let action = Rc::clone(delete_action.as_ref().expect("delete action missing"));
                let renderer = unsafe { ctx.renderer() };
                renderer.register_pointer_target(
                    transformed_rect(ctx.hit_transform, delete_rect),
                    move |_point, env| {
                        (action.as_ref())(env, index);
                        true
                    },
                );
            }

            content_rect.x1 = content_rect.x1.min(trailing_x);
            if content_rect.width() > 0.0 && content_rect.height() > 0.0 {
                Self::dispatch_in_rect_without_accessibility(ctx, env, item.content, content_rect);
            }

            {
                let scene = unsafe { ctx.scene() };
                let separator = vello::kurbo::Rect::new(
                    row_rect.x0 + 8.0,
                    row_rect.y1 - 1.0,
                    row_rect.x1 - 8.0,
                    row_rect.y1,
                );
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    vello::peniko::Color::new([0.9, 0.9, 0.92, 1.0]),
                    None,
                    &separator,
                );
            }
        }

        {
            let renderer = unsafe { ctx.renderer() };
            renderer.pop_layer();
        }

        let renderer = unsafe { ctx.renderer() };
        let handle_for_input = handle.clone();
        renderer.register_scroll_target(
            transformed_rect(ctx.hit_transform, viewport),
            move |dx, dy, is_line_delta| handle_for_input.apply_scroll_delta(dx, dy, is_line_delta),
        );
        let scene = unsafe { ctx.scene() };
        Self::draw_scroll_indicators(
            scene,
            ctx.transform,
            viewport,
            metrics,
            ScrollAxis::Vertical,
        );
    }

    fn render_table(
        state: &mut HydroState,
        ctx: RenderContext,
        table: Native<TableConfig>,
        env: &Environment,
    ) {
        let table = table.into_inner();
        let columns = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&table.columns)
        };
        if columns.is_empty() {
            return;
        }
        let viewport = ctx.bounds;
        let handle = {
            let renderer = unsafe { ctx.renderer() };
            renderer.take_pending_scroll_handle("render_table")
        };
        let scroll_metrics = handle.metrics();
        let slot_index = {
            let renderer = unsafe { ctx.renderer() };
            let index = renderer.lazy_table_controller.bind();
            index
        };
        {
            let renderer = unsafe { ctx.renderer() };
            let slot = &mut renderer.lazy_table_controller.slots[slot_index];
            refresh_table_slot_baseline(&columns, slot, state, env);
        }
        let row_window = {
            let renderer = unsafe { ctx.renderer() };
            let slot = &renderer.lazy_table_controller.slots[slot_index];
            resolve_table_visible_rows(scroll_metrics.offset_y, viewport.height(), slot.max_rows)
        };
        let mut column_window = {
            let renderer = unsafe { ctx.renderer() };
            let slot = &renderer.lazy_table_controller.slots[slot_index];
            resolve_visible_column_window(
                &slot.column_widths,
                scroll_metrics.offset_x,
                scroll_metrics.offset_x + viewport.width(),
            )
        };
        {
            let renderer = unsafe { ctx.renderer() };
            let slot = &mut renderer.lazy_table_controller.slots[slot_index];
            update_table_slot_visible_cell_widths(
                &columns,
                slot,
                row_window,
                column_window,
                state,
                env,
            );
        }
        {
            let renderer = unsafe { ctx.renderer() };
            let slot = &renderer.lazy_table_controller.slots[slot_index];
            column_window = resolve_visible_column_window(
                &slot.column_widths,
                scroll_metrics.offset_x,
                scroll_metrics.offset_x + viewport.width(),
            );
        }
        let table_metrics = {
            let renderer = unsafe { ctx.renderer() };
            let slot = &renderer.lazy_table_controller.slots[slot_index];
            table_metrics_from_slot(slot)
        };

        {
            let renderer = unsafe { ctx.renderer() };
            renderer.push_layer_rect(1.0, ctx.transform, viewport);
        }

        let origin_x = viewport.x0 - scroll_metrics.offset_x;
        let origin_y = viewport.y0 - scroll_metrics.offset_y;
        {
            let scene = unsafe { ctx.scene() };
            let header_rect = vello::kurbo::Rect::new(
                origin_x,
                origin_y,
                origin_x + table_metrics.table_width,
                origin_y + TABLE_HEADER_HEIGHT,
            );
            scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                vello::peniko::Color::new([0.95, 0.95, 0.96, 1.0]),
                None,
                &header_rect,
            );
        }

        let mut x_offset = column_window.leading_offset;
        for column_index in column_window.start..column_window.end {
            let column = &columns[column_index];
            let width = table_metrics.column_widths[column_index];
            let header_cell = table_header_cell_rect(origin_x, origin_y, x_offset, width);
            let header_view = AnyView::new(column.label());
            Self::dispatch_in_rect_without_accessibility(
                ctx,
                env,
                header_view,
                inset_rect(header_cell, 8.0, 6.0),
            );

            let rows = column.rows();
            for row_index in row_window.start..row_window.end {
                let cell_rect =
                    table_data_cell_rect(origin_x, origin_y, x_offset, width, row_index);
                if let Some(cell) = rows.get_view(row_index) {
                    let cell_view = AnyView::new(cell);
                    Self::dispatch_in_rect_without_accessibility(
                        ctx,
                        env,
                        cell_view,
                        inset_rect(cell_rect, 8.0, 6.0),
                    );
                }
                let scene = unsafe { ctx.scene() };
                scene.stroke(
                    &vello::kurbo::Stroke::new(1.0),
                    ctx.transform,
                    vello::peniko::Color::new([0.85, 0.85, 0.87, 1.0]),
                    None,
                    &cell_rect,
                );
            }

            let separator = vello::kurbo::Line::new(
                (origin_x + x_offset + width, origin_y),
                (
                    origin_x + x_offset + width,
                    origin_y + table_metrics.table_height,
                ),
            );
            let scene = unsafe { ctx.scene() };
            scene.stroke(
                &vello::kurbo::Stroke::new(1.0),
                ctx.transform,
                vello::peniko::Color::new([0.8, 0.8, 0.83, 1.0]),
                None,
                &separator,
            );
            x_offset += width;
        }

        {
            let renderer = unsafe { ctx.renderer() };
            renderer.pop_layer();
        }

        let renderer = unsafe { ctx.renderer() };
        let handle_for_input = handle.clone();
        renderer.register_scroll_target(
            transformed_rect(ctx.hit_transform, viewport),
            move |dx, dy, is_line_delta| handle_for_input.apply_scroll_delta(dx, dy, is_line_delta),
        );
        let scene = unsafe { ctx.scene() };
        Self::draw_scroll_indicators(
            scene,
            ctx.transform,
            viewport,
            scroll_metrics,
            ScrollAxis::All,
        );
    }

    fn render_divider(
        _state: &mut HydroState,
        ctx: RenderContext,
        _divider: Divider,
        env: &Environment,
    ) {
        let vertical = matches!(env.get::<StackAxis>(), Some(StackAxis::Horizontal));
        let rect = if vertical {
            vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x0 + 1.0,
                ctx.bounds.y1,
            )
        } else {
            vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                ctx.bounds.y0 + 1.0,
            )
        };

        let scene = unsafe { ctx.scene() };
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            vello::peniko::Color::new([0.75, 0.75, 0.75, 1.0]),
            None,
            &rect,
        );
    }

    fn render_str(state: &mut HydroState, ctx: RenderContext, text: Str, env: &Environment) {
        #[cfg(feature = "accessibility")]
        {
            if !env
                .get::<AccessibilityHidden>()
                .is_some_and(AccessibilityHidden::is_hidden)
            {
                let renderer = unsafe { ctx.renderer() };
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
            state,
            ctx,
            StyledStr::plain(text),
            HorizontalAlignment::Leading,
            env,
        );
    }

    fn render_text_config(
        state: &mut HydroState,
        ctx: RenderContext,
        text: Native<TextConfig>,
        env: &Environment,
    ) {
        let text = text.into_inner();
        let styled = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&text.content)
        };
        let alignment = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&text.paragraph_alignment)
        };
        Self::render_styled_text(state, ctx, styled, alignment, env);
    }

    fn render_styled_text(
        state: &mut HydroState,
        ctx: RenderContext,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
    ) {
        Self::render_styled_text_limited(state, ctx, styled, alignment, env, None);
    }

    fn render_styled_text_limited(
        state: &mut HydroState,
        ctx: RenderContext,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        max_lines: Option<usize>,
    ) {
        let layout = Self::build_text_layout(
            state,
            styled,
            alignment,
            env,
            Some(ctx.bounds.width() as f32),
        );
        Self::draw_text_layout(ctx, &layout, max_lines);
    }

    fn draw_text_layout(
        ctx: RenderContext,
        layout: &parley::Layout<[u8; 4]>,
        max_lines: Option<usize>,
    ) {
        if layout.is_empty() {
            return;
        }
        let text_transform =
            ctx.transform * vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0));
        let scene = unsafe { ctx.scene() };
        for (index, line) in layout.lines().enumerate() {
            if max_lines.is_some_and(|limit| index >= limit) {
                break;
            }
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    let run = glyph_run.run();
                    let style = glyph_run.style();
                    let brush = rgba8_to_peniko(style.brush);
                    let normalized_coords: Vec<vello::NormalizedCoord> =
                        run.normalized_coords().to_vec();

                    let mut run_x = glyph_run.offset();
                    let run_y = glyph_run.baseline();
                    let glyphs = glyph_run.glyphs().map(move |glyph| {
                        let x = run_x + glyph.x;
                        let y = run_y - glyph.y;
                        run_x += glyph.advance;
                        vello::Glyph { id: glyph.id, x, y }
                    });

                    scene
                        .draw_glyphs(run.font())
                        .brush(brush)
                        .transform(text_transform)
                        .font_size(run.font_size())
                        .normalized_coords(&normalized_coords)
                        .draw(vello::peniko::Fill::NonZero, glyphs);
                }
            }
        }
    }

    fn default_text_brush(env: &Environment) -> [u8; 4] {
        let color = theme::installed_color_signal::<theme::color::Foreground>(env).map_or_else(
            || Color::srgb(0, 0, 0).resolve(env).get(),
            |signal| signal.get(),
        );
        resolved_color_to_rgba8(color)
    }

    fn build_text_layout(
        state: &mut HydroState,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        max_width: Option<f32>,
    ) -> parley::Layout<[u8; 4]> {
        let mut plain = String::new();
        let mut spans = Vec::with_capacity(styled.chunks().len());
        for (chunk, style) in styled.chunks() {
            let start = plain.len();
            plain.push_str(chunk.as_str());
            let end = plain.len();
            spans.push((start..end, style.clone()));
        }

        if plain.is_empty() {
            return parley::Layout::new();
        }

        let mut family_storage = Vec::new();
        let default_font = waterui_text::font::Font::default().resolve(env).get();
        let default_brush = Self::default_text_brush(env);
        let mut builder = state
            .layout_cx
            .ranged_builder(&mut state.font_cx, &plain, 1.0, true);
        builder.push_default(parley::StyleProperty::Brush(default_brush));
        builder.push_default(parley::StyleProperty::FontSize(default_font.size));
        builder.push_default(parley::StyleProperty::FontWeight(parley_font_weight(
            default_font.weight,
        )));
        let default_font_stack = if let Some(family) = default_font.family {
            family_storage.push(family.to_string());
            let family_name = family_storage
                .last()
                .expect("default font family storage must contain the pushed value");
            parley::FontStack::Single(parley::FontFamily::Named(Cow::Borrowed(
                family_name.as_str(),
            )))
        } else {
            parley::FontStack::Single(parley::FontFamily::Generic(
                parley::style::GenericFamily::SansSerif,
            ))
        };
        builder.push_default(parley::StyleProperty::FontStack(default_font_stack));

        for (range, style) in spans {
            Self::push_text_style(&mut builder, &mut family_storage, style, range, env);
        }

        let mut layout = builder.build(&plain);
        layout.break_all_lines(max_width);
        layout.align(
            max_width,
            parley_alignment(alignment),
            parley::AlignmentOptions::default(),
        );
        layout
    }

    fn measure_text_dimensions(
        state: &mut HydroState,
        styled: StyledStr,
        alignment: HorizontalAlignment,
        env: &Environment,
        max_width: Option<f32>,
        max_lines: Option<usize>,
    ) -> ViewDimensions {
        let layout = Self::build_text_layout(state, styled, alignment, env, max_width);
        if layout.is_empty() {
            return ViewDimensions::new(LayoutSize::zero());
        }

        let mut width = 0.0_f32;
        let mut height = 0.0_f32;
        let mut first_baseline = None;
        let mut last_baseline = None;

        for (index, line) in layout.lines().enumerate() {
            if max_lines.is_some_and(|limit| index >= limit) {
                break;
            }
            let metrics = line.metrics();
            width = width.max(metrics.advance);
            height += metrics.line_height;
            if first_baseline.is_none() {
                first_baseline = Some(metrics.baseline);
            }
            last_baseline = Some(metrics.baseline);
        }

        let mut dimensions = ViewDimensions::new(LayoutSize::new(width, height));
        if let Some(first_baseline) = first_baseline {
            dimensions.set_vertical(VerticalAlignment::FirstBaseline, first_baseline);
        }
        if let Some(last_baseline) = last_baseline {
            dimensions.set_vertical(VerticalAlignment::LastBaseline, last_baseline);
        }
        dimensions
    }

    fn measure_text_intrinsic_size(
        state: &mut HydroState,
        styled: StyledStr,
        env: &Environment,
    ) -> LayoutSize {
        Self::measure_text_dimensions(state, styled, HorizontalAlignment::Leading, env, None, None)
            .size
    }

    fn measure_text_intrinsic_size_with_line_limit(
        state: &mut HydroState,
        styled: StyledStr,
        env: &Environment,
        max_lines: Option<usize>,
    ) -> LayoutSize {
        Self::measure_text_dimensions(
            state,
            styled,
            HorizontalAlignment::Leading,
            env,
            None,
            max_lines,
        )
        .size
    }

    fn push_text_style(
        builder: &mut parley::RangedBuilder<'_, [u8; 4]>,
        family_storage: &mut Vec<String>,
        style: TextStyle,
        range: std::ops::Range<usize>,
        env: &Environment,
    ) {
        let resolved_font = style.font.resolve(env).get();
        builder.push(
            parley::StyleProperty::FontSize(resolved_font.size),
            range.clone(),
        );
        builder.push(
            parley::StyleProperty::FontWeight(parley_font_weight(resolved_font.weight)),
            range.clone(),
        );
        if let Some(family) = resolved_font.family {
            family_storage.push(family.to_string());
            let family_name = family_storage
                .last()
                .expect("font family storage must contain the pushed value");
            builder.push(
                parley::StyleProperty::FontStack(parley::FontStack::Single(
                    parley::FontFamily::Named(Cow::Borrowed(family_name.as_str())),
                )),
                range.clone(),
            );
        }
        builder.push(
            parley::StyleProperty::FontStyle(if style.italic {
                parley::FontStyle::Italic
            } else {
                parley::FontStyle::Normal
            }),
            range.clone(),
        );
        builder.push(
            parley::StyleProperty::Underline(style.underline),
            range.clone(),
        );
        builder.push(
            parley::StyleProperty::Strikethrough(style.strikethrough),
            range.clone(),
        );
        if let Some(color) = style.foreground {
            builder.push(
                parley::StyleProperty::Brush(resolved_color_to_rgba8(color.resolve(env).get())),
                range,
            );
        }
    }

    fn render_button(
        _state: &mut HydroState,
        ctx: RenderContext,
        button: Native<ButtonConfig>,
        env: &Environment,
    ) {
        let button = button.into_inner();
        let style = button.style;
        {
            let scene = unsafe { ctx.scene() };
            draw_button_chrome(scene, ctx.transform, ctx.bounds, style);
        }

        let (padding_x, padding_y) = button_padding(style);
        let label_bounds = inset_rect(ctx.bounds, padding_x, padding_y);
        if label_bounds.width() > 0.0 && label_bounds.height() > 0.0 {
            Self::dispatch_in_rect_without_accessibility(ctx, env, button.label, label_bounds);
        } else {
            Self::dispatch_any_without_accessibility(ctx, env, button.label);
        }

        let hit_bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        {
            let renderer = unsafe { ctx.renderer() };
            let mut action = button.action;
            renderer.register_pointer_target(hit_bounds, move |_point, env| {
                action(env);
                true
            });
        }
    }

    fn render_menu(
        _state: &mut HydroState,
        ctx: RenderContext,
        menu: Native<ResolvedMenu>,
        env: &Environment,
    ) {
        let menu = menu.into_inner();
        let ResolvedMenu {
            label,
            items,
            accessibility_label: _,
        } = menu;
        let style = ButtonStyle::Borderless;
        {
            let scene = unsafe { ctx.scene() };
            draw_button_chrome(scene, ctx.transform, ctx.bounds, style);
        }

        let (padding_x, padding_y) = button_padding(style);
        let label_bounds = inset_rect(ctx.bounds, padding_x, padding_y);
        if label_bounds.width() > 0.0 && label_bounds.height() > 0.0 {
            Self::dispatch_in_rect_without_accessibility(ctx, env, label, label_bounds);
        } else {
            Self::dispatch_any_without_accessibility(ctx, env, label);
        }

        let hit_bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let anchor = LayoutPoint::new(hit_bounds.x0 as f32, hit_bounds.y1 as f32);
        let items = items;
        let renderer_ptr = ctx.renderer_ptr;
        let renderer = unsafe { ctx.renderer() };
        renderer.register_pointer_target(hit_bounds, move |_point, env| unsafe {
            (&mut *renderer_ptr).show_popup_menu_nodes(popup_menu_nodes(&items.get()), anchor, env)
        });
    }

    fn render_toggle(
        _state: &mut HydroState,
        ctx: RenderContext,
        toggle: Native<ToggleConfig>,
        env: &Environment,
    ) {
        let toggle = toggle.into_inner();
        let style = toggle.style;
        let (control_width, control_height) = toggle_control_size(style);
        let spacing = TOGGLE_LABEL_SPACING;
        let control_x0 = (ctx.bounds.x1 - control_width).max(ctx.bounds.x0);
        let control_y0 = ctx.bounds.y0 + ((ctx.bounds.height() - control_height) / 2.0).max(0.0);
        let control_bounds = vello::kurbo::Rect::new(
            control_x0,
            control_y0,
            control_x0 + control_width,
            control_y0 + control_height,
        );
        let label_bounds = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0,
            (control_x0 - spacing).max(ctx.bounds.x0),
            ctx.bounds.y1,
        );
        if label_bounds.width() > 0.0 {
            Self::dispatch_in_rect_without_accessibility(ctx, env, toggle.label, label_bounds);
        }

        let thumb_progress = {
            let renderer = unsafe { ctx.renderer() };
            renderer.resolve_toggle_progress(&toggle.toggle)
        };
        let track_color = lerp_color(
            [0.86, 0.86, 0.88, 1.0],
            [0.2, 0.45, 0.9, 1.0],
            thumb_progress,
        );
        let scene = unsafe { ctx.scene() };
        match style {
            ToggleStyle::Automatic | ToggleStyle::Switch => {
                draw_toggle_switch(
                    scene,
                    ctx.transform,
                    control_bounds,
                    thumb_progress,
                    track_color,
                );
            }
            ToggleStyle::Checkbox => {
                draw_toggle_checkbox(scene, ctx.transform, control_bounds, thumb_progress);
            }
            _ => panic!("hydrolysis ToggleStyle variant is not implemented"),
        }

        let hit_bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let toggle_binding = toggle.toggle;
        let renderer = unsafe { ctx.renderer() };
        renderer.register_pointer_target(hit_bounds, move |_point, _env| {
            let next = !toggle_binding.get();
            toggle_binding.set(next);
            true
        });
    }

    fn render_slider(
        state: &mut HydroState,
        ctx: RenderContext,
        slider: Native<SliderConfig>,
        env: &Environment,
    ) {
        let mut slider = slider.into_inner();
        slider.label = normalize_layout_view(slider.label, env);
        slider.min_value_label = normalize_layout_view(slider.min_value_label, env);
        slider.max_value_label = normalize_layout_view(slider.max_value_label, env);
        let label_height = if ctx.bounds.height() >= 36.0 {
            f64::from(measure_view_intrinsic(&slider.label, state, env).height).max(20.0)
        } else {
            0.0
        };
        if label_height > 0.0 {
            let label_rect = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
            );
            Self::dispatch_in_rect_without_accessibility(ctx, env, slider.label, label_rect);
        }

        let min_label_size = measure_view_intrinsic(&slider.min_value_label, state, env);
        let max_label_size = measure_view_intrinsic(&slider.max_value_label, state, env);
        let min_label_width = f64::from(min_label_size.width);
        let max_label_width = f64::from(max_label_size.width);
        let min_label_x0 = ctx.bounds.x0 + SLIDER_HORIZONTAL_INSET;
        let min_label_x1 = min_label_x0 + min_label_width;
        let max_label_x1 = ctx.bounds.x1 - SLIDER_HORIZONTAL_INSET;
        let max_label_x0 = max_label_x1 - max_label_width;
        let control_top = ctx.bounds.y0 + label_height;
        let control_bottom = ctx.bounds.y1;
        let control_height = control_bottom - control_top;

        if min_label_width > 0.0 && control_height > 0.0 {
            let min_label_rect =
                vello::kurbo::Rect::new(min_label_x0, control_top, min_label_x1, control_bottom);
            Self::dispatch_in_rect_without_accessibility(
                ctx,
                env,
                slider.min_value_label,
                min_label_rect,
            );
        }
        if max_label_width > 0.0 && control_height > 0.0 {
            let max_label_rect =
                vello::kurbo::Rect::new(max_label_x0, control_top, max_label_x1, control_bottom);
            Self::dispatch_in_rect_without_accessibility(
                ctx,
                env,
                slider.max_value_label,
                max_label_rect,
            );
        }

        let range_start = *slider.range.start();
        let range_end = *slider.range.end();
        let span = range_end - range_start;
        assert!(
            !(span <= 0.0),
            "hydrolysis slider requires range start < end"
        );

        let track_left = if min_label_width > 0.0 {
            min_label_x1 + SLIDER_HORIZONTAL_SPACING
        } else {
            ctx.bounds.x0 + SLIDER_HORIZONTAL_INSET
        };
        let track_right = if max_label_width > 0.0 {
            max_label_x0 - SLIDER_HORIZONTAL_SPACING
        } else {
            ctx.bounds.x1 - SLIDER_HORIZONTAL_INSET
        };
        let track_center_y = control_top + control_height / 2.0;
        let track_rect = vello::kurbo::Rect::new(
            track_left,
            track_center_y - SLIDER_TRACK_HEIGHT / 2.0,
            track_right,
            track_center_y + SLIDER_TRACK_HEIGHT / 2.0,
        );

        let clamped = {
            let renderer = unsafe { ctx.renderer() };
            renderer
                .read_signal(&slider.value)
                .clamp(range_start, range_end)
        };
        let progress = (clamped - range_start) / span;
        let fill_right = track_left + (track_right - track_left) * progress;
        let fill_rect = vello::kurbo::Rect::new(
            track_left,
            track_center_y - SLIDER_TRACK_HEIGHT / 2.0,
            fill_right,
            track_center_y + SLIDER_TRACK_HEIGHT / 2.0,
        );
        let thumb = vello::kurbo::Circle::new(
            vello::kurbo::Point::new(fill_right, track_center_y),
            SLIDER_THUMB_RADIUS,
        );

        let scene = unsafe { ctx.scene() };
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            vello::peniko::Color::new([0.8, 0.82, 0.87, 1.0]),
            None,
            &track_rect,
        );
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            vello::peniko::Color::new([0.2, 0.45, 0.9, 1.0]),
            None,
            &fill_rect,
        );
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            vello::peniko::Color::WHITE,
            None,
            &thumb,
        );
        scene.stroke(
            &vello::kurbo::Stroke::new(1.0),
            ctx.transform,
            vello::peniko::Color::new([0.7, 0.73, 0.78, 1.0]),
            None,
            &thumb,
        );

        let hit_bounds = transformed_rect(
            ctx.hit_transform,
            vello::kurbo::Rect::new(
                track_left - SLIDER_THUMB_RADIUS,
                control_top,
                track_right + SLIDER_THUMB_RADIUS,
                control_bottom,
            ),
        );
        let value_binding = slider.value;
        let usable_track = track_right - track_left;
        assert!(
            !(usable_track <= 0.0),
            "hydrolysis slider resolved a non-positive track width"
        );
        let inverse_transform = ctx.hit_transform.inverse();
        let value_epsilon = slider_value_epsilon(span, usable_track);
        tracing::trace!(
            target: "waterui::hydrolysis::hit_region",
            component = "slider",
            layout_bounds = ?ctx.bounds,
            hit_bounds = ?hit_bounds,
            track_left,
            track_right,
            control_top,
            control_bottom,
            "register slider drag region"
        );
        let renderer = unsafe { ctx.renderer() };
        renderer.register_pointer_drag_target(hit_bounds, move |point, _env| {
            let local_point = inverse_transform * point;
            let x = local_point.x.clamp(track_left, track_right);
            let t = (x - track_left) / usable_track;
            let next = range_start + span * t;
            if (value_binding.get() - next).abs() <= value_epsilon {
                return false;
            }
            value_binding.set(next);
            true
        });
    }

    fn render_stepper(
        _state: &mut HydroState,
        ctx: RenderContext,
        stepper: Native<StepperConfig>,
        env: &Environment,
    ) {
        let stepper = stepper.into_inner();
        let button_size = ctx
            .bounds
            .height()
            .clamp(STEPPER_BUTTON_MIN_SIZE, STEPPER_BUTTON_MAX_SIZE);
        let spacing = STEPPER_BUTTON_SPACING;
        let controls_width = button_size * 2.0 + spacing;
        let controls_x0 = (ctx.bounds.x1 - controls_width).max(ctx.bounds.x0);

        let label_bounds =
            vello::kurbo::Rect::new(ctx.bounds.x0, ctx.bounds.y0, controls_x0, ctx.bounds.y1);
        if label_bounds.width() > 0.0 {
            Self::dispatch_in_rect_without_accessibility(ctx, env, stepper.label, label_bounds);
        }

        let button_y0 = ctx.bounds.y0 + ((ctx.bounds.height() - button_size) / 2.0).max(0.0);
        let minus_bounds = vello::kurbo::Rect::new(
            controls_x0,
            button_y0,
            controls_x0 + button_size,
            button_y0 + button_size,
        );
        let plus_bounds = vello::kurbo::Rect::new(
            controls_x0 + button_size + spacing,
            button_y0,
            controls_x0 + controls_width,
            button_y0 + button_size,
        );
        let scene = unsafe { ctx.scene() };
        draw_stepper_button(scene, ctx.transform, minus_bounds);
        draw_stepper_button(scene, ctx.transform, plus_bounds);

        let line_color = vello::peniko::Color::new([0.2, 0.2, 0.22, 1.0]);
        let minus_line = vello::kurbo::Line::new(
            (minus_bounds.x0 + 6.0, minus_bounds.y0 + button_size / 2.0),
            (minus_bounds.x1 - 6.0, minus_bounds.y0 + button_size / 2.0),
        );
        let plus_horizontal = vello::kurbo::Line::new(
            (plus_bounds.x0 + 6.0, plus_bounds.y0 + button_size / 2.0),
            (plus_bounds.x1 - 6.0, plus_bounds.y0 + button_size / 2.0),
        );
        let plus_vertical = vello::kurbo::Line::new(
            (plus_bounds.x0 + button_size / 2.0, plus_bounds.y0 + 6.0),
            (plus_bounds.x0 + button_size / 2.0, plus_bounds.y1 - 6.0),
        );
        let stroke = vello::kurbo::Stroke::new(2.0);
        scene.stroke(&stroke, ctx.transform, line_color, None, &minus_line);
        scene.stroke(&stroke, ctx.transform, line_color, None, &plus_horizontal);
        scene.stroke(&stroke, ctx.transform, line_color, None, &plus_vertical);

        let range_start = *stepper.range.start();
        let range_end = *stepper.range.end();
        assert!(
            !(range_start > range_end),
            "hydrolysis stepper requires an ordered range"
        );

        let value_binding_minus = stepper.value.clone();
        let value_binding_plus = stepper.value;
        let step_signal_minus = stepper.step.clone();
        let step_signal_plus = stepper.step;

        let renderer = unsafe { ctx.renderer() };
        tracing::trace!(
            target: "waterui::hydrolysis::hit_region",
            component = "stepper",
            layout_bounds = ?ctx.bounds,
            minus_bounds = ?transformed_rect(ctx.hit_transform, minus_bounds),
            plus_bounds = ?transformed_rect(ctx.hit_transform, plus_bounds),
            "register stepper button regions"
        );
        renderer.register_pointer_target(
            transformed_rect(ctx.hit_transform, minus_bounds),
            move |_point, _env| {
                let step = step_signal_minus.get();
                assert!(!(step <= 0), "hydrolysis stepper requires positive step");
                let current = value_binding_minus.get();
                let next = current.saturating_sub(step).clamp(range_start, range_end);
                value_binding_minus.set(next);
                true
            },
        );
        renderer.register_pointer_target(
            transformed_rect(ctx.hit_transform, plus_bounds),
            move |_point, _env| {
                let step = step_signal_plus.get();
                assert!(!(step <= 0), "hydrolysis stepper requires positive step");
                let current = value_binding_plus.get();
                let next = current.saturating_add(step).clamp(range_start, range_end);
                value_binding_plus.set(next);
                true
            },
        );
    }

    fn render_progress(
        state: &mut HydroState,
        ctx: RenderContext,
        progress: Native<ProgressConfig>,
        env: &Environment,
    ) {
        let mut progress = progress.into_inner();
        progress.label = normalize_layout_view(progress.label, env);
        progress.value_label = normalize_layout_view(progress.value_label, env);
        let clamped = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&progress.value).clamp(0.0, 1.0) as f64
        };

        match progress.style {
            ProgressStyle::Linear => {
                let label_size = measure_view_intrinsic(&progress.label, state, env);
                let label_height = if label_size.width > 0.0 || label_size.height > 0.0 {
                    f64::from(label_size.height).max(PROGRESS_LINEAR_LABEL_HEIGHT)
                } else {
                    0.0
                };
                if label_height > 0.0 {
                    let label_rect = vello::kurbo::Rect::new(
                        ctx.bounds.x0,
                        ctx.bounds.y0,
                        ctx.bounds.x1,
                        (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
                    );
                    Self::dispatch_in_rect_without_accessibility(
                        ctx,
                        env,
                        progress.label,
                        label_rect,
                    );
                }

                let bar_y = ctx.bounds.y0 + label_height + PROGRESS_LINEAR_BAR_TOP_OFFSET;
                let bar = vello::kurbo::RoundedRect::from_rect(
                    vello::kurbo::Rect::new(
                        ctx.bounds.x0 + PROGRESS_LINEAR_BAR_HORIZONTAL_INSET,
                        bar_y,
                        ctx.bounds.x1 - PROGRESS_LINEAR_BAR_HORIZONTAL_INSET,
                        bar_y + PROGRESS_LINEAR_BAR_HEIGHT,
                    ),
                    4.0,
                );
                let width = bar.rect().width() * clamped;
                let fill = vello::kurbo::RoundedRect::from_rect(
                    vello::kurbo::Rect::new(
                        bar.rect().x0,
                        bar.rect().y0,
                        bar.rect().x0 + width,
                        bar.rect().y1,
                    ),
                    4.0,
                );
                let scene = unsafe { ctx.scene() };
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    vello::peniko::Color::new([0.84, 0.84, 0.87, 1.0]),
                    None,
                    &bar,
                );
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    vello::peniko::Color::new([0.20392157, 0.53333336, 0.94509804, 1.0]),
                    None,
                    &fill,
                );

                let value_label_rect = vello::kurbo::Rect::new(
                    ctx.bounds.x0,
                    bar.rect().y1 + PROGRESS_LINEAR_VALUE_LABEL_TOP_SPACING,
                    ctx.bounds.x1,
                    ctx.bounds.y1,
                );
                if value_label_rect.height() > 0.0 {
                    Self::dispatch_in_rect_without_accessibility(
                        ctx,
                        env,
                        progress.value_label,
                        value_label_rect,
                    );
                }
            }
            ProgressStyle::Circular => {
                let center = vello::kurbo::Point::new(
                    ctx.bounds.x0 + ctx.bounds.width() / 2.0,
                    ctx.bounds.y0 + ctx.bounds.height() / 2.0,
                );
                let radius = (ctx.bounds.width().min(ctx.bounds.height()) / 2.0 - 6.0).max(2.0);
                let track = vello::kurbo::Circle::new(center, radius);
                let arc =
                    circle_arc_path(center, radius, -core::f64::consts::FRAC_PI_2, TAU * clamped);
                let scene = unsafe { ctx.scene() };
                let stroke = vello::kurbo::Stroke::new(5.0);
                scene.stroke(
                    &stroke,
                    ctx.transform,
                    vello::peniko::Color::new([0.84, 0.84, 0.87, 1.0]),
                    None,
                    &track,
                );
                scene.stroke(
                    &stroke,
                    ctx.transform,
                    vello::peniko::Color::new([0.20392157, 0.53333336, 0.94509804, 1.0]),
                    None,
                    &arc,
                );
                let label_rect = vello::kurbo::Rect::new(
                    ctx.bounds.x0,
                    ctx.bounds.y1 + 4.0,
                    ctx.bounds.x1,
                    ctx.bounds.y1,
                );
                let _ = label_rect;
            }
            _ => {
                panic!("hydrolysis ProgressStyle variant is not implemented");
            }
        }
    }

    fn render_text_field(
        state: &mut HydroState,
        ctx: RenderContext,
        text_field: Native<ResolvedTextFieldConfig>,
        env: &Environment,
    ) {
        let mut text_field = text_field.into_inner();
        text_field.label = normalize_layout_view(text_field.label, env);
        let line_limit = text_field.line_limit.map(NonZeroUsize::get);
        #[cfg(feature = "accessibility")]
        {
            let prompt = {
                let renderer = unsafe { ctx.renderer() };
                renderer
                    .read_signal(&text_field.prompt.content)
                    .to_plain()
                    .to_string()
            };
            let value = {
                let renderer = unsafe { ctx.renderer() };
                renderer
                    .read_signal(&text_field.value)
                    .to_plain()
                    .to_string()
            };
            let default_label = {
                let renderer = unsafe { ctx.renderer() };
                renderer
                    .accessibility_label_from_view(&text_field.label, env)
                    .or_else(|| (!prompt.is_empty()).then_some(prompt.clone()))
            };
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            let renderer = unsafe { ctx.renderer() };
            let mut node = AccessibilityNode::new(renderer.resolve_accessibility_role(
                env,
                if line_limit == Some(1) {
                    AccessibilityNodeRole::TextInput
                } else {
                    AccessibilityNodeRole::MultilineTextInput
                },
            ));
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            if !prompt.is_empty() {
                node.set_placeholder(prompt);
            }
            if !value.is_empty() {
                node.set_value(value);
            }
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            node.add_action(AccessibilityAction::SetValue);
            if let Some(node_id) = renderer.register_accessibility_node(
                node,
                bounds,
                env,
                Some(AccessibilityActionTarget::TextField {
                    value: text_field.value.clone(),
                    line_limit,
                }),
            ) {
                renderer.push_pending_text_input_accessibility_node(node_id);
            }
        }
        let label_size = measure_view_intrinsic(&text_field.label, state, env);
        let label_height = if label_size.width > 0.0 || label_size.height > 0.0 {
            f64::from(label_size.height).max(INPUT_LABEL_HEIGHT)
        } else {
            0.0
        };
        if label_height > 0.0 {
            let label_rect = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
            );
            Self::dispatch_in_rect_without_accessibility(ctx, env, text_field.label, label_rect);
        }

        let field_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0 + label_height,
            ctx.bounds.x1,
            ctx.bounds.y1,
        );
        let scene = unsafe { ctx.scene() };
        draw_input_field(scene, ctx.transform, field_rect);

        let prompt_signal = text_field.prompt.content.clone();
        let selection_slot = {
            let renderer = unsafe { ctx.renderer() };
            renderer.bind_text_selection_slot()
        };
        let value_binding = text_field.value;
        let input_model = TextInputModel::TextField {
            value: value_binding.clone(),
            line_limit,
            selection_menu: text_field.selection_menu,
        };
        let (
            text_input_index,
            prompt,
            value,
            preedit,
            caret_opacity,
            is_focused,
            selection_visible,
        ) = {
            let renderer = unsafe { ctx.renderer() };
            let text_input_index = renderer.text_input_targets.len();
            let is_focused = renderer.focused_text_input.get() == Some(text_input_index);
            let selection_visible =
                is_focused || renderer.active_text_context_menu_target() == Some(text_input_index);
            let preedit = if is_focused {
                renderer.ime_preedit.clone().unwrap_or_default()
            } else {
                Str::new()
            };
            let caret_opacity = if is_focused {
                renderer.text_caret_opacity(Instant::now())
            } else {
                0.0
            };
            (
                text_input_index,
                renderer.read_signal(&prompt_signal).to_plain(),
                renderer.read_signal(&value_binding).to_plain(),
                preedit,
                caret_opacity,
                is_focused,
                selection_visible,
            )
        };
        let _ = text_input_index;
        let committed_with_preedit = value.clone() + preedit.as_str();
        let use_placeholder = committed_with_preedit.is_empty();
        let display = if use_placeholder {
            prompt
        } else {
            committed_with_preedit.clone()
        };
        let display_styled = if use_placeholder {
            StyledStr::plain(display).foreground(input_placeholder_color())
        } else {
            StyledStr::plain(display)
        };
        let text_bounds = inset_rect(
            field_rect,
            INPUT_FIELD_HORIZONTAL_INSET,
            INPUT_FIELD_VERTICAL_INSET,
        );
        let committed_layout = HydrolysisRenderer::build_text_layout(
            state,
            StyledStr::plain(value.clone()),
            HorizontalAlignment::Leading,
            env,
            Some(text_bounds.width() as f32),
        );
        let selection = {
            let mut slot = selection_slot.borrow_mut();
            if !slot.initialized {
                slot.anchor = value.len();
                slot.focus = value.len();
                slot.initialized = true;
            }
            slot.anchor = clamp_to_char_boundary(value.as_str(), slot.anchor);
            slot.focus = clamp_to_char_boundary(value.as_str(), slot.focus);
            let anchor_layout = input_model.layout_index_from_plain_index(slot.anchor);
            let focus_layout = input_model.layout_index_from_plain_index(slot.focus);
            let anchor_affinity = if anchor_layout >= value.len() {
                parley::Affinity::Upstream
            } else {
                parley::Affinity::Downstream
            };
            let focus_affinity = if focus_layout >= value.len() {
                parley::Affinity::Upstream
            } else {
                parley::Affinity::Downstream
            };
            let selection = parley::Selection::new(
                parley::Cursor::from_byte_index(&committed_layout, anchor_layout, anchor_affinity),
                parley::Cursor::from_byte_index(&committed_layout, focus_layout, focus_affinity),
            )
            .refresh(&committed_layout);
            slot.anchor = input_model.plain_index_from_layout_index(selection.anchor().index());
            slot.focus = input_model.plain_index_from_layout_index(selection.focus().index());
            selection
        };
        {
            let scene = unsafe { ctx.scene() };
            scene.push_layer(
                vello::peniko::Fill::NonZero,
                vello::peniko::BlendMode::default(),
                1.0,
                ctx.transform,
                &text_bounds,
            );
            if selection_visible && !selection.is_collapsed() {
                for (rect, _) in selection.geometry(&committed_layout) {
                    let highlight = vello::kurbo::Rect::new(
                        text_bounds.x0 + rect.x0,
                        text_bounds.y0 + rect.y0,
                        text_bounds.x0 + rect.x1,
                        text_bounds.y0 + rect.y1,
                    );
                    scene.fill(
                        vello::peniko::Fill::NonZero,
                        ctx.transform,
                        vello::peniko::Color::new(TEXT_SELECTION_FILL_COLOR),
                        None,
                        &highlight,
                    );
                }
            }
            Self::render_styled_text_limited(
                state,
                ctx.child(
                    vello::kurbo::Affine::translate((text_bounds.x0, text_bounds.y0)),
                    vello::kurbo::Rect::new(0.0, 0.0, text_bounds.width(), text_bounds.height()),
                ),
                display_styled,
                HorizontalAlignment::Leading,
                env,
                line_limit,
            );
            scene.pop_layer();
        }
        let cursor_area = {
            let rect = selection.focus().geometry(&committed_layout, 1.0);
            let x0 = text_bounds.x0 + rect.x0;
            let y0 = text_bounds.y0 + rect.y0;
            let x1 = text_bounds.x0 + rect.x1.max(rect.x0 + 1.0);
            let y1 = text_bounds.y0 + rect.y1.max(rect.y0 + 1.0);
            vello::kurbo::Rect::new(x0, y0, x1, y1)
        };
        if is_focused && selection.is_collapsed() && caret_opacity > 0.0 {
            let scene = unsafe { ctx.scene() };
            scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                vello::peniko::Color::new([0.12, 0.14, 0.18, caret_opacity]),
                None,
                &cursor_area,
            );
        }

        let renderer = unsafe { ctx.renderer() };
        renderer.register_cursor_target(
            transformed_rect(ctx.hit_transform, field_rect),
            CursorStyle::IBeam,
        );
        tracing::trace!(
            target: "waterui::hydrolysis::hit_region",
            component = "text_field",
            layout_bounds = ?ctx.bounds,
            field_bounds = ?transformed_rect(ctx.hit_transform, field_rect),
            cursor_area = ?transformed_rect(ctx.hit_transform, cursor_area),
            "register text field input region"
        );
        renderer.register_text_input_target(
            transformed_rect(ctx.hit_transform, field_rect),
            transformed_rect(ctx.hit_transform, cursor_area),
            transformed_rect(ctx.hit_transform, text_bounds),
            committed_layout,
            TextInputPurpose::Normal,
            input_model,
            selection_slot,
        );
    }

    fn render_secure_field(
        state: &mut HydroState,
        ctx: RenderContext,
        secure_field: Native<SecureFieldConfig>,
        env: &Environment,
    ) {
        let mut secure_field = secure_field.into_inner();
        secure_field.label = normalize_layout_view(secure_field.label, env);
        #[cfg(feature = "accessibility")]
        {
            let default_label = {
                let renderer = unsafe { ctx.renderer() };
                renderer.accessibility_label_from_view(&secure_field.label, env)
            };
            let secure_len = {
                let renderer = unsafe { ctx.renderer() };
                renderer
                    .read_signal(&secure_field.value)
                    .expose()
                    .chars()
                    .count()
            };
            let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
            let renderer = unsafe { ctx.renderer() };
            let mut node = AccessibilityNode::new(
                renderer.resolve_accessibility_role(env, AccessibilityNodeRole::PasswordInput),
            );
            let label = renderer.resolve_accessibility_label(env, default_label);
            if let Some(label) = label {
                node.set_label(label);
            }
            node.set_value("*".repeat(secure_len));
            node.add_action(AccessibilityAction::Focus);
            node.add_action(AccessibilityAction::Click);
            node.add_action(AccessibilityAction::SetValue);
            if let Some(node_id) = renderer.register_accessibility_node(
                node,
                bounds,
                env,
                Some(AccessibilityActionTarget::SecureField {
                    value: secure_field.value.clone(),
                }),
            ) {
                renderer.push_pending_text_input_accessibility_node(node_id);
            }
        }
        let label_size = measure_view_intrinsic(&secure_field.label, state, env);
        let label_height = if label_size.width > 0.0 || label_size.height > 0.0 {
            f64::from(label_size.height).max(INPUT_LABEL_HEIGHT)
        } else {
            0.0
        };
        if label_height > 0.0 {
            let label_rect = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                (ctx.bounds.y0 + label_height).min(ctx.bounds.y1),
            );
            Self::dispatch_in_rect_without_accessibility(ctx, env, secure_field.label, label_rect);
        }

        let field_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0,
            ctx.bounds.y0 + label_height,
            ctx.bounds.x1,
            ctx.bounds.y1,
        );
        let scene = unsafe { ctx.scene() };
        draw_input_field(scene, ctx.transform, field_rect);

        let selection_slot = {
            let renderer = unsafe { ctx.renderer() };
            renderer.bind_text_selection_slot()
        };
        let value_binding = secure_field.value;
        let input_model = TextInputModel::SecureField {
            value: value_binding.clone(),
        };
        let (text_input_index, masked, caret_opacity, is_focused, selection_visible, plain_value) = {
            let renderer = unsafe { ctx.renderer() };
            let text_input_index = renderer.text_input_targets.len();
            let is_focused = renderer.focused_text_input.get() == Some(text_input_index);
            let selection_visible =
                is_focused || renderer.active_text_context_menu_target() == Some(text_input_index);
            let plain_value = renderer.read_signal(&value_binding).expose().to_owned();
            let preedit_count = if is_focused {
                renderer
                    .ime_preedit
                    .as_ref()
                    .map_or(0, |value| value.chars().count())
            } else {
                0
            };
            let count = plain_value.chars().count() + preedit_count;
            let caret_opacity = if is_focused {
                renderer.text_caret_opacity(Instant::now())
            } else {
                0.0
            };
            (
                text_input_index,
                "*".repeat(count),
                caret_opacity,
                is_focused,
                selection_visible,
                plain_value,
            )
        };
        let _ = text_input_index;
        let text_bounds = inset_rect(
            field_rect,
            INPUT_FIELD_HORIZONTAL_INSET,
            INPUT_FIELD_VERTICAL_INSET,
        );
        let masked_display = StyledStr::plain(masked.clone());
        let committed_layout = HydrolysisRenderer::build_text_layout(
            state,
            StyledStr::plain(masked),
            HorizontalAlignment::Leading,
            env,
            Some(text_bounds.width() as f32),
        );
        let selection = {
            let mut slot = selection_slot.borrow_mut();
            if !slot.initialized {
                slot.anchor = plain_value.len();
                slot.focus = plain_value.len();
                slot.initialized = true;
            }
            slot.anchor = clamp_to_char_boundary(plain_value.as_str(), slot.anchor);
            slot.focus = clamp_to_char_boundary(plain_value.as_str(), slot.focus);
            let text_len = plain_value.chars().count();
            let anchor_layout = input_model.layout_index_from_plain_index(slot.anchor);
            let focus_layout = input_model.layout_index_from_plain_index(slot.focus);
            let anchor_affinity = if anchor_layout >= text_len {
                parley::Affinity::Upstream
            } else {
                parley::Affinity::Downstream
            };
            let focus_affinity = if focus_layout >= text_len {
                parley::Affinity::Upstream
            } else {
                parley::Affinity::Downstream
            };
            let selection = parley::Selection::new(
                parley::Cursor::from_byte_index(&committed_layout, anchor_layout, anchor_affinity),
                parley::Cursor::from_byte_index(&committed_layout, focus_layout, focus_affinity),
            )
            .refresh(&committed_layout);
            slot.anchor = input_model.plain_index_from_layout_index(selection.anchor().index());
            slot.focus = input_model.plain_index_from_layout_index(selection.focus().index());
            selection
        };
        {
            let scene = unsafe { ctx.scene() };
            scene.push_layer(
                vello::peniko::Fill::NonZero,
                vello::peniko::BlendMode::default(),
                1.0,
                ctx.transform,
                &text_bounds,
            );
            if selection_visible && !selection.is_collapsed() {
                for (rect, _) in selection.geometry(&committed_layout) {
                    let highlight = vello::kurbo::Rect::new(
                        text_bounds.x0 + rect.x0,
                        text_bounds.y0 + rect.y0,
                        text_bounds.x0 + rect.x1,
                        text_bounds.y0 + rect.y1,
                    );
                    scene.fill(
                        vello::peniko::Fill::NonZero,
                        ctx.transform,
                        vello::peniko::Color::new(TEXT_SELECTION_FILL_COLOR),
                        None,
                        &highlight,
                    );
                }
            }
            Self::render_styled_text_limited(
                state,
                ctx.child(
                    vello::kurbo::Affine::translate((text_bounds.x0, text_bounds.y0)),
                    vello::kurbo::Rect::new(0.0, 0.0, text_bounds.width(), text_bounds.height()),
                ),
                masked_display,
                HorizontalAlignment::Leading,
                env,
                Some(1),
            );
            scene.pop_layer();
        }
        let cursor_area = {
            let rect = selection.focus().geometry(&committed_layout, 1.0);
            let x0 = text_bounds.x0 + rect.x0;
            let y0 = text_bounds.y0 + rect.y0;
            let x1 = text_bounds.x0 + rect.x1.max(rect.x0 + 1.0);
            let y1 = text_bounds.y0 + rect.y1.max(rect.y0 + 1.0);
            vello::kurbo::Rect::new(x0, y0, x1, y1)
        };
        if is_focused && selection.is_collapsed() && caret_opacity > 0.0 {
            let scene = unsafe { ctx.scene() };
            scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                vello::peniko::Color::new([0.12, 0.14, 0.18, caret_opacity]),
                None,
                &cursor_area,
            );
        }

        let renderer = unsafe { ctx.renderer() };
        renderer.register_cursor_target(
            transformed_rect(ctx.hit_transform, field_rect),
            CursorStyle::IBeam,
        );
        tracing::trace!(
            target: "waterui::hydrolysis::hit_region",
            component = "secure_field",
            layout_bounds = ?ctx.bounds,
            field_bounds = ?transformed_rect(ctx.hit_transform, field_rect),
            cursor_area = ?transformed_rect(ctx.hit_transform, cursor_area),
            "register secure field input region"
        );
        renderer.register_text_input_target(
            transformed_rect(ctx.hit_transform, field_rect),
            transformed_rect(ctx.hit_transform, cursor_area),
            transformed_rect(ctx.hit_transform, text_bounds),
            committed_layout,
            TextInputPurpose::Password,
            input_model,
            selection_slot,
        );
    }

    fn render_picker(
        state: &mut HydroState,
        ctx: RenderContext,
        picker: Native<PickerConfig>,
        env: &Environment,
    ) {
        let picker = picker.into_inner();
        let items = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&picker.items)
        };
        assert!(
            !(items.is_empty()),
            "hydrolysis picker requires at least one item"
        );
        match picker.style {
            PickerStyle::Automatic | PickerStyle::Menu => {
                Self::render_menu_picker(state, ctx, picker.selection, items, env);
            }
            PickerStyle::Radio => {
                Self::render_radio_picker(state, ctx, picker.selection, items, env);
            }
            _ => panic!("hydrolysis PickerStyle variant is not implemented"),
        }
    }

    fn render_menu_picker(
        state: &mut HydroState,
        ctx: RenderContext,
        selection: nami::Binding<waterui_core::id::Id>,
        items: Vec<waterui_form::picker::PickerItem<waterui_core::id::Id>>,
        env: &Environment,
    ) {
        let selected = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&selection)
        };
        let menu_open = {
            let renderer = unsafe { ctx.renderer() };
            renderer.bind_picker_menu_state()
        };
        let is_menu_open = menu_open.get();
        let selected_index = items
            .iter()
            .position(|item| item.tag == selected)
            .unwrap_or_else(|| {
                panic!("hydrolysis picker selection is not present in picker items")
            });
        let mut option_texts = Vec::with_capacity(items.len());
        let mut max_item_text_height: f64 = 0.0;
        for item in &items {
            let styled = {
                let renderer = unsafe { ctx.renderer() };
                let label_signal = item.content.content();
                renderer.read_signal(&label_signal)
            };
            let plain = styled.to_plain();
            let size = HydrolysisRenderer::measure_text_intrinsic_size(
                state,
                StyledStr::plain(plain.clone()),
                env,
            );
            max_item_text_height = max_item_text_height.max(f64::from(size.height));
            option_texts.push(plain);
        }
        let selected_text = option_texts[selected_index].clone();

        let scene = unsafe { ctx.scene() };
        draw_input_field(scene, ctx.transform, ctx.bounds);
        draw_picker_indicator(scene, ctx.transform, ctx.bounds);

        let text_bounds = inset_rect(ctx.bounds, PICKER_HORIZONTAL_INSET, PICKER_VERTICAL_INSET);
        let text_bounds = vello::kurbo::Rect::new(
            text_bounds.x0,
            text_bounds.y0,
            (text_bounds.x1 - PICKER_INDICATOR_SPACE).max(text_bounds.x0),
            text_bounds.y1,
        );
        Self::render_styled_text(
            state,
            ctx.child(
                vello::kurbo::Affine::translate((text_bounds.x0, text_bounds.y0)),
                vello::kurbo::Rect::new(0.0, 0.0, text_bounds.width(), text_bounds.height()),
            ),
            StyledStr::plain(selected_text),
            HorizontalAlignment::Leading,
            env,
        );

        let renderer = unsafe { ctx.renderer() };
        let field_open_state = Rc::clone(&menu_open);
        renderer.register_pointer_target(
            transformed_rect(ctx.hit_transform, ctx.bounds),
            move |_point, _env| {
                field_open_state.set(!field_open_state.get());
                true
            },
        );

        if !is_menu_open {
            return;
        }

        let row_height = menu_picker_row_height(ctx.bounds, max_item_text_height);
        let popup_rect = menu_picker_popup_rect(ctx.bounds, row_height, items.len());
        let scene = unsafe { ctx.scene() };
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            vello::peniko::Color::new([0.98, 0.98, 0.99, 1.0]),
            None,
            &vello::kurbo::RoundedRect::from_rect(popup_rect, PICKER_MENU_POPUP_CORNER_RADIUS),
        );
        scene.stroke(
            &vello::kurbo::Stroke::new(1.0),
            ctx.transform,
            vello::peniko::Color::new([0.8, 0.82, 0.86, 1.0]),
            None,
            &vello::kurbo::RoundedRect::from_rect(popup_rect, PICKER_MENU_POPUP_CORNER_RADIUS),
        );

        for (index, item) in items.into_iter().enumerate() {
            let row_rect = menu_picker_option_rect(popup_rect, row_height, index);
            if item.tag == selected {
                let scene = unsafe { ctx.scene() };
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    vello::peniko::Color::new([0.88, 0.92, 0.99, 1.0]),
                    None,
                    &inset_rect(row_rect, 2.0, 1.0),
                );
            }
            if index + 1 < option_texts.len() {
                let separator = vello::kurbo::Rect::new(
                    row_rect.x0 + 6.0,
                    row_rect.y1 - 1.0,
                    row_rect.x1 - 6.0,
                    row_rect.y1,
                );
                let scene = unsafe { ctx.scene() };
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    vello::peniko::Color::new([0.9, 0.91, 0.94, 1.0]),
                    None,
                    &separator,
                );
            }
            let row_text_rect =
                inset_rect(row_rect, PICKER_HORIZONTAL_INSET, PICKER_VERTICAL_INSET);
            Self::render_styled_text(
                state,
                ctx.child(
                    vello::kurbo::Affine::translate((row_text_rect.x0, row_text_rect.y0)),
                    vello::kurbo::Rect::new(
                        0.0,
                        0.0,
                        row_text_rect.width(),
                        row_text_rect.height(),
                    ),
                ),
                StyledStr::plain(option_texts[index].clone()),
                HorizontalAlignment::Leading,
                env,
            );

            let open_state = Rc::clone(&menu_open);
            let selection = selection.clone();
            let tag = item.tag;
            let renderer = unsafe { ctx.renderer() };
            renderer.register_pointer_target(
                transformed_rect(ctx.hit_transform, row_rect),
                move |_point, _env| {
                    if selection.get() != tag {
                        selection.set(tag);
                    }
                    open_state.set(false);
                    true
                },
            );
        }
    }

    fn render_radio_picker(
        state: &mut HydroState,
        ctx: RenderContext,
        selection: nami::Binding<waterui_core::id::Id>,
        items: Vec<waterui_form::picker::PickerItem<waterui_core::id::Id>>,
        env: &Environment,
    ) {
        let selected = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&selection)
        };
        let mut row_y = ctx.bounds.y0 + PICKER_VERTICAL_INSET;
        for item in items {
            let label_signal = item.content.content();
            let label = {
                let renderer = unsafe { ctx.renderer() };
                renderer.read_signal(&label_signal)
            };
            let label_size =
                HydrolysisRenderer::measure_text_intrinsic_size(state, label.clone(), env);
            let row_height = f64::from(label_size.height).max(PICKER_RADIO_INDICATOR_SIZE);
            let row_rect = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                row_y,
                ctx.bounds.x1,
                (row_y + row_height).min(ctx.bounds.y1),
            );
            if row_rect.height() <= 0.0 {
                break;
            }
            row_y = row_rect.y1 + PICKER_RADIO_ROW_SPACING;

            let indicator_center = vello::kurbo::Point::new(
                row_rect.x0 + PICKER_HORIZONTAL_INSET + PICKER_RADIO_INDICATOR_SIZE / 2.0,
                row_rect.y0 + row_rect.height() / 2.0,
            );
            let indicator_radius = PICKER_RADIO_INDICATOR_SIZE / 2.0;
            let indicator = vello::kurbo::Circle::new(indicator_center, indicator_radius);
            let is_selected = item.tag == selected;
            let scene = unsafe { ctx.scene() };
            scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                vello::peniko::Color::new([1.0, 1.0, 1.0, 1.0]),
                None,
                &indicator,
            );
            scene.stroke(
                &vello::kurbo::Stroke::new(1.0),
                ctx.transform,
                if is_selected {
                    vello::peniko::Color::new([0.2, 0.45, 0.9, 1.0])
                } else {
                    vello::peniko::Color::new([0.65, 0.67, 0.72, 1.0])
                },
                None,
                &indicator,
            );
            if is_selected {
                let inner = vello::kurbo::Circle::new(indicator_center, indicator_radius * 0.45);
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    ctx.transform,
                    vello::peniko::Color::new([0.2, 0.45, 0.9, 1.0]),
                    None,
                    &inner,
                );
            }

            let label_rect = vello::kurbo::Rect::new(
                indicator_center.x + indicator_radius + PICKER_RADIO_LABEL_SPACING,
                row_rect.y0,
                row_rect.x1 - PICKER_HORIZONTAL_INSET,
                row_rect.y1,
            );
            Self::render_styled_text(
                state,
                ctx.child(
                    vello::kurbo::Affine::translate((label_rect.x0, label_rect.y0)),
                    vello::kurbo::Rect::new(0.0, 0.0, label_rect.width(), label_rect.height()),
                ),
                label,
                HorizontalAlignment::Leading,
                env,
            );

            let tag = item.tag;
            let renderer = unsafe { ctx.renderer() };
            renderer.register_pointer_target(transformed_rect(ctx.hit_transform, row_rect), {
                let selection = selection.clone();
                move |_point, _env| {
                    if selection.get() == tag {
                        return false;
                    }
                    selection.set(tag);
                    true
                }
            });
        }
    }

    fn render_dynamic(
        state: &mut HydroState,
        ctx: RenderContext,
        dynamic: Native<Dynamic>,
        env: &Environment,
    ) {
        let dynamic = dynamic.into_inner();
        let identity = dynamic.identity();
        {
            let renderer = unsafe { ctx.renderer() };
            renderer.dynamic_identities_current_frame.push(identity);
        }
        let pending_view = {
            let renderer = unsafe { ctx.renderer() };
            if let Some(node) = renderer.dynamic_nodes.get(&identity) {
                Rc::clone(&node.pending_view)
            } else {
                let pending_view = Rc::new(RefCell::new(None::<AnyView>));
                let is_initial = Rc::new(Cell::new(true));
                let rebuild_requested = Rc::clone(&renderer.rebuild_requested);
                dynamic.connect({
                    let pending_view = Rc::clone(&pending_view);
                    let is_initial = Rc::clone(&is_initial);
                    let rebuild_requested = Rc::clone(&rebuild_requested);
                    move |update| {
                        *pending_view.borrow_mut() = Some(update.into_value());
                        if !is_initial.replace(false) {
                            rebuild_requested.set(true);
                        }
                    }
                });
                renderer.dynamic_nodes.insert(
                    identity,
                    DynamicNode {
                        pending_view: Rc::clone(&pending_view),
                        cached_subtree: None,
                    },
                );
                pending_view
            }
        };

        let update = pending_view.borrow_mut().take();
        if let Some(content) = update {
            let content = normalize_layout_view(content, env);
            let dimensions = measure_view_dimensions(&content, state, env);
            state.dynamic_intrinsic_cache.insert(identity, dimensions);
            let local_ctx = RenderContext {
                renderer_ptr: ctx.renderer_ptr,
                transform: vello::kurbo::Affine::IDENTITY,
                hit_transform: vello::kurbo::Affine::IDENTITY,
                bounds: ctx.bounds,
            };
            let subtree = Self::render_dynamic_subtree(local_ctx, env, content);
            let renderer = unsafe { ctx.renderer() };
            renderer
                .dynamic_nodes
                .get_mut(&identity)
                .expect("hydrolysis dynamic node missing after connect")
                .cached_subtree = Some(subtree);
        }

        let renderer_ptr = {
            let renderer = unsafe { ctx.renderer() };
            renderer
                .dynamic_nodes
                .get(&identity)
                .and_then(|node| node.cached_subtree.as_ref())
                .expect("hydrolysis Dynamic must provide an initial view before dispatch")
                as *const DynamicSubtree
        };
        let subtree = unsafe { &*renderer_ptr };
        let renderer = unsafe { ctx.renderer() };
        renderer.replay_dynamic_subtree(ctx, subtree);
    }

    fn render_system_icon(
        state: &mut HydroState,
        ctx: RenderContext,
        icon: Native<SystemIcon>,
        env: &Environment,
    ) {
        let styled = StyledStr::plain(icon.into_inner().name);
        Self::render_styled_text(state, ctx, styled, HorizontalAlignment::Leading, env);
    }

    fn render_gpu_surface(
        _state: &mut HydroState,
        ctx: RenderContext,
        surface: Native<GpuSurface>,
        env: &Environment,
    ) {
        let renderer = unsafe { ctx.renderer() };
        let slot_index = renderer.bind_gpu_surface_slot(surface.into_inner(), env);
        renderer.push_gpu_surface_layer(slot_index, ctx.transform, ctx.bounds);
    }

    fn render_scene_view(
        _state: &mut HydroState,
        ctx: RenderContext,
        scene_view: Native<SceneView>,
        _env: &Environment,
    ) {
        let mut scene_view = scene_view.into_inner();
        let rebuild_handle = {
            let renderer = unsafe { ctx.renderer() };
            renderer.rebuild_handle()
        };
        scene_view
            .content_mut()
            .set_invalidator(Some(Rc::new(move || rebuild_handle.set(true))));

        let scene = unsafe { ctx.scene() };
        let mut scene2d = VelloScene2D::new(scene);
        #[allow(clippy::cast_precision_loss)]
        let needs_next_frame = scene_view.content_mut().build_scene(
            &mut scene2d,
            ctx.bounds.width() as f32,
            ctx.bounds.height() as f32,
        );
        if needs_next_frame {
            let renderer = unsafe { ctx.renderer() };
            renderer.request_rebuild();
        }
    }

    fn render_view_effect(
        _state: &mut HydroState,
        ctx: RenderContext,
        effect: Native<ViewEffectErased>,
        env: &Environment,
    ) {
        let mut effect = effect.into_inner();
        let renderer = unsafe { ctx.renderer() };
        let (device_ptr, queue_ptr) = renderer.state().frame_resource_ptrs();
        let device = unsafe { &*device_ptr };
        let queue = unsafe { &*queue_ptr };

        let input_width = (ctx.bounds.width().max(1.0).round()) as u32;
        let input_height = (ctx.bounds.height().max(1.0).round()) as u32;
        let output_size = effect.output_size();
        let (output_width, output_height) = output_size.compute(input_width, input_height);
        assert!(
            !(output_width == 0 || output_height == 0),
            "hydrolysis ViewEffect requires non-zero output dimensions"
        );

        let subtree = Self::render_subtree_scene(ctx, env, effect.take_content());

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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor::default());
        renderer
            .vello_renderer
            .render_to_texture(
                device,
                queue,
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

        let setup_context = EffectContext {
            device,
            queue,
            input_format: wgpu::TextureFormat::Rgba8Unorm,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            pipeline_cache: None,
        };
        pollster::block_on(effect.setup(&setup_context));

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

        let input = EffectInput {
            device,
            queue,
            texture: &input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: input_width,
            height: input_height,
        };
        let output = EffectOutput {
            device,
            queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format: wgpu::TextureFormat::Rgba8Unorm,
            width: output_width,
            height: output_height,
        };
        effect.render(&input, &output);
        if effect.needs_redraw() {
            renderer.request_rebuild();
        }

        let image = renderer.vello_renderer.register_texture(output_texture);
        renderer.active_filter_images.push(image.clone());
        let image_transform = vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0))
            * vello::kurbo::Affine::scale_non_uniform(
                ctx.bounds.width() / f64::from(output_width),
                ctx.bounds.height() / f64::from(output_height),
            );
        let scene = unsafe { ctx.scene() };
        scene.draw_image(
            &vello::peniko::ImageBrush::new(image),
            ctx.transform * image_transform,
        );
    }

    fn render_resolved_color(
        _state: &mut HydroState,
        ctx: RenderContext,
        color: Native<ResolvedColor>,
        _env: &Environment,
    ) {
        let scene = unsafe { ctx.scene() };
        let brush = resolved_color_to_peniko(color.into_inner());
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            brush,
            None,
            &ctx.bounds,
        );
    }

    fn render_resolved_gradient(
        _state: &mut HydroState,
        ctx: RenderContext,
        gradient: Native<ResolvedGradient>,
        _env: &Environment,
    ) {
        let scene = unsafe { ctx.scene() };
        let brush = resolved_gradient_to_brush(&gradient.into_inner(), ctx.bounds);
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            &brush,
            None,
            &ctx.bounds,
        );
    }

    fn render_resolved_shape(
        _state: &mut HydroState,
        ctx: RenderContext,
        shape: Native<ResolvedShape>,
        _env: &Environment,
    ) {
        let resolved = shape.into_inner();
        let path = resolved_shape_to_path(&resolved, ctx.bounds);
        let fill = resolved_color_to_peniko(resolved.fill);
        let scene = unsafe { ctx.scene() };
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            fill,
            None,
            &path,
        );
    }

    fn render_environment_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Environment>,
        _env: &Environment,
    ) {
        let renderer = unsafe { ctx.renderer() };
        renderer.dispatch_with_render_depth(metadata.content, &metadata.value, ctx);
    }

    fn render_retain_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Retain>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let renderer = unsafe { ctx.renderer() };
        renderer.current_frame_retain.push(value);
        renderer.dispatch_with_render_depth(content, env, ctx);
    }

    fn render_opacity_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Opacity>,
        env: &Environment,
    ) {
        let alpha = {
            let renderer = unsafe { ctx.renderer() };
            renderer.resolve_animated_scalar(&metadata.value.value)
        };
        {
            let renderer = unsafe { ctx.renderer() };
            renderer.push_layer_rect(alpha, ctx.transform, ctx.bounds);
        }

        {
            let renderer = unsafe { ctx.renderer() };
            let previous_opacity = renderer.hit_test_opacity;
            renderer.hit_test_opacity = previous_opacity * alpha;
            renderer.dispatch_with_render_depth(metadata.content, env, ctx);
            renderer.hit_test_opacity = previous_opacity;
        }

        {
            let renderer = unsafe { ctx.renderer() };
            renderer.pop_layer();
        }
    }

    fn render_applied_filter_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<AppliedFilter>,
        env: &Environment,
    ) {
        let Metadata {
            content,
            value: mut filter,
        } = metadata;
        let renderer = unsafe { ctx.renderer() };
        let (device_ptr, queue_ptr) = renderer.state().frame_resource_ptrs();
        let device = unsafe { &*device_ptr };
        let queue = unsafe { &*queue_ptr };

        let width = (ctx.bounds.width().max(1.0).round()) as u32;
        let height = (ctx.bounds.height().max(1.0).round()) as u32;
        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let mut subtree_scene = vello::Scene::new();
        core::mem::swap(&mut renderer.scene, &mut subtree_scene);
        renderer.dispatch_with_render_depth(content, env, ctx);
        core::mem::swap(&mut renderer.scene, &mut subtree_scene);

        let input_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_applied_filter_input"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let input_view = input_texture.create_view(&wgpu::TextureViewDescriptor::default());
        renderer
            .vello_renderer
            .render_to_texture(
                device,
                queue,
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

        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_applied_filter_output"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let filter_context = FilterContext {
            device,
            queue,
            input_format: wgpu::TextureFormat::Rgba8Unorm,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            pipeline_cache: None,
        };
        pollster::block_on(filter.setup(&filter_context));
        filter.sync_targets();

        let input = FilterInput {
            device,
            queue,
            texture: &input_texture,
            view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format: wgpu::TextureFormat::Rgba8Unorm,
            width,
            height,
        };
        let output = FilterOutput {
            device,
            queue,
            texture: &output_texture,
            view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            format: wgpu::TextureFormat::Rgba8Unorm,
            width,
            height,
        };
        let needs_redraw = match filter.render(&input, &output) {
            Ok(needs_redraw) => needs_redraw || filter.redraw_hint(),
            Err(err) => {
                panic!("hydrolysis filter render failed: {err}");
            }
        };
        if needs_redraw {
            renderer.request_rebuild();
        }

        let image = renderer.vello_renderer.register_texture(output_texture);
        renderer.active_filter_images.push(image.clone());
        let image_transform = vello::kurbo::Affine::translate((ctx.bounds.x0, ctx.bounds.y0))
            * vello::kurbo::Affine::scale_non_uniform(
                ctx.bounds.width() / f64::from(width),
                ctx.bounds.height() / f64::from(height),
            );
        let scene = unsafe { ctx.scene() };
        scene.draw_image(
            &vello::peniko::ImageBrush::new(image),
            ctx.transform * image_transform,
        );
    }

    fn render_scale_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Scale>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let center = anchor_point(ctx.bounds, value.anchor);
        let (scale_x, scale_y) = {
            let renderer = unsafe { ctx.renderer() };
            (
                renderer.resolve_animated_scalar(&value.x),
                renderer.resolve_animated_scalar(&value.y),
            )
        };
        let transform = vello::kurbo::Affine::translate((center.x, center.y))
            * vello::kurbo::Affine::scale_non_uniform(f64::from(scale_x), f64::from(scale_y))
            * vello::kurbo::Affine::translate((-center.x, -center.y));
        Self::dispatch_any(ctx.child(transform, ctx.bounds), env, content);
    }

    fn render_rotation_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Rotation>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let center = anchor_point(ctx.bounds, value.anchor);
        let radians = {
            let renderer = unsafe { ctx.renderer() };
            f64::from(renderer.resolve_animated_scalar(&value.angle)).to_radians()
        };
        let transform = vello::kurbo::Affine::translate((center.x, center.y))
            * vello::kurbo::Affine::rotate(radians)
            * vello::kurbo::Affine::translate((-center.x, -center.y));
        Self::dispatch_any(ctx.child(transform, ctx.bounds), env, content);
    }

    fn render_offset_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Offset>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let (offset_x, offset_y) = {
            let renderer = unsafe { ctx.renderer() };
            (
                renderer.resolve_animated_scalar(&value.x),
                renderer.resolve_animated_scalar(&value.y),
            )
        };
        let transform = vello::kurbo::Affine::translate((f64::from(offset_x), f64::from(offset_y)));
        Self::dispatch_any(ctx.child(transform, ctx.bounds), env, content);
    }

    fn render_clip_shape_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<ClipShape>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let clip_path = path_commands_to_path(value.commands(), ctx.bounds);
        {
            let renderer = unsafe { ctx.renderer() };
            renderer.push_layer_path(1.0, ctx.transform, clip_path);
        }
        Self::dispatch_any(ctx, env, content);
        {
            let renderer = unsafe { ctx.renderer() };
            renderer.pop_layer();
        }
    }

    fn render_border_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Border>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let border = value;
        Self::dispatch_any(ctx, env, content);

        if border.width <= 0.0 {
            return;
        }

        let scene = unsafe { ctx.scene() };
        let brush = resolved_color_to_peniko(border.color.resolve(env).get());
        let width = f64::from(border.width);

        if border.edges.all() && border.corner_radius > 0.0 {
            let rounded =
                vello::kurbo::RoundedRect::from_rect(ctx.bounds, f64::from(border.corner_radius));
            let stroke = vello::kurbo::Stroke::new(width);
            scene.stroke(&stroke, ctx.transform, brush, None, &rounded);
            return;
        }

        if border.edges.top {
            let top = vello::kurbo::Rect::new(
                ctx.bounds.x0,
                ctx.bounds.y0,
                ctx.bounds.x1,
                ctx.bounds.y0 + width,
            );
            scene.fill(
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
            scene.fill(
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
            scene.fill(
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
            scene.fill(
                vello::peniko::Fill::NonZero,
                ctx.transform,
                brush,
                None,
                &trailing,
            );
        }
    }

    fn render_shadow_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Shadow>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let shadow = value;
        let spread = f64::from(shadow.radius.max(0.0));
        let offset_x = f64::from(shadow.offset.x);
        let offset_y = f64::from(shadow.offset.y);
        let shadow_rect = vello::kurbo::Rect::new(
            ctx.bounds.x0 + offset_x - spread,
            ctx.bounds.y0 + offset_y - spread,
            ctx.bounds.x1 + offset_x + spread,
            ctx.bounds.y1 + offset_y + spread,
        );
        let shadow_color = resolved_color_to_peniko(shadow.color.resolve(env).get());

        let scene = unsafe { ctx.scene() };
        scene.fill(
            vello::peniko::Fill::NonZero,
            ctx.transform,
            shadow_color,
            None,
            &shadow_rect,
        );
        Self::dispatch_any(ctx, env, content);
    }

    fn render_focused_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Focused>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let should_focus = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&value.0)
        };
        let renderer = unsafe { ctx.renderer() };
        let start = renderer.text_input_targets.len();
        Self::dispatch_any(ctx, env, content);
        let end = renderer.text_input_targets.len();
        let focus_target_count = end - start;
        assert!(
            focus_target_count == 1,
            "hydrolysis .focused() requires exactly one TextField or SecureField in the wrapped subtree, found {focus_target_count}"
        );
        let target = renderer
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
            renderer.focused_text_input.get(),
            Some(index) if index >= start && index < end
        ) {
            renderer.set_focused_text_input(None);
        }
    }

    fn render_hittable_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Hittable>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let enabled = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&value.enabled)
        };
        let renderer = unsafe { ctx.renderer() };
        let pointer_start = renderer.pointer_targets.len();
        let gesture_start = renderer.gesture_engine.target_count();
        let cursor_start = renderer.cursor_targets.len();
        let hover_start = renderer.hover_targets.len();
        let hover_cursor_start = renderer.hover_controller.cursor();
        let scroll_start = renderer.scroll_targets.len();
        let text_start = renderer.text_input_targets.len();

        Self::dispatch_any(ctx, env, content);

        if enabled {
            return;
        }

        renderer.pointer_targets.truncate(pointer_start);
        renderer.ensure_active_pointer_drag_target_is_live();
        renderer.gesture_engine.truncate_targets(gesture_start);
        renderer.cursor_targets.truncate(cursor_start);
        renderer.hover_targets.truncate(hover_start);
        renderer.hover_controller.rewind_to(hover_cursor_start);
        renderer.scroll_targets.truncate(scroll_start);
        let text_end = renderer.text_input_targets.len();
        renderer.text_input_targets.truncate(text_start);

        if matches!(
            renderer.focused_text_input.get(),
            Some(index) if index >= text_start && index < text_end
        ) {
            renderer.set_focused_text_input(None);
        }
    }

    fn render_cursor_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<Cursor>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let style = {
            let renderer = unsafe { ctx.renderer() };
            renderer.read_signal(&value.style)
        };
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let renderer = unsafe { ctx.renderer() };
        renderer.register_cursor_target(bounds, style);
        Self::dispatch_any(ctx, env, content);
    }

    fn render_gesture_observer_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<GestureObserver>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let GestureObserver {
            gesture, action, ..
        } = value;
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let gesture_group_identity = gesture_group_identity(&content);
        let renderer = unsafe { ctx.renderer() };
        let group_id = renderer.gesture_group_id_for_identity(gesture_group_identity);
        renderer.register_gesture_target(bounds, group_id, gesture, action);

        Self::dispatch_any(ctx, env, content);
    }

    fn render_lifecycle_hook_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<LifeCycleHook>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        match value.lifecycle() {
            LifeCycle::Appear => value.handle(env),
            LifeCycle::Disappear => {
                let renderer = unsafe { ctx.renderer() };
                let slot = renderer.lifecycle_disappear_slot;
                renderer.lifecycle_disappear_slot += 1;
                renderer
                    .lifecycle_disappear_current
                    .insert(slot, DeferredLifeCycleHook::new(value, env.clone()));
            }
            _ => panic!("hydrolysis lifecycle variant is not supported"),
        }
        Self::dispatch_any(ctx, env, content);
    }

    fn render_on_event_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<OnEvent>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let event = value.event();
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let renderer = unsafe { ctx.renderer() };
        match event {
            Event::HoverEnter => {
                let mut handler = value;
                renderer.register_hover_enter_target(bounds, move |env| {
                    handler.handle(env);
                    true
                });
            }
            Event::HoverMove => {
                let mut handler = value;
                renderer.register_hover_move_target(bounds, move |point, env| {
                    let hover_env =
                        env.extending(HoverEvent::new(waterui_core::layout::Point::new(
                            point.x as f32 - bounds.x0 as f32,
                            point.y as f32 - bounds.y0 as f32,
                        )));
                    handler.handle(&hover_env);
                    true
                });
            }
            Event::HoverExit => {
                let mut handler = value;
                renderer.register_hover_exit_target(bounds, move |env| {
                    handler.handle(env);
                    true
                });
            }
            _ => panic!("hydrolysis event variant is not supported"),
        }
        Self::dispatch_any(ctx, env, content);
    }

    fn render_context_menu_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<ResolvedContextMenu>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let bounds = transformed_rect(ctx.hit_transform, ctx.bounds);
        let renderer = unsafe { ctx.renderer() };
        renderer.register_context_menu_target(bounds, value.items);
        Self::dispatch_any(ctx, env, content);
    }

    fn render_passthrough_metadata<T: MetadataKey>(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: Metadata<T>,
        env: &Environment,
    ) {
        let Metadata { content, value } = metadata;
        let _ = value;
        Self::dispatch_any(ctx, env, content);
    }

    fn render_passthrough_ignorable_metadata<T: MetadataKey>(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: IgnorableMetadata<T>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let _ = value;
        Self::dispatch_any(ctx, env, content);
    }

    fn render_accessibility_label_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityLabel>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let mut local_env = env.clone();
        local_env.insert(value);
        Self::dispatch_any(ctx, &local_env, content);
    }

    fn render_accessibility_role_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityRole>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let mut local_env = env.clone();
        local_env.insert(value);
        Self::dispatch_any(ctx, &local_env, content);
    }

    fn render_accessibility_hidden_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityHidden>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let mut local_env = env.clone();
        local_env.insert(value);
        Self::dispatch_any(ctx, &local_env, content);
    }

    fn render_accessibility_children_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityChildren>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let mut local_env = env.clone();
        local_env.insert(value);
        Self::dispatch_any(ctx, &local_env, content);
    }

    fn render_accessibility_state_metadata(
        _state: &mut HydroState,
        ctx: RenderContext,
        metadata: IgnorableMetadata<AccessibilityState>,
        env: &Environment,
    ) {
        let IgnorableMetadata { content, value } = metadata;
        let mut local_env = env.clone();
        local_env.insert(value);
        Self::dispatch_any(ctx, &local_env, content);
    }

    #[must_use]
    pub fn state(&self) -> &HydroState {
        self.dispatcher.state()
    }

    fn draw_scroll_indicators(
        scene: &mut vello::Scene,
        transform: vello::kurbo::Affine,
        viewport: vello::kurbo::Rect,
        metrics: ScrollMetrics,
        axis: ScrollAxis,
    ) {
        let indicator_color = vello::peniko::Color::new([0.4, 0.4, 0.4, 0.55]);
        match axis {
            ScrollAxis::Vertical | ScrollAxis::All => {
                if metrics.max_y > 0.0 {
                    let track_height = viewport.height();
                    let min_thumb_height = track_height.min(12.0);
                    let thumb_height = (track_height
                        * (metrics.viewport_height / metrics.content_height))
                        .clamp(min_thumb_height, track_height);
                    let travel = track_height - thumb_height;
                    let progress = if metrics.max_y > 0.0 {
                        metrics.offset_y / metrics.max_y
                    } else {
                        0.0
                    };
                    let thumb_y = viewport.y0 + travel * progress;
                    let thumb = vello::kurbo::RoundedRect::from_rect(
                        vello::kurbo::Rect::new(
                            viewport.x1 - 4.0,
                            thumb_y,
                            viewport.x1 - 1.5,
                            thumb_y + thumb_height,
                        ),
                        1.25,
                    );
                    scene.fill(
                        vello::peniko::Fill::NonZero,
                        transform,
                        indicator_color,
                        None,
                        &thumb,
                    );
                }
            }
            _ => {}
        }

        match axis {
            ScrollAxis::Horizontal | ScrollAxis::All => {
                if metrics.max_x > 0.0 {
                    let track_width = viewport.width();
                    let min_thumb_width = track_width.min(12.0);
                    let thumb_width = (track_width
                        * (metrics.viewport_width / metrics.content_width))
                        .clamp(min_thumb_width, track_width);
                    let travel = track_width - thumb_width;
                    let progress = if metrics.max_x > 0.0 {
                        metrics.offset_x / metrics.max_x
                    } else {
                        0.0
                    };
                    let thumb_x = viewport.x0 + travel * progress;
                    let thumb = vello::kurbo::RoundedRect::from_rect(
                        vello::kurbo::Rect::new(
                            thumb_x,
                            viewport.y1 - 4.0,
                            thumb_x + thumb_width,
                            viewport.y1 - 1.5,
                        ),
                        1.25,
                    );
                    scene.fill(
                        vello::peniko::Fill::NonZero,
                        transform,
                        indicator_color,
                        None,
                        &thumb,
                    );
                }
            }
            _ => {}
        }
    }

    pub fn state_mut(&mut self) -> &mut HydroState {
        self.dispatcher.state_mut()
    }

    #[must_use]
    pub fn scene(&self) -> &vello::Scene {
        &self.scene
    }

    pub fn reset_scene(&mut self) {
        for image in self.active_filter_images.drain(..) {
            self.vello_renderer.unregister_texture(image);
        }
        self.pointer_targets.clear();
        self.gesture_engine.clear_targets();
        self.cursor_targets.clear();
        self.hover_targets.clear();
        self.context_menu_targets.clear();
        self.text_input_targets.clear();
        self.scroll_targets.clear();
        self.scene.reset();
        self.render_layers.clear();
        self.active_scene_layers.clear();
        #[cfg(feature = "accessibility")]
        {
            self.pending_accessibility_tree_update = None;
            self.pending_text_input_accessibility_nodes.clear();
        }
    }

    pub fn begin_rebuild_frame(&mut self) {
        self.current_frame_retain.clear();
        self.lifecycle_disappear_current.clear();
        self.lifecycle_disappear_slot = 0;
        self.hit_test_opacity = 1.0;
        self.hit_test_order = 0;
        self.gesture_group_ids.clear();
        self.next_gesture_group_id = 0;
        self.animation_controller.begin_rebuild_frame();
        self.scroll_controller.begin_rebuild_frame();
        self.hover_controller.begin_rebuild_frame();
        self.lazy_stack_controller.begin_rebuild_frame();
        self.lazy_list_controller.begin_rebuild_frame();
        self.lazy_table_controller.begin_rebuild_frame();
        self.lazy_viewport_stack.clear();
        self.pending_scroll_handles.clear();
        self.pending_navigation_entries.clear();
        self.navigation_cursor = 0;
        self.gpu_surface_cursor = 0;
        self.render_layers.clear();
        self.active_scene_layers.clear();
        self.dynamic_identities_current_frame.clear();
        self.picker_menu_cursor = 0;
        self.text_selection_cursor = 0;
        self.local_state_registry.borrow_mut().begin_rebuild_frame();
        #[cfg(feature = "accessibility")]
        {
            self.accessibility_nodes.clear();
            self.accessibility_root_children.clear();
            self.accessibility_actions.clear();
            self.accessibility_next_node_id = ACCESSIBILITY_FIRST_NODE_ID;
            self.pending_text_input_accessibility_nodes.clear();
            self.accessibility_suppression_depth = 0;
        }
    }

    pub fn finish_rebuild_frame(&mut self) {
        assert!(
            self.active_scene_layers.is_empty(),
            "hydrolysis renderer: scene layer stack must be empty at end of rebuild (len={})",
            self.active_scene_layers.len()
        );
        self.flush_vello_scene_layer();
        self.previous_frame_retain = core::mem::take(&mut self.current_frame_retain);

        let previous_hooks = core::mem::take(&mut self.lifecycle_disappear_previous);
        for (slot, hook) in previous_hooks {
            if !self.lifecycle_disappear_current.contains_key(&slot) {
                hook.call();
            }
        }
        self.lifecycle_disappear_previous = core::mem::take(&mut self.lifecycle_disappear_current);

        if matches!(
            self.focused_text_input.get(),
            Some(index) if index >= self.text_input_targets.len()
        ) {
            self.set_focused_text_input(None);
        }
        if matches!(
            self.active_text_selection_drag,
            Some(index) if index >= self.text_input_targets.len()
        ) {
            self.active_text_selection_drag = None;
        }

        self.animation_controller.finish_rebuild_frame();
        self.scroll_controller.finish_rebuild_frame();
        self.hover_controller.finish_rebuild_frame();
        self.lazy_stack_controller.finish_rebuild_frame();
        self.lazy_list_controller.finish_rebuild_frame();
        self.lazy_table_controller.finish_rebuild_frame();
        self.navigation_slots.truncate(self.navigation_cursor);
        self.gpu_surface_slots.truncate(self.gpu_surface_cursor);
        self.picker_menu_slots.truncate(self.picker_menu_cursor);
        self.text_selection_slots
            .truncate(self.text_selection_cursor);
        self.local_state_registry
            .borrow_mut()
            .finish_rebuild_frame();
        let active_dynamic_identities = self.dynamic_identities_current_frame.clone();
        self.dynamic_nodes
            .retain(|identity, _| active_dynamic_identities.contains(identity));
        self.dispatcher
            .state_mut()
            .dynamic_intrinsic_cache
            .retain(|identity, _| active_dynamic_identities.contains(identity));
        #[cfg(feature = "accessibility")]
        self.finalize_accessibility_tree_update();
    }

    pub fn scene_mut(&mut self) -> &mut vello::Scene {
        &mut self.scene
    }

    pub fn vello_renderer(&mut self) -> &mut vello::Renderer {
        &mut self.vello_renderer
    }

    pub fn dispatcher_mut(&mut self) -> &mut ViewDispatcher<HydroState, RenderContext, ()> {
        &mut self.dispatcher
    }

    pub fn set_frame_resources(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.dispatcher
            .state_mut()
            .set_frame_resources(device, queue);
    }

    pub fn clear_frame_resources(&mut self) {
        self.dispatcher.state_mut().clear_frame_resources();
    }

    fn bind_gpu_surface_slot(&mut self, surface: GpuSurface, env: &Environment) -> usize {
        let index = self.gpu_surface_cursor;
        self.gpu_surface_cursor = self
            .gpu_surface_cursor
            .checked_add(1)
            .expect("hydrolysis gpu surface slot cursor overflow");

        if index == self.gpu_surface_slots.len() {
            self.gpu_surface_slots
                .push(EmbeddedGpuSurfaceRuntime::new(surface, env));
        } else {
            self.gpu_surface_slots[index].replace_surface(surface, env);
        }

        index
    }

    fn push_layer_rect(
        &mut self,
        alpha: f32,
        transform: vello::kurbo::Affine,
        rect: vello::kurbo::Rect,
    ) {
        self.scene.push_layer(
            vello::peniko::Fill::NonZero,
            vello::peniko::BlendMode::default(),
            alpha,
            transform,
            &rect,
        );
        self.active_scene_layers.push(ActiveSceneLayer {
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
        self.scene.push_layer(
            vello::peniko::Fill::NonZero,
            vello::peniko::BlendMode::default(),
            alpha,
            transform,
            &path,
        );
        self.active_scene_layers.push(ActiveSceneLayer {
            alpha,
            transform,
            shape: LayerShape::Path(path),
        });
    }

    fn pop_layer(&mut self) {
        self.scene.pop_layer();
        self.active_scene_layers
            .pop()
            .expect("hydrolysis renderer: pop_layer underflow");
    }

    fn flush_vello_scene_layer(&mut self) {
        assert!(
            (self.scene.encoding().n_open_clips as usize) == self.active_scene_layers.len(),
            "hydrolysis renderer: scene clip count {} does not match tracked scene layers {}",
            self.scene.encoding().n_open_clips,
            self.active_scene_layers.len()
        );

        for _ in 0..self.active_scene_layers.len() {
            self.scene.pop_layer();
        }

        if self.scene.encoding().is_empty() {
            for layer in &self.active_scene_layers {
                layer.push_to_scene(&mut self.scene);
            }
            return;
        }
        let scene = core::mem::take(&mut self.scene);
        self.render_layers.push(RenderLayer::Vello(scene));

        for layer in &self.active_scene_layers {
            layer.push_to_scene(&mut self.scene);
        }
    }

    fn push_gpu_surface_layer(
        &mut self,
        slot_index: usize,
        transform: vello::kurbo::Affine,
        bounds: vello::kurbo::Rect,
    ) {
        self.flush_vello_scene_layer();
        self.render_layers
            .push(RenderLayer::GpuSurface(GpuSurfaceLayer {
                slot_index,
                transform,
                bounds,
                active_layers: self.active_scene_layers.clone(),
            }));
    }

    pub fn poll_gpu_surface_redraw_handles(&mut self) -> bool {
        let mut requested = false;
        for runtime in &self.gpu_surface_slots {
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
    pub fn rebuild_handle(&self) -> Rc<Cell<bool>> {
        Rc::clone(&self.rebuild_requested)
    }

    pub fn take_rebuild_request(&self) -> bool {
        self.rebuild_requested.replace(false)
    }

    #[must_use]
    pub fn focused_text_input_state(&self) -> Option<TextInputState> {
        let index = self.focused_text_input.get()?;
        let target = self.text_input_targets.as_slice().get(index)?;
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
        self.cursor_targets
            .iter()
            .rev()
            .find(|target| target.bounds.contains(point))
            .map_or(CursorStyle::Arrow, |target| target.style)
    }

    #[cfg(feature = "accessibility")]
    pub fn set_accessibility_root_label(&mut self, label: &str) {
        self.accessibility_root_label.clear();
        self.accessibility_root_label.push_str(label);
    }

    #[cfg(feature = "accessibility")]
    #[must_use]
    pub fn take_accessibility_tree_update(&mut self) -> Option<AccessibilityTreeUpdate> {
        self.pending_accessibility_tree_update.take()
    }

    #[cfg(feature = "accessibility")]
    pub fn handle_accessibility_action(
        &mut self,
        request: AccessibilityActionRequest,
        env: &Environment,
    ) -> bool {
        let action = request.action;
        let action_data = request.data;
        let target_node = request.target_node;
        let focus_action = matches!(
            action,
            AccessibilityAction::Focus | AccessibilityAction::Click
        );
        if target_node == ACCESSIBILITY_ROOT_NODE_ID {
            return match action {
                AccessibilityAction::Focus => {
                    let changed = self.accessibility_focus != ACCESSIBILITY_ROOT_NODE_ID;
                    self.accessibility_focus = ACCESSIBILITY_ROOT_NODE_ID;
                    changed
                }
                AccessibilityAction::Click => false,
                _ => panic!(
                    "hydrolysis accessibility root does not support action {:?}",
                    action
                ),
            };
        }
        let target = self
            .accessibility_actions
            .get(&target_node)
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "hydrolysis accessibility action {:?} targets unmapped node {:?}",
                    action, target_node
                )
            });
        let changed = match target {
            AccessibilityActionTarget::PointerPrimaryClick { point } => {
                handle_accessibility_pointer_action(self, action, point, env)
            }
            AccessibilityActionTarget::Toggle { binding } => match action {
                AccessibilityAction::Click => {
                    let next = !binding.get();
                    binding.set(next);
                    true
                }
                AccessibilityAction::Focus => true,
                _ => panic!(
                    "hydrolysis accessibility toggle does not support action {:?}",
                    action
                ),
            },
            AccessibilityActionTarget::Slider { value, range, step } => {
                handle_accessibility_slider_action(
                    &value,
                    *range.start(),
                    *range.end(),
                    step,
                    action,
                    action_data,
                )
            }
            AccessibilityActionTarget::Stepper { value, step, range } => {
                handle_accessibility_stepper_action(
                    &value,
                    &step,
                    *range.start(),
                    *range.end(),
                    action,
                    action_data,
                )
            }
            AccessibilityActionTarget::TextField { value, line_limit } => {
                handle_accessibility_text_field_action(
                    self,
                    target_node,
                    &value,
                    line_limit,
                    action,
                    action_data,
                )
            }
            AccessibilityActionTarget::SecureField { value } => {
                handle_accessibility_secure_field_action(
                    self,
                    target_node,
                    &value,
                    action,
                    action_data,
                )
            }
            AccessibilityActionTarget::PickerCycle { selection, ids } => {
                handle_accessibility_picker_cycle_action(&selection, &ids, action)
            }
            AccessibilityActionTarget::PickerSelect { selection, target } => {
                handle_accessibility_picker_select_action(&selection, target, action)
            }
            AccessibilityActionTarget::Scroll { handle, axis } => {
                handle_accessibility_scroll_action(&handle, axis, action)
            }
        };
        if changed && focus_action {
            self.accessibility_focus = target_node;
        }
        changed
    }

    pub fn advance_animations(&mut self) -> bool {
        let now = Instant::now();
        self.animation_controller.tick(now)
            || self.navigation_slots.iter().any(|slot| {
                slot.transition
                    .as_ref()
                    .is_some_and(|state| state.is_active(now))
            })
            || self.advance_text_caret_animation(now)
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
            self.accessibility_root_bounds = transformed_rect(hit_transform, bounds);
        }
        let mut local_env = env.clone();
        let local_state_registry = Rc::clone(&self.local_state_registry);
        local_env.insert(LocalStateScope::root());
        local_env.insert(LocalStateStore::new(move |path, index, type_id, init| {
            local_state_registry
                .borrow_mut()
                .bind_slot(path, index, type_id, &*init)
        }));
        let ctx = RenderContext::with_renderer_transforms(self, bounds, transform, hit_transform);
        self.render_depth = 0;
        self.dispatch_with_render_depth(view, &local_env, ctx);
    }

    pub fn render_scene_to_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        target_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        base_color: vello::peniko::Color,
    ) {
        self.render_scene_to_surface(
            device,
            queue,
            target,
            target_format,
            width,
            height,
            base_color,
        );
    }

    fn ensure_gpu_surface_compositor_state(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) {
        if self.gpu_surface_compositor.is_none() {
            self.gpu_surface_compositor =
                Some(GpuSurfaceCompositorState::new(device, queue, target_format));
            return;
        }
        self.gpu_surface_compositor
            .as_mut()
            .expect("hydrolysis renderer: missing gpu surface compositor state")
            .ensure_target_format(device, queue, target_format);
    }

    fn ensure_vello_layer_surface(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let size = (width, height);
        let needs_recreate = self
            .vello_layer_surface
            .as_ref()
            .is_none_or(|state| state.size != size);
        if !needs_recreate {
            return;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hydrolysis_vello_layer_surface"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.vello_layer_surface = Some(VelloLayerSurfaceState {
            size,
            _texture: texture,
            view,
        });
    }

    fn render_vello_layer_to_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &vello::Scene,
        width: u32,
        height: u32,
    ) -> wgpu::TextureView {
        self.ensure_vello_layer_surface(device, width, height);
        let view = self
            .vello_layer_surface
            .as_ref()
            .expect("hydrolysis renderer: missing vello layer surface state")
            .view
            .clone();
        let params = vello::RenderParams {
            base_color: vello::peniko::Color::TRANSPARENT,
            width,
            height,
            antialiasing_method: vello::AaConfig::Area,
        };
        self.vello_renderer
            .render_to_texture(device, queue, scene, &view, &params)
            .expect("hydrolysis renderer: failed to render vello layer scene");
        view
    }

    fn render_active_layers_mask_to_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        active_layers: &[ActiveSceneLayer],
    ) -> wgpu::TextureView {
        assert!(
            !active_layers.is_empty(),
            "hydrolysis renderer: active layer mask requires at least one layer"
        );
        let mut mask_scene = vello::Scene::new();
        for layer in active_layers {
            layer.push_to_scene(&mut mask_scene);
        }
        mask_scene.fill(
            vello::peniko::Fill::NonZero,
            vello::kurbo::Affine::IDENTITY,
            vello::peniko::Color::WHITE,
            None,
            &vello::kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
        );
        for _ in 0..active_layers.len() {
            mask_scene.pop_layer();
        }
        self.render_vello_layer_to_texture(device, queue, &mask_scene, width, height)
    }

    fn default_compositor_mask_view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> wgpu::TextureView {
        self.ensure_gpu_surface_compositor_state(device, queue, target_format);
        self.gpu_surface_compositor
            .as_ref()
            .expect("hydrolysis renderer: missing gpu surface compositor state")
            .white_mask_view
            .clone()
    }

    fn clear_target_surface(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        base_color: vello::peniko::Color,
    ) {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("hydrolysis_surface_clear_encoder"),
        });
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hydrolysis_surface_clear_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color_to_wgpu(base_color)),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        drop(_pass);
        queue.submit(std::iter::once(encoder.finish()));
    }

    fn composite_texture_layer(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        target_format: wgpu::TextureFormat,
        layer_view: &wgpu::TextureView,
        mask_view: &wgpu::TextureView,
        uniform_bytes: &[u8; 64],
        load_op: wgpu::LoadOp<wgpu::Color>,
    ) {
        self.ensure_gpu_surface_compositor_state(device, queue, target_format);
        let compositor = self
            .gpu_surface_compositor
            .as_ref()
            .expect("hydrolysis renderer: missing gpu surface compositor state");

        queue.write_buffer(&compositor.uniform_buffer, 0, uniform_bytes);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_bind_group"),
            layout: &compositor.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: compositor.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&compositor.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(layer_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(mask_view),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_encoder"),
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("hydrolysis_gpu_surface_compositor_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: load_op,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&compositor.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..6, 0..1);
        drop(pass);
        queue.submit(std::iter::once(encoder.finish()));
    }

    pub fn render_scene_to_surface(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        target_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        base_color: vello::peniko::Color,
    ) {
        assert!(
            matches!(
                target_format.remove_srgb_suffix(),
                wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Bgra8Unorm
            ) || matches!(
                target_format,
                wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
            ),
            "hydrolysis renderer: unsupported surface format {target_format:?}"
        );

        self.flush_vello_scene_layer();
        let fullscreen_uniform =
            encode_compositor_uniform([[-1.0, 1.0], [1.0, 1.0], [1.0, -1.0], [-1.0, -1.0]]);
        let render_layers = core::mem::take(&mut self.render_layers);
        if render_layers.is_empty() {
            self.clear_target_surface(device, queue, target, base_color);
            return;
        }
        let mut needs_redraw = false;
        let mut is_first_layer = true;

        for layer in &render_layers {
            let load_op = if is_first_layer {
                wgpu::LoadOp::Clear(color_to_wgpu(base_color))
            } else {
                wgpu::LoadOp::Load
            };
            is_first_layer = false;
            match layer {
                RenderLayer::Vello(scene) => {
                    let view =
                        self.render_vello_layer_to_texture(device, queue, scene, width, height);
                    let mask_view = self.default_compositor_mask_view(device, queue, target_format);
                    self.composite_texture_layer(
                        device,
                        queue,
                        target,
                        target_format,
                        &view,
                        &mask_view,
                        &fullscreen_uniform,
                        load_op,
                    );
                }
                RenderLayer::GpuSurface(layer) => {
                    let prepared = self
                        .gpu_surface_slots
                        .get_mut(layer.slot_index)
                        .unwrap_or_else(|| {
                            panic!("hydrolysis gpu surface slot {} missing", layer.slot_index)
                        })
                        .prepare_layer(
                            device,
                            queue,
                            target_format,
                            width,
                            height,
                            layer.transform,
                            layer.bounds,
                        );
                    if prepared.needs_redraw {
                        needs_redraw = true;
                    }
                    let mask_view = if layer.active_layers.is_empty() {
                        self.default_compositor_mask_view(device, queue, target_format)
                    } else {
                        self.render_active_layers_mask_to_texture(
                            device,
                            queue,
                            width,
                            height,
                            &layer.active_layers,
                        )
                    };
                    self.composite_texture_layer(
                        device,
                        queue,
                        target,
                        target_format,
                        &prepared.view,
                        &mask_view,
                        &prepared.uniform_bytes,
                        load_op,
                    );
                }
            }
        }
        self.render_layers = render_layers;

        if needs_redraw {
            self.request_redraw();
        }
    }

    pub fn handle_pointer_down(
        &mut self,
        x: f32,
        y: f32,
        button: PointerButton,
        env: &Environment,
    ) -> bool {
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        let at = Instant::now();
        let mut rebuild_requested = false;
        self.active_pointer_drag_target = None;
        self.active_pointer_drag_signature = None;
        self.active_text_selection_drag = None;
        let overlay_hit = matches!(
            self.active_text_context_menu,
            Some(ActiveTextContextMenu::Overlay {
                index: _,
                overlay: _
            })
        );
        if overlay_hit {
            let changed = self.handle_text_context_menu_overlay_pointer_down(point);
            if changed || self.active_text_context_menu.is_none() {
                return changed;
            }
        }
        if button != PointerButton::Secondary {
            self.dismiss_active_text_context_menu();
        }
        self.dismiss_active_popup_menu();
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            button = ?button,
            pointer_targets = self.pointer_targets.len(),
            text_inputs = self.text_input_targets.len(),
            gesture_targets = self.gesture_engine.target_count(),
            "pointer down begin"
        );

        let gesture_changed = self.gesture_engine.handle_pointer_down(point, at, env);
        rebuild_requested |= gesture_changed;

        let mut pointer_indices: Vec<usize> = self
            .pointer_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.bounds.contains(point))
            .map(|(index, _)| index)
            .collect();
        pointer_indices.sort_unstable_by(|left, right| {
            let left_target = &self.pointer_targets[*left];
            let right_target = &self.pointer_targets[*right];
            Self::target_hit_priority(right_target.depth, right_target.order, *right).cmp(
                &Self::target_hit_priority(left_target.depth, left_target.order, *left),
            )
        });
        let focused = self.topmost_text_input_index_at_point(point);
        let top_pointer_priority = pointer_indices.first().map(|index| {
            let target = &self.pointer_targets[*index];
            Self::target_hit_priority(target.depth, target.order, *index)
        });
        let focused_priority = focused.map(|index| {
            let target = &self.text_input_targets[index];
            Self::target_hit_priority(target.depth, target.order, index)
        });
        let focus_wins = matches!(
            (focused_priority, top_pointer_priority),
            (Some(focus_priority), Some(pointer_priority)) if focus_priority > pointer_priority
        ) || matches!((focused_priority, top_pointer_priority), (Some(_), None));
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            button = ?button,
            pointer_hits = ?pointer_indices,
            pointer_top_priority = ?top_pointer_priority,
            focused_candidate = ?focused,
            focused_priority = ?focused_priority,
            focus_wins,
            "pointer down candidates"
        );
        if focus_wins {
            let mut changed = self.set_focused_text_input(focused);
            if let Some(index) = focused {
                match button {
                    PointerButton::Primary => {
                        let click_count = self.next_text_selection_click_count(index, point, at);
                        changed |=
                            self.apply_text_selection_click_gesture(index, point, click_count);
                        self.active_text_selection_drag = Some(index);
                    }
                    PointerButton::Secondary => {
                        let keep_selection = {
                            let target = &self.text_input_targets[index];
                            let selection_index =
                                Self::text_selection_index_from_point(target, point);
                            let slot = target.selection.borrow();
                            selection_range_contains_index(&target.model, &slot, selection_index)
                        };
                        if !keep_selection {
                            changed |= self.update_text_selection_from_pointer(index, point, false);
                        }
                        changed |= self.show_text_context_menu(index, point, env);
                    }
                    _ => {}
                }
            }
            return rebuild_requested || changed;
        }

        if button != PointerButton::Primary {
            if button == PointerButton::Secondary
                && let Some(target) = self.topmost_context_menu_target_at_point(point)
            {
                if self.set_focused_text_input(focused) {
                    rebuild_requested = true;
                }
                let changed = self.show_popup_menu_nodes(
                    popup_menu_nodes(&target.items.get()),
                    LayoutPoint::new(point.x as f32, point.y as f32),
                    env,
                );
                return rebuild_requested || changed;
            }
            if self.set_focused_text_input(focused) {
                rebuild_requested = true;
            }
            return rebuild_requested;
        }

        for index in pointer_indices {
            let target = self.pointer_targets[index].clone();
            tracing::trace!(
                target: "waterui::hydrolysis::input",
                x,
                y,
                pointer_index = index,
                captures_drag = target.captures_drag,
                bounds = ?target.bounds,
                order = target.order,
                "dispatch pointer target"
            );
            let changed = (target.action.borrow_mut())(point, env);
            if target.captures_drag {
                self.active_pointer_drag_target = Some(Rc::clone(&target.action));
                self.active_pointer_drag_signature = Some((target.depth, target.order));
            }
            if !changed && !target.captures_drag {
                continue;
            }
            tracing::trace!(
                target: "waterui::hydrolysis::input",
                x,
                y,
                pointer_index = index,
                captures_drag = target.captures_drag,
                order = target.order,
                "pointer target handled event"
            );
            return rebuild_requested || changed;
        }
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            focused_candidate = ?focused,
            "pointer down text-input fallback"
        );
        if self.set_focused_text_input(focused) {
            rebuild_requested = true;
        }
        rebuild_requested
    }

    pub fn handle_pointer_up(
        &mut self,
        x: f32,
        y: f32,
        _button: PointerButton,
        env: &Environment,
    ) -> bool {
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        let at = Instant::now();
        let mut changed = self.handle_pointer_move(x, y, env);
        self.active_text_selection_drag = None;
        self.active_pointer_drag_target = None;
        self.active_pointer_drag_signature = None;
        let gesture_changed = self.gesture_engine.handle_pointer_up(point, at, env);
        changed |= gesture_changed;
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            changed,
            gesture_changed,
            "pointer up handled"
        );
        changed
    }

    pub fn handle_pointer_move(&mut self, x: f32, y: f32, env: &Environment) -> bool {
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        let at = Instant::now();
        let mut rebuild_requested = false;
        let mut drag_changed = false;
        if let Some(index) = self.active_text_selection_drag {
            let text_drag_changed = self.update_text_selection_from_pointer(index, point, true);
            drag_changed |= text_drag_changed;
            rebuild_requested |= text_drag_changed;
        }
        if let Some(action) = self.active_pointer_drag_target.clone() {
            let pointer_drag_changed = (action.borrow_mut())(point, env);
            drag_changed |= pointer_drag_changed;
            rebuild_requested |= pointer_drag_changed;
        }
        let gesture_changed = self.gesture_engine.handle_pointer_move(point, at, env);
        rebuild_requested |= gesture_changed;
        rebuild_requested |= self.sync_hover_targets(point, env, true);
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            changed = rebuild_requested,
            drag_changed,
            gesture_changed,
            dragging = self.active_pointer_drag_target.is_some(),
            gesture_active = self.gesture_engine.has_active_recognizer(),
            "pointer move handled"
        );
        rebuild_requested
    }

    pub fn sync_pointer_hover_state(&mut self, x: f32, y: f32, env: &Environment) -> bool {
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        let changed = self.sync_hover_targets(point, env, false);
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            changed,
            dragging = self.active_pointer_drag_target.is_some(),
            gesture_active = self.gesture_engine.has_active_recognizer(),
            "pointer hover sync handled"
        );
        changed
    }

    fn sync_hover_targets(
        &mut self,
        point: vello::kurbo::Point,
        env: &Environment,
        dispatch_move: bool,
    ) -> bool {
        let mut changed = false;
        for target in &mut self.hover_targets {
            let contains = target.bounds.contains(point);
            let slot_hovering = self.hover_controller.slots[target.slot.index].hovering;
            if contains && !slot_hovering {
                self.hover_controller.set_hovering(target.slot, true);
                if let Some(on_enter) = target.on_enter.as_mut() {
                    changed |= (on_enter.borrow_mut())(env);
                }
            } else if !contains && slot_hovering {
                self.hover_controller.set_hovering(target.slot, false);
                if let Some(on_exit) = target.on_exit.as_mut() {
                    changed |= (on_exit.borrow_mut())(env);
                }
            }
            if contains
                && dispatch_move
                && let Some(on_move) = target.on_move.as_mut()
            {
                changed |= (on_move.borrow_mut())(point, env);
            }
        }
        changed
    }

    pub fn handle_pointer_cancel(&mut self, env: &Environment) -> bool {
        let mut rebuild_requested = false;
        self.active_text_selection_drag = None;
        self.active_pointer_drag_target = None;
        self.active_pointer_drag_signature = None;
        let gesture_changed = self
            .gesture_engine
            .handle_pointer_cancel(Instant::now(), env);
        rebuild_requested |= gesture_changed;
        for target in &mut self.hover_targets {
            let hovering = self.hover_controller.slots[target.slot.index].hovering;
            if !hovering {
                continue;
            }
            self.hover_controller.set_hovering(target.slot, false);
            if let Some(on_exit) = target.on_exit.as_mut() {
                rebuild_requested |= (on_exit.borrow_mut())(env);
            }
        }
        rebuild_requested
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
        let at = Instant::now();
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
        let at = Instant::now();
        self.gesture_engine
            .handle_rotation(center, delta, phase, at, env)
    }

    pub fn handle_gesture_tick(&mut self, at: Instant, env: &Environment) -> bool {
        self.gesture_engine.handle_tick(at, env)
    }

    pub fn next_gesture_deadline(&self) -> Option<Instant> {
        let gesture_deadline = self.gesture_engine.next_deadline();
        let caret_deadline = self
            .focused_text_input
            .get()
            .and(self.text_caret_next_frame_at);
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

    fn sync_active_pointer_drag_target_after_layout(
        &mut self,
        pointer: Option<vello::kurbo::Point>,
    ) {
        let Some(active) = self.active_pointer_drag_target.as_ref() else {
            return;
        };
        let alive = self
            .pointer_targets
            .iter()
            .any(|target| Rc::ptr_eq(&target.action, active));
        if alive {
            return;
        }
        if let Some((depth, order)) = self.active_pointer_drag_signature {
            if let Some(target) = self.pointer_targets.iter().find(|target| {
                target.captures_drag && target.depth == depth && target.order == order
            }) {
                self.active_pointer_drag_target = Some(Rc::clone(&target.action));
                return;
            }
        }
        let Some(point) = pointer else {
            self.active_pointer_drag_target = None;
            self.active_pointer_drag_signature = None;
            return;
        };
        let mut indices: Vec<usize> = self
            .pointer_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.captures_drag && target.bounds.contains(point))
            .map(|(index, _)| index)
            .collect();
        indices.sort_unstable_by(|left, right| {
            let left_target = &self.pointer_targets[*left];
            let right_target = &self.pointer_targets[*right];
            Self::target_hit_priority(right_target.depth, right_target.order, *right).cmp(
                &Self::target_hit_priority(left_target.depth, left_target.order, *left),
            )
        });
        if let Some(index) = indices.first().copied() {
            let target = &self.pointer_targets[index];
            self.active_pointer_drag_target = Some(Rc::clone(&target.action));
            self.active_pointer_drag_signature = Some((target.depth, target.order));
        } else {
            self.active_pointer_drag_target = None;
            self.active_pointer_drag_signature = None;
        }
    }

    pub fn handle_text_input(&mut self, text: &str) -> bool {
        let preedit_cleared = self.ime_preedit.take().is_some();
        if text.is_empty() {
            tracing::trace!(
                target: "waterui::hydrolysis::input",
                focused = ?self.focused_text_input.get(),
                preedit_cleared,
                "text input ignored empty payload"
            );
            return preedit_cleared;
        }
        let changed = self.insert_text_into_focused_target(text) || preedit_cleared;
        if changed {
            self.reset_text_caret_animation(Instant::now());
        }
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            focused = ?self.focused_text_input.get(),
            text = text,
            changed,
            "text input handled"
        );
        changed
    }

    pub fn handle_ime_preedit(&mut self, text: &str) -> bool {
        if self.focused_text_input.get().is_none() {
            tracing::trace!(
                target: "waterui::hydrolysis::input",
                text = text,
                "ime preedit dropped without focused text input"
            );
            return false;
        }
        let next = if text.is_empty() {
            None
        } else {
            Some(Str::from(text.to_owned()))
        };
        if self.ime_preedit == next {
            return false;
        }
        self.ime_preedit = next;
        self.reset_text_caret_animation(Instant::now());
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            focused = ?self.focused_text_input.get(),
            preedit = ?self.ime_preedit,
            "ime preedit updated"
        );
        true
    }

    pub fn handle_ime_commit(&mut self, text: &str) -> bool {
        self.handle_text_input(text)
    }

    pub fn handle_ime_disabled(&mut self) -> bool {
        let changed = self.ime_preedit.take().is_some();
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            changed,
            "ime disabled handled"
        );
        changed
    }

    pub fn handle_key(&mut self, key: &KeyCode, modifiers: Modifiers) -> bool {
        if self.focused_text_input.get().is_none() {
            return false;
        }

        if modifiers.alt {
            tracing::trace!(
                target: "waterui::hydrolysis::input",
                key = ?key,
                modifiers = ?modifiers,
                "key ignored due command modifiers"
            );
            return false;
        }

        let command_modifier = modifiers.control || modifiers.super_key;
        let changed = if command_modifier {
            match key {
                KeyCode::Character(value) => match value.to_ascii_lowercase().as_str() {
                    "a" => self.select_all_in_focused_target(),
                    "c" => self.copy_selection_in_focused_target(),
                    "x" => self.cut_selection_in_focused_target(),
                    "v" => self.paste_clipboard_into_focused_target(),
                    _ => false,
                },
                KeyCode::Named(value) if value == "Escape" => {
                    let changed = self.active_text_context_menu.is_some()
                        || self.active_popup_menu_group.is_some();
                    self.dismiss_active_text_context_menu();
                    self.dismiss_active_popup_menu();
                    changed
                }
                _ => false,
            }
        } else {
            match key {
                KeyCode::Named(value) if value == "Backspace" => {
                    if self.ime_preedit.take().is_some() {
                        true
                    } else {
                        self.delete_backward_in_focused_target()
                    }
                }
                KeyCode::Named(value) if value == "Delete" => {
                    if self.ime_preedit.take().is_some() {
                        true
                    } else {
                        self.delete_forward_in_focused_target()
                    }
                }
                KeyCode::Named(value) if value == "ArrowLeft" => {
                    self.move_focused_caret_horizontal(true, modifiers.shift)
                }
                KeyCode::Named(value) if value == "ArrowRight" => {
                    self.move_focused_caret_horizontal(false, modifiers.shift)
                }
                KeyCode::Named(value) if value == "Home" => {
                    self.move_focused_caret_to_boundary(false, modifiers.shift)
                }
                KeyCode::Named(value) if value == "End" => {
                    self.move_focused_caret_to_boundary(true, modifiers.shift)
                }
                KeyCode::Named(value) if value == "Escape" => {
                    let changed = self.active_text_context_menu.is_some()
                        || self.active_popup_menu_group.is_some();
                    self.dismiss_active_text_context_menu();
                    self.dismiss_active_popup_menu();
                    changed
                }
                KeyCode::Character(text) => {
                    if self.ime_preedit.is_some() || text.is_empty() {
                        false
                    } else {
                        self.insert_text_into_focused_target(text.as_str())
                    }
                }
                KeyCode::Named(_) | KeyCode::Unidentified => false,
            }
        };
        if changed {
            self.reset_text_caret_animation(Instant::now());
        }
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            key = ?key,
            modifiers = ?modifiers,
            focused = ?self.focused_text_input.get(),
            changed,
            "key handled"
        );
        changed
    }

    pub fn handle_scroll(&mut self, x: f32, y: f32, dx: f32, dy: f32, is_line_delta: bool) -> bool {
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        for target in self.scroll_targets.iter_mut().rev() {
            if target.bounds.contains(point) {
                return (target.action.borrow_mut())(dx, dy, is_line_delta);
            }
        }
        false
    }

    fn register_pointer_target<F>(&mut self, bounds: vello::kurbo::Rect, action: F)
    where
        F: 'static + FnMut(vello::kurbo::Point, &Environment) -> bool,
    {
        self.register_pointer_target_action(
            bounds,
            false,
            Rc::new(RefCell::new(action)),
            self.render_depth,
        );
    }

    fn register_pointer_drag_target<F>(&mut self, bounds: vello::kurbo::Rect, action: F)
    where
        F: 'static + FnMut(vello::kurbo::Point, &Environment) -> bool,
    {
        self.register_pointer_target_action(
            bounds,
            true,
            Rc::new(RefCell::new(action)),
            self.render_depth,
        );
    }

    fn register_pointer_target_action(
        &mut self,
        bounds: vello::kurbo::Rect,
        captures_drag: bool,
        action: PointerAction,
        depth: usize,
    ) {
        if self.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let order = self.next_hit_test_order();
        self.pointer_targets.push(PointerTarget {
            bounds,
            captures_drag,
            depth,
            order,
            action,
        });
    }

    fn register_gesture_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        group_id: usize,
        gesture: Gesture,
        action: BoxedAction<()>,
    ) {
        if self.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
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
        if self.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
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

    fn ensure_active_pointer_drag_target_is_live(&mut self) {
        let Some(active) = self.active_pointer_drag_target.as_ref() else {
            return;
        };
        let alive = self
            .pointer_targets
            .iter()
            .any(|target| Rc::ptr_eq(&target.action, active));
        if !alive {
            self.active_pointer_drag_target = None;
            self.active_pointer_drag_signature = None;
        }
    }

    fn register_cursor_target(&mut self, bounds: vello::kurbo::Rect, style: CursorStyle) {
        self.register_cursor_target_style(bounds, style);
    }

    fn register_cursor_target_style(&mut self, bounds: vello::kurbo::Rect, style: CursorStyle) {
        if self.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        self.cursor_targets.push(CursorTarget { bounds, style });
    }

    fn register_hover_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        on_enter: Option<HoverAction>,
        on_move: Option<PointerAction>,
        on_exit: Option<HoverAction>,
    ) {
        if self.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let (slot, _hovering) = self.hover_controller.bind();
        self.hover_targets.push(HoverTarget {
            bounds,
            slot,
            on_enter,
            on_move,
            on_exit,
        });
    }

    fn register_hover_enter_target<F>(&mut self, bounds: vello::kurbo::Rect, action: F)
    where
        F: 'static + FnMut(&Environment) -> bool,
    {
        self.register_hover_target(bounds, Some(Rc::new(RefCell::new(action))), None, None);
    }

    fn register_hover_exit_target<F>(&mut self, bounds: vello::kurbo::Rect, action: F)
    where
        F: 'static + FnMut(&Environment) -> bool,
    {
        self.register_hover_target(bounds, None, None, Some(Rc::new(RefCell::new(action))));
    }

    fn register_hover_move_target<F>(&mut self, bounds: vello::kurbo::Rect, action: F)
    where
        F: 'static + FnMut(vello::kurbo::Point, &Environment) -> bool,
    {
        self.register_hover_target(bounds, None, Some(Rc::new(RefCell::new(action))), None);
    }

    fn register_text_input_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        cursor_area: vello::kurbo::Rect,
        text_bounds: vello::kurbo::Rect,
        layout: parley::Layout<[u8; 4]>,
        purpose: TextInputPurpose,
        model: TextInputModel,
        selection: Rc<RefCell<TextSelectionSlot>>,
    ) {
        #[cfg(feature = "accessibility")]
        let accessibility_node_id = self.take_pending_text_input_accessibility_node();
        self.register_text_input_target_data(
            bounds,
            cursor_area,
            text_bounds,
            layout,
            purpose,
            model,
            selection,
            self.render_depth,
            None,
            #[cfg(feature = "accessibility")]
            accessibility_node_id,
        );
    }

    fn register_text_input_target_data(
        &mut self,
        bounds: vello::kurbo::Rect,
        cursor_area: vello::kurbo::Rect,
        text_bounds: vello::kurbo::Rect,
        layout: parley::Layout<[u8; 4]>,
        purpose: TextInputPurpose,
        model: TextInputModel,
        selection: Rc<RefCell<TextSelectionSlot>>,
        depth: usize,
        focus_binding: Option<Binding<bool>>,
        #[cfg(feature = "accessibility")] accessibility_node_id: Option<AccessibilityNodeId>,
    ) {
        if self.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let order = self.next_hit_test_order();
        self.text_input_targets.push(TextInputTarget {
            bounds,
            cursor_area,
            text_bounds,
            layout,
            purpose,
            depth,
            order,
            model,
            selection,
            focus_binding,
            #[cfg(feature = "accessibility")]
            accessibility_node_id,
        });
    }

    #[cfg(feature = "accessibility")]
    fn push_pending_text_input_accessibility_node(&mut self, node_id: AccessibilityNodeId) {
        self.pending_text_input_accessibility_nodes
            .push_back(node_id);
    }

    #[cfg(feature = "accessibility")]
    fn take_pending_text_input_accessibility_node(&mut self) -> Option<AccessibilityNodeId> {
        self.pending_text_input_accessibility_nodes.pop_front()
    }

    fn register_scroll_target<F>(&mut self, bounds: vello::kurbo::Rect, action: F)
    where
        F: 'static + FnMut(f32, f32, bool) -> bool,
    {
        self.register_scroll_target_action(bounds, Rc::new(RefCell::new(action)));
    }

    fn register_scroll_target_action(&mut self, bounds: vello::kurbo::Rect, action: ScrollAction) {
        if self.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        self.scroll_targets.push(ScrollTarget { bounds, action });
    }

    #[cfg(feature = "accessibility")]
    fn push_accessibility_suppression(&mut self) {
        self.accessibility_suppression_depth = self
            .accessibility_suppression_depth
            .checked_add(1)
            .expect("hydrolysis accessibility suppression depth overflow");
    }

    #[cfg(feature = "accessibility")]
    fn pop_accessibility_suppression(&mut self) {
        self.accessibility_suppression_depth = self
            .accessibility_suppression_depth
            .checked_sub(1)
            .expect("hydrolysis accessibility suppression underflow");
    }

    #[cfg(feature = "accessibility")]
    fn apply_accessibility_state(&self, env: &Environment, node: &mut AccessibilityNode) {
        let Some(state) = env.get::<AccessibilityState>() else {
            return;
        };
        if state.is_disabled() {
            node.set_disabled();
        }
        if state.is_selected() {
            node.set_selected(true);
        }
        if let Some(checked) = state.checked_state() {
            node.set_toggled(AccessibilityToggled::from(checked));
        }
        if let Some(expanded) = state.expanded_state() {
            node.set_expanded(expanded);
        }
        if state.is_busy() {
            node.set_busy();
        }
        if state.is_hidden() {
            node.set_hidden();
        }
    }

    #[cfg(feature = "accessibility")]
    fn register_accessibility_node_internal(
        &mut self,
        mut node: AccessibilityNode,
        bounds: vello::kurbo::Rect,
        env: &Environment,
        action_target: Option<AccessibilityActionTarget>,
        attach_to_root: bool,
    ) -> Option<AccessibilityNodeId> {
        if self.accessibility_suppression_depth > 0 {
            return None;
        }
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            return None;
        }
        self.apply_accessibility_state(env, &mut node);
        let node_id = self.next_accessibility_node_id();
        node.set_bounds(kurbo_rect_to_accesskit_rect(bounds));
        if attach_to_root {
            self.accessibility_root_children.push(node_id);
        }
        self.accessibility_nodes.push((node_id, node));
        if let Some(target) = action_target {
            self.accessibility_actions.insert(node_id, target);
        }
        Some(node_id)
    }

    #[cfg(feature = "accessibility")]
    fn register_accessibility_node(
        &mut self,
        node: AccessibilityNode,
        bounds: vello::kurbo::Rect,
        env: &Environment,
        action_target: Option<AccessibilityActionTarget>,
    ) -> Option<AccessibilityNodeId> {
        self.register_accessibility_node_internal(node, bounds, env, action_target, true)
    }

    #[cfg(feature = "accessibility")]
    fn register_accessibility_child_node(
        &mut self,
        node: AccessibilityNode,
        bounds: vello::kurbo::Rect,
        env: &Environment,
        action_target: Option<AccessibilityActionTarget>,
    ) -> Option<AccessibilityNodeId> {
        self.register_accessibility_node_internal(node, bounds, env, action_target, false)
    }

    #[cfg(feature = "accessibility")]
    fn accessibility_label_from_view(
        &mut self,
        view: &AnyView,
        env: &Environment,
    ) -> Option<String> {
        self.accessibility_label_from_view_with_budget(view, env, 32)
    }

    #[cfg(feature = "accessibility")]
    fn accessibility_label_from_view_with_budget(
        &mut self,
        view: &AnyView,
        env: &Environment,
        remaining: usize,
    ) -> Option<String> {
        assert!(
            !(remaining == 0),
            "hydrolysis accessibility label extraction exceeded recursion budget for {}",
            view.name()
        );
        if let Some(metadata) = view.downcast_ref::<Metadata<Environment>>() {
            return self.accessibility_label_from_view_with_budget(
                &metadata.content,
                &metadata.value,
                remaining - 1,
            );
        }
        if let Some(content) = passthrough_content(view) {
            return self.accessibility_label_from_view_with_budget(content, env, remaining - 1);
        }
        if let Some(label) = view.downcast_ref::<SemanticLabel>() {
            let styled = self.read_signal(&label.__text().__resolve(env).content);
            return Some(styled.to_plain().to_string());
        }
        if let Some(label) = view.downcast_ref::<Str>() {
            return Some(label.as_str().to_owned());
        }
        if let Some(label) = view.downcast_ref::<&'static str>() {
            return Some((*label).to_owned());
        }
        if let Some(label) = view.downcast_ref::<String>() {
            return Some(label.clone());
        }
        if let Some(label) = view.downcast_ref::<Cow<'static, str>>() {
            return Some(label.to_string());
        }
        if let Some(text) = view.downcast_ref::<Native<TextConfig>>() {
            let styled = self.read_signal(&text.as_inner().content);
            return Some(styled.to_plain().to_string());
        }
        if let Some(icon) = view.downcast_ref::<Native<SystemIcon>>() {
            return Some(icon.as_inner().name.as_str().to_owned());
        }
        None
    }

    #[cfg(feature = "accessibility")]
    fn resolve_accessibility_label(
        &mut self,
        env: &Environment,
        default_label: Option<String>,
    ) -> Option<String> {
        env.get::<AccessibilityLabel>()
            .map(|label| label.as_str().as_str().to_owned())
            .or(default_label)
    }

    #[cfg(feature = "accessibility")]
    fn resolve_accessibility_role(
        &self,
        env: &Environment,
        default_role: AccessibilityNodeRole,
    ) -> AccessibilityNodeRole {
        env.get::<AccessibilityRole>()
            .map(|role| accessibility_role_to_accesskit_role(role.clone()))
            .unwrap_or(default_role)
    }

    #[cfg(feature = "accessibility")]
    fn finalize_accessibility_tree_update(&mut self) {
        let mut root = AccessibilityNode::new(AccessibilityNodeRole::Window);
        root.set_label(self.accessibility_root_label.clone());
        root.set_bounds(kurbo_rect_to_accesskit_rect(self.accessibility_root_bounds));
        root.set_children(self.accessibility_root_children.clone());
        let mut nodes = Vec::with_capacity(self.accessibility_nodes.len() + 1);
        nodes.push((ACCESSIBILITY_ROOT_NODE_ID, root));
        nodes.extend(self.accessibility_nodes.iter().cloned());
        if !self
            .accessibility_nodes
            .iter()
            .any(|(id, _)| *id == self.accessibility_focus)
        {
            self.accessibility_focus = ACCESSIBILITY_ROOT_NODE_ID;
        }
        self.pending_accessibility_tree_update = Some(AccessibilityTreeUpdate {
            nodes,
            tree: Some(AccessibilityTree::new(ACCESSIBILITY_ROOT_NODE_ID)),
            tree_id: AccessibilityTreeId::ROOT,
            focus: self.accessibility_focus,
        });
    }
}

fn color_to_wgpu(color: vello::peniko::Color) -> wgpu::Color {
    wgpu::Color {
        r: f64::from(color.components[0]),
        g: f64::from(color.components[1]),
        b: f64::from(color.components[2]),
        a: f64::from(color.components[3]),
    }
}

impl Drop for HydrolysisRenderer {
    fn drop(&mut self) {
        for (_, hook) in core::mem::take(&mut self.lifecycle_disappear_previous) {
            hook.call();
        }
        for (_, hook) in core::mem::take(&mut self.lifecycle_disappear_current) {
            hook.call();
        }
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
fn remap_accessibility_node_id(
    node_id: AccessibilityNodeId,
    id_map: &BTreeMap<AccessibilityNodeId, AccessibilityNodeId>,
) -> AccessibilityNodeId {
    *id_map
        .get(&node_id)
        .expect("hydrolysis dynamic accessibility node mapping missing reference")
}

#[cfg(feature = "accessibility")]
fn remap_accessibility_node_id_vec(
    node_ids: &[AccessibilityNodeId],
    id_map: &BTreeMap<AccessibilityNodeId, AccessibilityNodeId>,
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
    id_map: &BTreeMap<AccessibilityNodeId, AccessibilityNodeId>,
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

#[cfg(feature = "accessibility")]
fn register_accessibility_text_run_node(
    renderer: &mut HydrolysisRenderer,
    value: &str,
    bounds: vello::kurbo::Rect,
    env: &Environment,
) -> Option<AccessibilityNodeId> {
    let mut text_run = AccessibilityNode::new(AccessibilityNodeRole::TextRun);
    text_run.set_value(value.to_owned());
    text_run.set_character_lengths(accessibility_character_lengths(value));
    renderer.register_accessibility_child_node(text_run, bounds, env, None)
}

#[cfg(feature = "accessibility")]
fn accessibility_character_lengths(value: &str) -> Vec<u8> {
    value
        .chars()
        .map(|ch| {
            u8::try_from(ch.len_utf8())
                .expect("hydrolysis accessibility text run utf8 length must fit into u8")
        })
        .collect()
}

#[cfg(feature = "accessibility")]
fn collapsed_accessibility_text_selection(
    node_id: AccessibilityNodeId,
    character_index: usize,
) -> AccessibilityTextSelection {
    AccessibilityTextSelection {
        anchor: AccessibilityTextPosition {
            node: node_id,
            character_index,
        },
        focus: AccessibilityTextPosition {
            node: node_id,
            character_index,
        },
    }
}

#[cfg(feature = "accessibility")]
fn transform_accessibility_action_target(
    target: &AccessibilityActionTarget,
    transform: vello::kurbo::Affine,
) -> AccessibilityActionTarget {
    match target {
        AccessibilityActionTarget::PointerPrimaryClick { point } => {
            AccessibilityActionTarget::PointerPrimaryClick {
                point: transform * *point,
            }
        }
        AccessibilityActionTarget::Toggle { binding } => AccessibilityActionTarget::Toggle {
            binding: binding.clone(),
        },
        AccessibilityActionTarget::Slider { value, range, step } => {
            AccessibilityActionTarget::Slider {
                value: value.clone(),
                range: range.clone(),
                step: *step,
            }
        }
        AccessibilityActionTarget::Stepper { value, step, range } => {
            AccessibilityActionTarget::Stepper {
                value: value.clone(),
                step: step.clone(),
                range: range.clone(),
            }
        }
        AccessibilityActionTarget::TextField { value, line_limit } => {
            AccessibilityActionTarget::TextField {
                value: value.clone(),
                line_limit: *line_limit,
            }
        }
        AccessibilityActionTarget::SecureField { value } => {
            AccessibilityActionTarget::SecureField {
                value: value.clone(),
            }
        }
        AccessibilityActionTarget::PickerCycle { selection, ids } => {
            AccessibilityActionTarget::PickerCycle {
                selection: selection.clone(),
                ids: ids.clone(),
            }
        }
        AccessibilityActionTarget::PickerSelect { selection, target } => {
            AccessibilityActionTarget::PickerSelect {
                selection: selection.clone(),
                target: *target,
            }
        }
        AccessibilityActionTarget::Scroll { handle, axis } => AccessibilityActionTarget::Scroll {
            handle: handle.clone(),
            axis: axis.clone(),
        },
    }
}

#[cfg(feature = "accessibility")]
fn accessibility_activation_point(bounds: vello::kurbo::Rect) -> vello::kurbo::Point {
    vello::kurbo::Point::new((bounds.x0 + bounds.x1) * 0.5, (bounds.y0 + bounds.y1) * 0.5)
}

#[cfg(feature = "accessibility")]
fn accessibility_role_to_accesskit_role(role: AccessibilityRole) -> AccessibilityNodeRole {
    match role {
        AccessibilityRole::Button => AccessibilityNodeRole::Button,
        AccessibilityRole::Link => AccessibilityNodeRole::Link,
        AccessibilityRole::Image => AccessibilityNodeRole::Image,
        AccessibilityRole::Text => AccessibilityNodeRole::Label,
        AccessibilityRole::Header => AccessibilityNodeRole::Header,
        AccessibilityRole::Footer => AccessibilityNodeRole::Footer,
        AccessibilityRole::Navigation => AccessibilityNodeRole::Navigation,
        AccessibilityRole::Main => AccessibilityNodeRole::Main,
        AccessibilityRole::Search => AccessibilityNodeRole::Search,
        AccessibilityRole::Article => AccessibilityNodeRole::Article,
        AccessibilityRole::Section => AccessibilityNodeRole::Section,
        AccessibilityRole::List => AccessibilityNodeRole::List,
        AccessibilityRole::ListItem => AccessibilityNodeRole::ListItem,
        AccessibilityRole::Checkbox => AccessibilityNodeRole::CheckBox,
        AccessibilityRole::RadioButton => AccessibilityNodeRole::RadioButton,
        AccessibilityRole::Switch => AccessibilityNodeRole::Switch,
        AccessibilityRole::Slider => AccessibilityNodeRole::Slider,
        AccessibilityRole::ProgressBar => AccessibilityNodeRole::ProgressIndicator,
        AccessibilityRole::Tab => AccessibilityNodeRole::Tab,
        AccessibilityRole::TabList => AccessibilityNodeRole::TabList,
        AccessibilityRole::TabPanel => AccessibilityNodeRole::TabPanel,
        AccessibilityRole::Menu => AccessibilityNodeRole::Menu,
        AccessibilityRole::MenuItem => AccessibilityNodeRole::MenuItem,
        AccessibilityRole::MenuBar => AccessibilityNodeRole::MenuBar,
        AccessibilityRole::MenuItemCheckbox => AccessibilityNodeRole::MenuItemCheckBox,
        AccessibilityRole::MenuItemRadio => AccessibilityNodeRole::MenuItemRadio,
        AccessibilityRole::Combobox => AccessibilityNodeRole::ComboBox,
        AccessibilityRole::Option => AccessibilityNodeRole::ListBoxOption,
        AccessibilityRole::Group => AccessibilityNodeRole::Group,
        _ => panic!("hydrolysis accessibility role variant is not implemented"),
    }
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_pointer_action(
    renderer: &mut HydrolysisRenderer,
    action: AccessibilityAction,
    point: vello::kurbo::Point,
    env: &Environment,
) -> bool {
    let x = point.x as f32;
    let y = point.y as f32;
    match action {
        AccessibilityAction::Click => {
            let mut changed = renderer.handle_pointer_down(x, y, PointerButton::Primary, env);
            changed |= renderer.handle_pointer_up(x, y, PointerButton::Primary, env);
            changed
        }
        AccessibilityAction::Focus => {
            renderer.handle_pointer_down(x, y, PointerButton::Primary, env)
        }
        _ => panic!(
            "hydrolysis accessibility pointer target does not support action {:?}",
            action
        ),
    }
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_scroll_action(
    handle: &ScrollHandle,
    axis: ScrollAxis,
    action: AccessibilityAction,
) -> bool {
    match action {
        AccessibilityAction::ScrollLeft => match axis {
            ScrollAxis::Horizontal | ScrollAxis::All => handle.apply_scroll_delta(1.0, 0.0, true),
            ScrollAxis::Vertical => false,
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        },
        AccessibilityAction::ScrollRight => match axis {
            ScrollAxis::Horizontal | ScrollAxis::All => handle.apply_scroll_delta(-1.0, 0.0, true),
            ScrollAxis::Vertical => false,
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        },
        AccessibilityAction::ScrollUp => match axis {
            ScrollAxis::Vertical | ScrollAxis::All => handle.apply_scroll_delta(0.0, 1.0, true),
            ScrollAxis::Horizontal => false,
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        },
        AccessibilityAction::ScrollDown => match axis {
            ScrollAxis::Vertical | ScrollAxis::All => handle.apply_scroll_delta(0.0, -1.0, true),
            ScrollAxis::Horizontal => false,
            _ => panic!("scroll axis variant is not supported by hydrolysis"),
        },
        _ => false,
    }
}

#[cfg(feature = "accessibility")]
fn slider_step_for_range(range: RangeInclusive<f64>) -> f64 {
    let start = *range.start();
    let end = *range.end();
    let span = end - start;
    assert!(
        !(span <= 0.0),
        "hydrolysis accessibility slider requires range start < end"
    );
    span / 100.0
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_slider_action(
    value: &nami::Binding<f64>,
    start: f64,
    end: f64,
    step: f64,
    action: AccessibilityAction,
    data: Option<AccessibilityActionData>,
) -> bool {
    if matches!(action, AccessibilityAction::Focus) {
        return true;
    }
    assert!(
        !(step <= 0.0),
        "hydrolysis accessibility slider requires positive step"
    );
    let previous = value.get().clamp(start, end);
    let next = match action {
        AccessibilityAction::Increment => (previous + step).min(end),
        AccessibilityAction::Decrement => (previous - step).max(start),
        AccessibilityAction::SetValue => match data {
            Some(AccessibilityActionData::NumericValue(target)) => target.clamp(start, end),
            _ => {
                panic!("hydrolysis accessibility slider SetValue requires NumericValue data")
            }
        },
        _ => panic!(
            "hydrolysis accessibility slider does not support action {:?}",
            action
        ),
    };
    if (next - previous).abs() <= f64::EPSILON {
        return false;
    }
    value.set(next);
    true
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_stepper_action(
    value: &nami::Binding<i32>,
    step: &nami::Computed<i32>,
    start: i32,
    end: i32,
    action: AccessibilityAction,
    data: Option<AccessibilityActionData>,
) -> bool {
    if matches!(action, AccessibilityAction::Focus) {
        return true;
    }
    let step_value = step.get();
    assert!(
        !(step_value <= 0),
        "hydrolysis accessibility stepper requires positive step"
    );
    let previous = value.get().clamp(start, end);
    let next = match action {
        AccessibilityAction::Increment => previous.saturating_add(step_value).min(end),
        AccessibilityAction::Decrement => previous.saturating_sub(step_value).max(start),
        AccessibilityAction::SetValue => match data {
            Some(AccessibilityActionData::NumericValue(target)) => {
                let rounded = target.round() as i32;
                rounded.clamp(start, end)
            }
            Some(AccessibilityActionData::Value(ref text)) => {
                let parsed = text
                    .parse::<i32>()
                    .expect("hydrolysis accessibility stepper SetValue text must parse as i32");
                parsed.clamp(start, end)
            }
            _ => panic!("hydrolysis accessibility stepper SetValue requires numeric data"),
        },
        _ => panic!(
            "hydrolysis accessibility stepper does not support action {:?}",
            action
        ),
    };
    if next == previous {
        return false;
    }
    value.set(next);
    true
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_text_field_action(
    renderer: &mut HydrolysisRenderer,
    node_id: AccessibilityNodeId,
    value: &nami::Binding<StyledStr>,
    line_limit: Option<usize>,
    action: AccessibilityAction,
    data: Option<AccessibilityActionData>,
) -> bool {
    match action {
        AccessibilityAction::Click | AccessibilityAction::Focus => {
            renderer.focus_text_input_for_accessibility_node(node_id)
        }
        AccessibilityAction::SetValue => {
            let Some(AccessibilityActionData::Value(text)) = data else {
                panic!("hydrolysis accessibility text field SetValue requires Value data");
            };
            let normalized = normalized_insert_text(text.as_ref(), line_limit);
            assert!(
                !(exceeds_line_limit(normalized.as_str(), line_limit)),
                "hydrolysis accessibility text field SetValue exceeds line_limit {:?}",
                line_limit
            );
            value.set(StyledStr::plain(normalized));
            true
        }
        AccessibilityAction::ReplaceSelectedText => {
            let Some(AccessibilityActionData::Value(text)) = data else {
                panic!(
                    "hydrolysis accessibility text field ReplaceSelectedText requires Value data"
                );
            };
            let normalized = normalized_insert_text(text.as_ref(), line_limit);
            let mut plain = value.get().to_plain().to_string();
            assert!(
                !(!apply_text_insert(&mut plain, normalized.as_str(), line_limit)),
                "hydrolysis accessibility text field ReplaceSelectedText exceeds line_limit {:?}",
                line_limit
            );
            value.set(StyledStr::plain(plain));
            true
        }
        _ => panic!(
            "hydrolysis accessibility text field does not support action {:?}",
            action
        ),
    }
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_secure_field_action(
    renderer: &mut HydrolysisRenderer,
    node_id: AccessibilityNodeId,
    value: &nami::Binding<FormSecure>,
    action: AccessibilityAction,
    data: Option<AccessibilityActionData>,
) -> bool {
    match action {
        AccessibilityAction::Click | AccessibilityAction::Focus => {
            renderer.focus_text_input_for_accessibility_node(node_id)
        }
        AccessibilityAction::SetValue => {
            let Some(AccessibilityActionData::Value(text)) = data else {
                panic!("hydrolysis accessibility secure field SetValue requires Value data");
            };
            let normalized = normalized_insert_text(text.as_ref(), Some(1));
            let mut next = FormSecure::default();
            next.set(normalized);
            value.set(next);
            true
        }
        AccessibilityAction::ReplaceSelectedText => {
            let Some(AccessibilityActionData::Value(text)) = data else {
                panic!(
                    "hydrolysis accessibility secure field ReplaceSelectedText requires Value data"
                );
            };
            let mut plain = value.get().expose().to_owned();
            assert!(
                !(!apply_text_insert(&mut plain, text.as_ref(), Some(1))),
                "hydrolysis accessibility secure field ReplaceSelectedText exceeds line_limit 1"
            );
            let mut next = FormSecure::default();
            next.set(plain);
            value.set(next);
            true
        }
        _ => panic!(
            "hydrolysis accessibility secure field does not support action {:?}",
            action
        ),
    }
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_picker_cycle_action(
    selection: &nami::Binding<waterui_core::id::Id>,
    ids: &[waterui_core::id::Id],
    action: AccessibilityAction,
) -> bool {
    match action {
        AccessibilityAction::Click => {
            assert!(
                !(ids.is_empty()),
                "hydrolysis accessibility picker cycle requires non-empty options"
            );
            let current = selection.get();
            let index = ids.iter().position(|id| *id == current).unwrap_or_else(|| {
                panic!("hydrolysis accessibility picker selection is not present in options")
            });
            let next = ids[(index + 1) % ids.len()];
            if next == current {
                return false;
            }
            selection.set(next);
            true
        }
        AccessibilityAction::Focus => true,
        _ => panic!(
            "hydrolysis accessibility picker cycle does not support action {:?}",
            action
        ),
    }
}

#[cfg(feature = "accessibility")]
fn handle_accessibility_picker_select_action(
    selection: &nami::Binding<waterui_core::id::Id>,
    target: waterui_core::id::Id,
    action: AccessibilityAction,
) -> bool {
    match action {
        AccessibilityAction::Click | AccessibilityAction::Focus => {
            if selection.get() == target {
                return false;
            }
            selection.set(target);
            true
        }
        _ => panic!(
            "hydrolysis accessibility picker select does not support action {:?}",
            action
        ),
    }
}

fn gesture_group_identity(view: &AnyView) -> usize {
    gesture_group_identity_with_budget(view, 64)
}

fn gesture_group_identity_with_budget(view: &AnyView, remaining: usize) -> usize {
    assert!(
        !(remaining == 0),
        "hydrolysis gesture group identity extraction exceeded recursion budget for {}",
        view.name()
    );
    if let Some(metadata) = view.downcast_ref::<Metadata<Environment>>() {
        return gesture_group_identity_with_budget(&metadata.content, remaining - 1);
    }
    if let Some(content) = passthrough_content(view) {
        return gesture_group_identity_with_budget(content, remaining - 1);
    }
    view.stable_ptr() as usize
}

fn passthrough_content<'a>(view: &'a AnyView) -> Option<&'a AnyView> {
    macro_rules! passthrough_metadata_content {
        ($($ty:ty),+ $(,)?) => {
            $(
                if let Some(metadata) = view.downcast_ref::<Metadata<$ty>>() {
                    return Some(&metadata.content);
                }
            )+
        };
    }

    macro_rules! passthrough_ignorable_metadata_content {
        ($($ty:ty),+ $(,)?) => {
            $(
                if let Some(metadata) = view.downcast_ref::<IgnorableMetadata<$ty>>() {
                    return Some(&metadata.content);
                }
            )+
        };
    }

    passthrough_metadata_content!(
        Environment,
        Retain,
        Opacity,
        AppliedFilter,
        Scale,
        Rotation,
        Offset,
        ClipShape,
        Border,
        Shadow,
        Focused,
        Hittable,
        GestureObserver,
        LifeCycleHook,
        OnEvent,
        Secure,
        StandardDynamicRange,
        HighDynamicRange,
        Cursor,
        IgnoreSafeArea,
        ContextMenu,
        ResolvedContextMenu,
        Draggable,
        DropDestination,
        Background
    );
    passthrough_ignorable_metadata_content!(
        MaterialBackground,
        AccessibilityLabel,
        AccessibilityRole,
        AccessibilityHidden,
        AccessibilityChildren,
        AccessibilityState
    );

    None
}

fn effective_stretch_axis(view: &AnyView) -> StretchAxis {
    if let Some(content) = passthrough_content(view) {
        return effective_stretch_axis(content);
    }
    view.stretch_axis()
}

fn is_layout_terminal(view: &AnyView) -> bool {
    if view.downcast_ref::<Str>().is_some() || view.downcast_ref::<Divider>().is_some() {
        return true;
    }

    macro_rules! check_native_terminal {
        ($ty:ty) => {
            if view.downcast_ref::<$ty>().is_some() {
                return true;
            }
        };
    }
    hydro_native_view_types!(check_native_terminal);
    false
}

pub(crate) fn normalize_view_for_render(view: AnyView, env: &Environment) -> AnyView {
    normalize_layout_view(view, env)
}

fn normalize_layout_view(view: AnyView, env: &Environment) -> AnyView {
    normalize_layout_view_with_budget(view, env, 64)
}

fn local_state_body_env(env: &Environment) -> Environment {
    env.get::<LocalStateScope>()
        .map_or_else(|| env.clone(), |scope| env.extending(scope.reset()))
}

fn local_state_child_env(env: &Environment, index: usize) -> Environment {
    env.get::<LocalStateScope>()
        .map_or_else(|| env.clone(), |scope| env.extending(scope.child(index)))
}

fn local_state_overlay_env(base: &Environment, current: &Environment) -> Environment {
    current
        .get::<LocalStateScope>()
        .map_or_else(|| base.clone(), |scope| base.extending(scope.clone()))
}

fn normalize_layout_view_with_budget(
    view: AnyView,
    env: &Environment,
    remaining: usize,
) -> AnyView {
    assert!(
        !(remaining == 0),
        "hydrolysis layout normalization exceeded recursion budget for {}",
        view.name()
    );
    let next_remaining = remaining - 1;
    let mut view = view;

    if view.is::<Metadata<Environment>>() {
        let Metadata { content, value } = *view
            .downcast::<Metadata<Environment>>()
            .expect("layout normalization failed to downcast Metadata<Environment>");
        let scoped_env = local_state_overlay_env(&value, env);
        let normalized_content =
            normalize_layout_view_with_budget(content, &scoped_env, next_remaining);
        return AnyView::new(Metadata {
            content: normalized_content,
            value,
        });
    }
    macro_rules! normalize_passthrough_metadata {
        ($($ty:ty),+ $(,)?) => {
            $(
                if view.is::<Metadata<$ty>>() {
                    let Metadata { content, value } = *view
                        .downcast::<Metadata<$ty>>()
                        .expect("layout normalization failed to downcast metadata");
                    let normalized_content =
                        normalize_layout_view_with_budget(content, env, next_remaining);
                    return AnyView::new(Metadata {
                        content: normalized_content,
                        value,
                    });
                }
            )+
        };
    }
    macro_rules! normalize_passthrough_ignorable_metadata {
        ($($ty:ty),+ $(,)?) => {
            $(
                if view.is::<IgnorableMetadata<$ty>>() {
                    let IgnorableMetadata { content, value } = *view
                        .downcast::<IgnorableMetadata<$ty>>()
                        .expect("layout normalization failed to downcast ignorable metadata");
                    let normalized_content =
                        normalize_layout_view_with_budget(content, env, next_remaining);
                    return AnyView::new(IgnorableMetadata {
                        content: normalized_content,
                        value,
                    });
                }
            )+
        };
    }

    normalize_passthrough_metadata!(
        Retain,
        Opacity,
        AppliedFilter,
        Scale,
        Rotation,
        Offset,
        ClipShape,
        Border,
        Shadow,
        Focused,
        Hittable,
        GestureObserver,
        LifeCycleHook,
        OnEvent,
        Secure,
        StandardDynamicRange,
        HighDynamicRange,
        Cursor,
        IgnoreSafeArea,
        ContextMenu,
        ResolvedContextMenu,
        Draggable,
        DropDestination,
        Background
    );
    normalize_passthrough_ignorable_metadata!(
        MaterialBackground,
        AccessibilityLabel,
        AccessibilityRole,
        AccessibilityHidden,
        AccessibilityChildren,
        AccessibilityState
    );

    if view.is::<Native<FixedContainer>>() {
        let native = *view
            .downcast::<Native<FixedContainer>>()
            .expect("layout normalization failed to downcast Native<FixedContainer>");
        let (layout, children) = native.into_inner().into_inner();
        let mut normalized_children = Vec::with_capacity(children.len());
        for (index, child) in children.into_iter().enumerate() {
            let child_env = local_state_child_env(env, index);
            normalized_children.push(normalize_layout_view_with_budget(
                child,
                &child_env,
                next_remaining,
            ));
        }
        return AnyView::new(Native::new(FixedContainer::from_parts(
            layout,
            normalized_children,
        )));
    }

    if view.is::<Native<LazyContainer>>() {
        return view;
    }

    if view.is::<Native<ScrollView>>() {
        let native = *view
            .downcast::<Native<ScrollView>>()
            .expect("layout normalization failed to downcast Native<ScrollView>");
        let (axis, content) = native.into_inner().into_inner();
        let child_env = local_state_child_env(env, 0);
        let normalized_content =
            normalize_layout_view_with_budget(content, &child_env, next_remaining);
        return AnyView::new(Native::new(ScrollView::new(axis, normalized_content)));
    }

    if is_layout_terminal(&view) {
        return view;
    }

    let body_env = local_state_body_env(env);
    view = AnyView::new(view.body(&body_env));
    normalize_layout_view_with_budget(view, &body_env, next_remaining)
}

fn estimate_layout_intrinsic<'a>(
    layout: &dyn Layout,
    children: impl IntoIterator<Item = &'a AnyView>,
    state: &mut HydroState,
    env: &Environment,
) -> LayoutSize {
    let mut subviews = Vec::new();
    for child in children {
        subviews.push(HydroSubview::from_view(child, state, env));
    }
    let refs: Vec<&dyn SubView> = subviews.iter().map(|view| view as &dyn SubView).collect();
    layout.size_that_fits(ProposalSize::UNSPECIFIED, &refs)
}

fn resolved_color_to_peniko(color: ResolvedColor) -> vello::peniko::Color {
    let srgb = color.to_srgb_with_headroom();
    vello::peniko::Color::new([srgb.red, srgb.green, srgb.blue, color.opacity])
}

fn resolved_gradient_to_brush(
    gradient: &ResolvedGradient,
    bounds: vello::kurbo::Rect,
) -> vello::peniko::Brush {
    let mut stops: Vec<vello::peniko::ColorStop> =
        gradient.stops.iter().map(to_peniko_stop).collect();

    let brush = match gradient.gradient_type {
        GradientType::Linear => {
            let start = resolved_point_to_kurbo(gradient.start_point, bounds);
            let end = resolved_point_to_kurbo(gradient.end_point, bounds);
            vello::peniko::Gradient::new_linear(start, end).with_stops(&*stops)
        }
        GradientType::Radial => {
            let center = resolved_point_to_kurbo(gradient.start_point, bounds);
            let radius_scale = bounds.width().min(bounds.height()) as f32;
            let start_radius = gradient.start_value * radius_scale;
            let end_radius = gradient.end_value * radius_scale;
            vello::peniko::Gradient::new_two_point_radial(center, start_radius, center, end_radius)
                .with_stops(&*stops)
        }
        GradientType::Angular => {
            let sweep = gradient.end_value - gradient.start_value;
            let sweep_fraction = f64::from(sweep) / TAU;
            if sweep_fraction < 1.0 {
                let last_color = stops
                    .last()
                    .expect("resolved gradient must contain at least one stop")
                    .color;
                for stop in &mut stops {
                    stop.offset = (f64::from(stop.offset) * sweep_fraction) as f32;
                }
                stops.push(vello::peniko::ColorStop {
                    offset: sweep_fraction as f32,
                    color: last_color,
                });
                stops.push(vello::peniko::ColorStop {
                    offset: 1.0,
                    color: last_color,
                });
            }
            let center = resolved_point_to_kurbo(gradient.start_point, bounds);
            vello::peniko::Gradient::new_sweep(center, gradient.start_value, 0.0)
                .with_stops(&*stops)
        }
        GradientType::Mesh => {
            panic!("resolved mesh gradient must not be dispatched through ResolvedGradient")
        }
    };

    vello::peniko::Brush::Gradient(brush)
}

fn resolved_point_to_kurbo(point: [f32; 2], bounds: vello::kurbo::Rect) -> vello::kurbo::Point {
    vello::kurbo::Point::new(
        f64::from(point[0]) * bounds.width(),
        f64::from(point[1]) * bounds.height(),
    )
}

fn to_peniko_stop(stop: &ResolvedGradientStop) -> vello::peniko::ColorStop {
    vello::peniko::ColorStop {
        offset: stop.position,
        color: resolved_color_to_peniko(stop.color).into(),
    }
}

fn resolved_shape_to_path(
    shape: &ResolvedShape,
    bounds: vello::kurbo::Rect,
) -> vello::kurbo::BezPath {
    path_commands_to_path(&shape.commands, bounds)
}

fn path_commands_to_path(
    commands: &[PathCommand],
    bounds: vello::kurbo::Rect,
) -> vello::kurbo::BezPath {
    let width = bounds.width();
    let height = bounds.height();
    let mut path = vello::kurbo::BezPath::new();
    let mut has_current = false;

    for command in commands {
        match command {
            PathCommand::MoveTo { x, y } => {
                path.move_to(vello::kurbo::Point::new(
                    f64::from(*x) * width,
                    f64::from(*y) * height,
                ));
                has_current = true;
            }
            PathCommand::LineTo { x, y } => {
                assert!(
                    !(!has_current),
                    "PathCommand::LineTo requires an active current point"
                );
                path.line_to(vello::kurbo::Point::new(
                    f64::from(*x) * width,
                    f64::from(*y) * height,
                ));
            }
            PathCommand::QuadTo { cx, cy, x, y } => {
                assert!(
                    !(!has_current),
                    "PathCommand::QuadTo requires an active current point"
                );
                path.quad_to(
                    vello::kurbo::Point::new(f64::from(*cx) * width, f64::from(*cy) * height),
                    vello::kurbo::Point::new(f64::from(*x) * width, f64::from(*y) * height),
                );
            }
            PathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => {
                assert!(
                    !(!has_current),
                    "PathCommand::CubicTo requires an active current point"
                );
                path.curve_to(
                    vello::kurbo::Point::new(f64::from(*c1x) * width, f64::from(*c1y) * height),
                    vello::kurbo::Point::new(f64::from(*c2x) * width, f64::from(*c2y) * height),
                    vello::kurbo::Point::new(f64::from(*x) * width, f64::from(*y) * height),
                );
            }
            PathCommand::Arc {
                cx,
                cy,
                rx,
                ry,
                start,
                sweep,
            } => {
                let center_x = f64::from(*cx) * width;
                let center_y = f64::from(*cy) * height;
                let radius_x = f64::from(*rx) * width;
                let radius_y = f64::from(*ry) * height;
                let start = f64::from(*start);
                let step = f64::from(*sweep) / 32.0;

                let start_point = vello::kurbo::Point::new(
                    center_x + radius_x * start.cos(),
                    center_y + radius_y * start.sin(),
                );
                if has_current {
                    path.line_to(start_point);
                } else {
                    path.move_to(start_point);
                    has_current = true;
                }

                let mut angle = start;
                for _ in 0..32 {
                    angle += step;
                    path.line_to(vello::kurbo::Point::new(
                        center_x + radius_x * angle.cos(),
                        center_y + radius_y * angle.sin(),
                    ));
                }
            }
            PathCommand::Close => {
                path.close_path();
                has_current = false;
            }
        }
    }

    path
}

fn anchor_point(bounds: vello::kurbo::Rect, anchor: waterui::style::Anchor) -> vello::kurbo::Point {
    vello::kurbo::Point::new(
        bounds.x0 + bounds.width() * f64::from(anchor.x),
        bounds.y0 + bounds.height() * f64::from(anchor.y),
    )
}

fn resolved_color_to_rgba8(color: ResolvedColor) -> [u8; 4] {
    let srgb = color.to_srgb_with_headroom();
    [
        (srgb.red.clamp(0.0, 1.0) * 255.0).round() as u8,
        (srgb.green.clamp(0.0, 1.0) * 255.0).round() as u8,
        (srgb.blue.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

fn rgba8_to_peniko(color: [u8; 4]) -> vello::peniko::Color {
    vello::peniko::Color::new([
        f32::from(color[0]) / 255.0,
        f32::from(color[1]) / 255.0,
        f32::from(color[2]) / 255.0,
        f32::from(color[3]) / 255.0,
    ])
}

fn parley_font_weight(weight: TextFontWeight) -> parley::FontWeight {
    let value = match weight {
        TextFontWeight::Thin => 100.0,
        TextFontWeight::UltraLight => 200.0,
        TextFontWeight::Light => 300.0,
        TextFontWeight::Normal => 400.0,
        TextFontWeight::Medium => 500.0,
        TextFontWeight::SemiBold => 600.0,
        TextFontWeight::Bold => 700.0,
        TextFontWeight::UltraBold => 800.0,
        TextFontWeight::Black => 900.0,
    };
    parley::FontWeight::new(value)
}

fn parley_alignment(alignment: HorizontalAlignment) -> parley::Alignment {
    if alignment == HorizontalAlignment::Leading {
        parley::Alignment::Start
    } else if alignment == HorizontalAlignment::Trailing {
        parley::Alignment::End
    } else {
        parley::Alignment::Center
    }
}

fn transformed_rect(
    transform: vello::kurbo::Affine,
    rect: vello::kurbo::Rect,
) -> vello::kurbo::Rect {
    let points = [
        transform * vello::kurbo::Point::new(rect.x0, rect.y0),
        transform * vello::kurbo::Point::new(rect.x1, rect.y0),
        transform * vello::kurbo::Point::new(rect.x0, rect.y1),
        transform * vello::kurbo::Point::new(rect.x1, rect.y1),
    ];
    let min_x = points
        .iter()
        .fold(f64::INFINITY, |acc, point| acc.min(point.x));
    let min_y = points
        .iter()
        .fold(f64::INFINITY, |acc, point| acc.min(point.y));
    let max_x = points
        .iter()
        .fold(f64::NEG_INFINITY, |acc, point| acc.max(point.x));
    let max_y = points
        .iter()
        .fold(f64::NEG_INFINITY, |acc, point| acc.max(point.y));
    vello::kurbo::Rect::new(min_x, min_y, max_x, max_y)
}

fn lerp_f32(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

fn lerp_f64(from: f64, to: f64, t: f32) -> f64 {
    from + (to - from) * f64::from(t)
}

fn lerp_color(from: [f32; 4], to: [f32; 4], t: f32) -> vello::peniko::Color {
    vello::peniko::Color::new([
        lerp_f32(from[0], to[0], t),
        lerp_f32(from[1], to[1], t),
        lerp_f32(from[2], to[2], t),
        lerp_f32(from[3], to[3], t),
    ])
}

fn inset_rect(rect: vello::kurbo::Rect, dx: f64, dy: f64) -> vello::kurbo::Rect {
    vello::kurbo::Rect::new(
        rect.x0 + dx,
        rect.y0 + dy,
        (rect.x1 - dx).max(rect.x0 + dx),
        (rect.y1 - dy).max(rect.y0 + dy),
    )
}

fn circle_arc_path(
    center: vello::kurbo::Point,
    radius: f64,
    start_angle: f64,
    sweep: f64,
) -> vello::kurbo::BezPath {
    let mut path = vello::kurbo::BezPath::new();
    if sweep == 0.0 {
        return path;
    }
    let segments = 64usize;
    let step = sweep / segments as f64;
    let mut angle = start_angle;
    path.move_to(vello::kurbo::Point::new(
        center.x + radius * angle.cos(),
        center.y + radius * angle.sin(),
    ));
    for _ in 0..segments {
        angle += step;
        path.line_to(vello::kurbo::Point::new(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        ));
    }
    path
}

fn text_cursor_area_from_layout(
    text_bounds: vello::kurbo::Rect,
    layout: &parley::Layout<[u8; 4]>,
    max_lines: Option<usize>,
    fallback_line_height: f64,
) -> vello::kurbo::Rect {
    let left = text_bounds.x0;
    let right = text_bounds.x1.max(left + 1.0);
    let top = text_bounds.y0;
    let bottom = text_bounds.y1.max(top + 1.0);
    if layout.is_empty() {
        let available_height = bottom - top;
        let line_height = fallback_line_height.clamp(1.0, available_height);
        let y0 = top + ((available_height - line_height) * 0.5).max(0.0);
        return vello::kurbo::Rect::new(left, y0, left + 1.0, (y0 + line_height).min(bottom));
    }

    let mut caret_x = left;
    let mut caret_top = top;
    let mut caret_bottom = (top + 1.0).min(bottom);
    for (index, line) in layout.lines().enumerate() {
        if max_lines.is_some_and(|limit| index >= limit) {
            break;
        }
        let metrics = line.metrics();
        caret_x = left + f64::from(metrics.offset + metrics.advance);
        let line_top = f64::from(metrics.baseline - metrics.ascent);
        let line_bottom = f64::from(metrics.baseline + metrics.descent);
        caret_top = top + line_top;
        caret_bottom = top + line_bottom.max(line_top + 1.0);
    }
    let caret_x = caret_x.clamp(left, right - 1.0);
    let caret_top = caret_top.clamp(top, bottom - 1.0);
    let caret_bottom = caret_bottom.clamp(caret_top + 1.0, bottom);
    vello::kurbo::Rect::new(caret_x, caret_top, caret_x + 1.0, caret_bottom)
}

fn draw_button_chrome(
    scene: &mut vello::Scene,
    transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
    style: ButtonStyle,
) {
    let rounded = vello::kurbo::RoundedRect::from_rect(bounds, 6.0);
    match style {
        ButtonStyle::Automatic => {
            scene.fill(
                vello::peniko::Fill::NonZero,
                transform,
                vello::peniko::Color::new([0.93, 0.93, 0.95, 1.0]),
                None,
                &rounded,
            );
            scene.stroke(
                &vello::kurbo::Stroke::new(1.0),
                transform,
                vello::peniko::Color::new([0.74, 0.74, 0.78, 1.0]),
                None,
                &rounded,
            );
        }
        ButtonStyle::Bordered => {
            scene.fill(
                vello::peniko::Fill::NonZero,
                transform,
                vello::peniko::Color::new([1.0, 1.0, 1.0, 1.0]),
                None,
                &rounded,
            );
            scene.stroke(
                &vello::kurbo::Stroke::new(1.0),
                transform,
                vello::peniko::Color::new([0.73, 0.74, 0.78, 1.0]),
                None,
                &rounded,
            );
        }
        ButtonStyle::BorderedProminent => {
            scene.fill(
                vello::peniko::Fill::NonZero,
                transform,
                vello::peniko::Color::new([0.2, 0.45, 0.9, 1.0]),
                None,
                &rounded,
            );
            scene.stroke(
                &vello::kurbo::Stroke::new(1.0),
                transform,
                vello::peniko::Color::new([0.15, 0.35, 0.75, 1.0]),
                None,
                &rounded,
            );
        }
        ButtonStyle::Link => {
            let underline_bounds = inset_rect(bounds, BUTTON_LINK_HORIZONTAL_PADDING, 0.0);
            let underline_y = (bounds.y1 - BUTTON_LINK_UNDERLINE_BOTTOM_INSET).max(bounds.y0);
            let underline = vello::kurbo::Line::new(
                vello::kurbo::Point::new(underline_bounds.x0, underline_y),
                vello::kurbo::Point::new(underline_bounds.x1, underline_y),
            );
            scene.stroke(
                &vello::kurbo::Stroke::new(BUTTON_LINK_UNDERLINE_THICKNESS),
                transform,
                vello::peniko::Color::new([0.2, 0.45, 0.9, 1.0]),
                None,
                &underline,
            );
        }
        ButtonStyle::Plain | ButtonStyle::Borderless => {}
        _ => panic!("hydrolysis ButtonStyle variant is not implemented"),
    }
}

fn draw_toggle_switch(
    scene: &mut vello::Scene,
    transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
    progress: f32,
    track_color: vello::peniko::Color,
) {
    let thumb_center_x = lerp_f64(bounds.x0 + 15.0, bounds.x1 - 15.0, progress);
    let thumb_center = vello::kurbo::Point::new(thumb_center_x, bounds.y0 + bounds.height() / 2.0);
    let track = vello::kurbo::RoundedRect::from_rect(bounds, 15.5);
    let thumb = vello::kurbo::Circle::new(thumb_center, 13.0);
    scene.fill(
        vello::peniko::Fill::NonZero,
        transform,
        track_color,
        None,
        &track,
    );
    scene.stroke(
        &vello::kurbo::Stroke::new(1.0),
        transform,
        vello::peniko::Color::new([0.72, 0.74, 0.78, 1.0]),
        None,
        &track,
    );
    scene.fill(
        vello::peniko::Fill::NonZero,
        transform,
        vello::peniko::Color::WHITE,
        None,
        &thumb,
    );
    scene.stroke(
        &vello::kurbo::Stroke::new(1.0),
        transform,
        vello::peniko::Color::new([0.76, 0.78, 0.82, 1.0]),
        None,
        &thumb,
    );
}

fn draw_toggle_checkbox(
    scene: &mut vello::Scene,
    transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
    progress: f32,
) {
    let rounded = vello::kurbo::RoundedRect::from_rect(bounds, 4.0);
    let fill = lerp_color([1.0, 1.0, 1.0, 1.0], [0.2, 0.45, 0.9, 1.0], progress);
    scene.fill(
        vello::peniko::Fill::NonZero,
        transform,
        fill,
        None,
        &rounded,
    );
    scene.stroke(
        &vello::kurbo::Stroke::new(1.0),
        transform,
        lerp_color([0.7, 0.72, 0.76, 1.0], [0.16, 0.34, 0.72, 1.0], progress),
        None,
        &rounded,
    );
    if progress <= 0.0 {
        return;
    }
    let check = vello::kurbo::BezPath::from_vec(vec![
        vello::kurbo::PathEl::MoveTo(vello::kurbo::Point::new(
            bounds.x0 + bounds.width() * 0.25,
            bounds.y0 + bounds.height() * 0.55,
        )),
        vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(
            bounds.x0 + bounds.width() * 0.45,
            bounds.y0 + bounds.height() * 0.75,
        )),
        vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(
            bounds.x0 + bounds.width() * 0.78,
            bounds.y0 + bounds.height() * 0.3,
        )),
    ]);
    scene.stroke(
        &vello::kurbo::Stroke::new(2.0),
        transform,
        vello::peniko::Color::new([1.0, 1.0, 1.0, progress]),
        None,
        &check,
    );
}

fn draw_picker_indicator(
    scene: &mut vello::Scene,
    transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
) {
    let center_x = bounds.x1 - PICKER_HORIZONTAL_INSET - PICKER_INDICATOR_SPACE * 0.5;
    let center_y = bounds.y0 + bounds.height() * 0.5;
    let chevron = vello::kurbo::BezPath::from_vec(vec![
        vello::kurbo::PathEl::MoveTo(vello::kurbo::Point::new(center_x - 4.0, center_y - 2.0)),
        vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(center_x, center_y + 2.0)),
        vello::kurbo::PathEl::LineTo(vello::kurbo::Point::new(center_x + 4.0, center_y - 2.0)),
    ]);
    scene.stroke(
        &vello::kurbo::Stroke::new(1.5),
        transform,
        vello::peniko::Color::new([0.45, 0.47, 0.52, 1.0]),
        None,
        &chevron,
    );
}

fn draw_navigation_transition(
    scene: &mut vello::Scene,
    transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
    style: NavigationTransition,
    direction: NavigationTransitionDirection,
    progress: f64,
    from_scene: &vello::Scene,
    to_scene: &vello::Scene,
) {
    let width = bounds.width();
    match style {
        NavigationTransition::PushPop => {
            let (from_x, to_x) = match direction {
                NavigationTransitionDirection::Push => (
                    -width * NAVIGATION_PUSHPOP_PARALLAX_FACTOR * progress,
                    width * (1.0 - progress),
                ),
                NavigationTransitionDirection::Pop => (
                    width * progress,
                    -width * NAVIGATION_PUSHPOP_PARALLAX_FACTOR * (1.0 - progress),
                ),
            };
            append_scene_with_alpha(scene, transform, bounds, from_scene, from_x, 1.0);
            append_scene_with_alpha(scene, transform, bounds, to_scene, to_x, 1.0);
        }
        NavigationTransition::Fade => {
            append_scene_with_alpha(
                scene,
                transform,
                bounds,
                from_scene,
                0.0,
                1.0f32 - progress as f32,
            );
            append_scene_with_alpha(scene, transform, bounds, to_scene, 0.0, progress as f32);
        }
        NavigationTransition::None => {
            scene.append(to_scene, Some(transform));
        }
    }
}

fn append_scene_with_alpha(
    scene: &mut vello::Scene,
    transform: vello::kurbo::Affine,
    clip_bounds: vello::kurbo::Rect,
    content: &vello::Scene,
    offset_x: f64,
    alpha: f32,
) {
    if alpha <= 0.0 {
        return;
    }
    scene.push_layer(
        vello::peniko::Fill::NonZero,
        vello::peniko::BlendMode::default(),
        alpha,
        transform,
        &clip_bounds,
    );
    scene.append(
        content,
        Some(transform * vello::kurbo::Affine::translate((offset_x, 0.0))),
    );
    scene.pop_layer();
}

fn draw_input_field(
    scene: &mut vello::Scene,
    transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
) {
    let rounded = vello::kurbo::RoundedRect::from_rect(bounds, 6.0);
    scene.fill(
        vello::peniko::Fill::NonZero,
        transform,
        vello::peniko::Color::new([1.0, 1.0, 1.0, 1.0]),
        None,
        &rounded,
    );
    scene.stroke(
        &vello::kurbo::Stroke::new(1.0),
        transform,
        vello::peniko::Color::new([0.75, 0.75, 0.78, 1.0]),
        None,
        &rounded,
    );
}

fn draw_stepper_button(
    scene: &mut vello::Scene,
    transform: vello::kurbo::Affine,
    bounds: vello::kurbo::Rect,
) {
    scene.fill(
        vello::peniko::Fill::NonZero,
        transform,
        vello::peniko::Color::new([0.93, 0.93, 0.95, 1.0]),
        None,
        &vello::kurbo::RoundedRect::from_rect(bounds, 6.0),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui::gesture::{DragGesture, GestureObserver, MagnificationGesture};
    use waterui::{Binding, SignalExt as _, ViewExt as _};
    use waterui_canvas::Canvas;

    #[test]
    fn measure_layout_dimensions_collects_alignment_keys_from_wrapper_layouts() {
        let env = Environment::default();
        let child = normalize_layout_view(
            AnyView::new(().size(20.0, 10.0).horizontal_alignment_guide(
                HorizontalAlignment::Leading,
                |dimensions: &ViewDimensions| dimensions.size.width * 0.5,
            )),
            &env,
        );
        let layout = VStackLayout {
            alignment: HorizontalAlignment::Leading,
            spacing: 0.0,
        };
        let mut state = HydroState::default();
        let dimensions = measure_layout_dimensions(
            &layout,
            [&child],
            ProposalSize::UNSPECIFIED,
            &mut state,
            &env,
        );

        assert_eq!(
            dimensions.explicit_horizontal(HorizontalAlignment::Leading),
            Some(10.0)
        );
    }

    #[test]
    fn gesture_group_identity_collapses_nested_gesture_observers_on_same_view() {
        let view = AnyView::new(Metadata::new(
            Metadata::new(
                ().size(20.0, 10.0),
                GestureObserver::new(DragGesture::new(8.0)).action(|| {}),
            ),
            GestureObserver::new(MagnificationGesture::new(1.0)).action(|| {}),
        ));
        let outer = view
            .downcast_ref::<Metadata<GestureObserver>>()
            .expect("expected outer gesture observer metadata");
        let inner = outer
            .content
            .downcast_ref::<Metadata<GestureObserver>>()
            .expect("expected inner gesture observer metadata");

        assert_eq!(
            gesture_group_identity(&outer.content),
            gesture_group_identity(&inner.content)
        );
    }

    #[test]
    fn renderer_magnification_targets_outer_observer_in_stacked_gesture_chain() {
        use std::{cell::Cell, rc::Rc};
        use waterui_core::Metadata;

        let offset = Binding::f32(0.0);
        let scale = Binding::f32(1.0);
        let drag_hits = Rc::new(Cell::new(0u32));
        let magnify_hits = Rc::new(Cell::new(0u32));
        let view = {
            let canvas = Canvas::with_signal(offset.zip(&scale), |_ctx, (_offset, _scale)| {})
                .size(120.0, 120.0);
            let canvas = {
                let drag_hits = Rc::clone(&drag_hits);
                Metadata::new(
                    canvas,
                    GestureObserver::new(DragGesture::new(0.0)).action(move || {
                        drag_hits.set(drag_hits.get() + 1);
                    }),
                )
            };
            {
                let magnify_hits = Rc::clone(&magnify_hits);
                Metadata::new(
                    canvas,
                    GestureObserver::new(MagnificationGesture::new(1.0)).action(move || {
                        magnify_hits.set(magnify_hits.get() + 1);
                    }),
                )
            }
        };

        let mut platform =
            crate::platform::OffscreenWindow::new(160, 160, wgpu::TextureFormat::Rgba8Unorm);
        let mut renderer = {
            let surface = platform.surface();
            HydrolysisRenderer::new(surface.device())
        };
        let env = Environment::new();
        let bounds = vello::kurbo::Rect::new(0.0, 0.0, 160.0, 160.0);
        let surface = platform.surface();
        renderer.set_frame_resources(surface.device(), surface.queue());
        renderer.reset_scene();
        renderer.begin_rebuild_frame();
        renderer.dispatch(view, &env, bounds);
        renderer.finish_rebuild_frame();

        let point = vello::kurbo::Point::new(60.0, 60.0);
        let debug_targets = renderer.gesture_engine.debug_targets_at(point);
        assert_eq!(
            debug_targets.len(),
            2,
            "expected stacked drag+magnification gesture targets at point, got {:?}",
            debug_targets
        );
        assert_eq!(debug_targets[0].2, debug_targets[1].2);

        assert!(renderer.apply_magnification_gesture(60.0, 60.0, 1.2, &env));
        assert_eq!(drag_hits.get(), 0);
        assert_eq!(magnify_hits.get(), 3);
    }
}
