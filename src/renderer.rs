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

#[cfg(feature = "accessibility")]
mod accessibility;
mod bindings;
mod color;
mod effects;
mod frame;
#[cfg(feature = "frame-profile")]
mod gpu_profile;
mod identity;
mod input;
mod interaction_layers;
mod lifecycle;
mod metadata;
mod migration_counters;
mod native_measure;
mod navigation;
mod recording;
mod render;
mod retained;
mod scene_ingest;
mod signals;
#[cfg(test)]
pub(crate) mod tests;
mod tree;
mod views;

pub(crate) use effects::*;
pub(crate) use frame::*;
#[cfg(feature = "frame-profile")]
pub(crate) use gpu_profile::GpuFrameProfiler;
#[cfg(feature = "frame-profile")]
pub use gpu_profile::{FrameStageTimes, GpuIdentity};
pub(crate) use identity::*;
pub use migration_counters::MigrationCounters;
pub(crate) use native_measure::*;
pub use recording::{Recording, VelloDrawContext};
pub(crate) use retained::*;
pub(crate) use scene_ingest::CheckedScene2D;
#[cfg(test)]
pub(crate) use scene_ingest::assert_well_formed_image;
pub(crate) use tree::*;
pub(crate) use views::*;
pub(crate) use waterui_backend_core::frame_signals::FrameSignals;

#[cfg(feature = "accessibility")]
use accessibility::*;
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
use rustc_hash::FxHashSet;
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};

#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, ActionData as AccessibilityActionData,
    ActionRequest as AccessibilityActionRequest, Node as AccessibilityNode,
    NodeId as AccessibilityNodeId, Rect as AccessibilityRect, Role as AccessibilityNodeRole,
    TextDirection as AccessibilityTextDirection, Toggled as AccessibilityToggled,
    TreeId as AccessibilityTreeId, TreeInfo as AccessibilityTree,
    TreeUpdate as AccessibilityTreeUpdate,
};
use executor_core::spawn_local;
use nami::{Binding, Signal};
use std::sync::Arc;
use waterkit_clipboard::Clipboard;
use waterui::ViewExt;
#[cfg(feature = "accessibility")]
use waterui::accessibility::AccessibilityValue;
use waterui::accessibility::{
    AccessibilityChildren, AccessibilityHidden, AccessibilityIdentifier, AccessibilityLabel,
    AccessibilityRole, AccessibilityState, AccessibilityStateSignal,
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
use waterui::metadata::anchored_overlay::AnchoredOverlay;
use waterui::metadata::context_menu::{ContextMenu, ResolvedContextMenu};
use waterui::metadata::secure::{HighDynamicRange, Secure, StandardDynamicRange};
use waterui::navigation::tab::{NativeTabStyle, TabsLayout};
use waterui::navigation::{
    CustomNavigationController, NavigationController, NavigationSplitLayout, NavigationStack,
    NavigationToolbarPlacement, NavigationTransaction, NavigationTransitionDestination,
    NavigationTransitionSource, NavigationView,
};
use waterui::style::{Offset, Rotation, Scale, Shadow};
use waterui::theme;
use waterui::widget::Divider;
use waterui::window::{Window, WindowState, WindowStyle};
use waterui_controls::button::{Button, ButtonConfig, ButtonStyle};
use waterui_controls::label::Label as SemanticLabel;
use waterui_controls::menu::{CommandRole, ResolvedCommand, ResolvedMenu, ResolvedMenuItem};
use waterui_controls::slider::SliderConfig;
use waterui_controls::stepper::StepperConfig;
use waterui_controls::text_field::{ResolvedTextFieldConfig, TextField};
use waterui_controls::toggle::ToggleConfig;
use waterui_core::dynamic::{Dynamic, DynamicInitialContent};
use waterui_core::event::{Event, HoverEvent, LifeCycle, LifeCycleHook, OnEvent};
use waterui_core::handler::{AnyViewBuilder, BoxedAction, SharedAction};
use waterui_core::key::{KeyHandling, KeyPress, OnKeyPress};
use waterui_core::layout::{
    HorizontalAlignment, Layout, PlacedSubview, Point as LayoutPoint, ProposalSize,
    Rect as LayoutRect, Size as LayoutSize, StretchAxis, SubView, VerticalAlignment,
    ViewDimensions,
};
#[cfg(feature = "accessibility")]
use waterui_core::metadata::MetadataKey;
use waterui_core::view::Hook;
use waterui_core::views::Views;
use waterui_core::{
    AnyView, Environment, IgnorableMetadata, Metadata, Native, Retain, Str, View, impl_extractor,
};
use waterui_form::picker::PickerConfig;
use waterui_form::picker::color::ColorPickerConfig;
use waterui_form::picker::date::DatePickerConfig;
use waterui_form::secure::{Secure as FormSecure, SecureFieldConfig};
use waterui_graphics::color::{Color, ResolvedColor};

use shaderloom::WgslModuleCache;
use waterui_graphics::filter_view::{EffectContext, EffectInput, EffectOutput};
use waterui_graphics::gpu_surface::GestureState;
use waterui_graphics::view_effect::{
    ViewEffectContext, ViewEffectErased, ViewEffectInput, ViewEffectOutput,
};
use waterui_graphics::{
    AppliedFilter, GpuContext, GpuFrame, GpuSurface, GradientType, PointerState, RedrawHandle,
    ResolvedGradient, ResolvedGradientStop, SceneEngine, SceneView, SharedSceneRenderer,
};

use waterui_icon::SystemIcon;
use waterui_layout::container::{FixedContainer, LazyContainer};
use waterui_layout::safe_area::IgnoreSafeArea;
#[cfg(feature = "accessibility")]
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
    LegacyRenderer, LegacyRendererOptions, RadioIndicatorState, RadioSelectionMotion,
    TextCaretMotion, TextContextMenuMetrics, legacy_init_threads,
};
use crate::gesture::GestureEngine;
use crate::platform::{
    KeyCode, Modifiers, PointerButton, PointerKind, TextInputPurpose, TextInputState, TouchPhase,
};
#[cfg(feature = "accessibility")]
use crate::scroll::ScrollHandle;
use crate::time::Instant;
use crate::widgets::inset_rect;

#[derive(Clone, Copy)]
pub(crate) struct DynamicRangePreference(pub(crate) bool);

const OPACITY_ANIMATION_KEY: usize = 0x0100_0001;
const SCALE_X_ANIMATION_KEY: usize = 0x0100_0002;
const SCALE_Y_ANIMATION_KEY: usize = 0x0100_0003;
const ROTATION_ANIMATION_KEY: usize = 0x0100_0004;
const OFFSET_X_ANIMATION_KEY: usize = 0x0100_0005;
const OFFSET_Y_ANIMATION_KEY: usize = 0x0100_0006;
const MORPH_PROGRESS_ANIMATION_KEY: usize = 0x0100_0007;

#[cfg(feature = "accessibility")]
pub(crate) use accessibility::{
    AccessibilityActionTarget, AccessibilityActivation, accessibility_container_child_environment,
    slider_step_for_range,
};
pub(crate) use input::{
    TextInputModel, TextInputTargetRegistration, TextSelectionSlot, clamp_to_char_boundary,
    text_editing,
};

/// The GPU-free dispatch core: everything the retained view tree's build,
/// patch and accessibility emission need, without a device, a scene, or a
/// style.
///
/// [`HydrolysisRenderer`] owns one and dereferences to it, so rendering code
/// reaches this state transparently while [`crate::SemanticRuntime`] can hold
/// just the core and prove by type that build, patch and semantic emission
/// can never touch the GPU. Methods that only use this state live in
/// `impl SemanticCore` blocks; methods that also touch rendering state stay
/// on `HydrolysisRenderer`.
///
/// `pub` only because `HydrolysisRenderer` dereferences to it — every member
/// is crate-internal.
pub struct SemanticCore {
    state: HydroState,
    hit_test: HitTestState,
    gesture_engine: GestureEngine,
    gesture_group_ids: BTreeMap<usize, usize>,
    next_gesture_group_id: usize,
    text_editing: TextEditingState,
    popup_menu: PopupMenuState,
    /// The identity the runner gave the window this core renders — menu-chord
    /// dispatch scopes mounted `Menu` sources by it (water-rs/hydrolysis#247).
    /// Renderers no runner claimed — embedded GPU hosts, direct test
    /// construction — keep [`WindowId::Orphan`].
    window_id: WindowId,
    render_depth: usize,
    /// The retained nodes whose subtrees are currently flushing, innermost
    /// last — the ancestry chain input registration reads to tell a gesture
    /// registered inside a view from one attached to the view itself. The
    /// accessibility builder keeps its own copy for semantic-key identity; the
    /// input side additionally gets the pushes that mark where a view begins
    /// (a sub-view's root) without entering the a11y chain.
    owner_stack: Vec<RetainedIdentity>,
    /// Frame triggers shared with reactive closures; see [`FrameSignals`].
    signals: FrameSignals,
    lifecycle: LifecycleState,
    animation_controller: AnimationController,
    frame_instant: Instant,
    /// Retained render-tree GPU surfaces (`GpuSurfaceNode`-owned runtimes),
    /// registered at node build time. Polled by
    /// [`HydrolysisRenderer::poll_gpu_surface_redraw_handles`] for off-thread
    /// redraw requests; dead entries (node dropped on a Dynamic swap) are pruned
    /// by strong count.
    node_gpu_surfaces: Vec<Rc<RefCell<EmbeddedGpuSurfaceRuntime>>>,
    /// Retained render-tree view effects, registered for exact async setup when
    /// Hydrolysis itself is hosted inside a `GpuSurface`.
    node_view_effects: Vec<Weak<RefCell<ViewEffectRuntime>>>,
    /// Retained render-tree applied filters (`AppliedFilterNode`-owned runtimes),
    /// registered at node build time. Refreshed by
    /// [`HydrolysisRenderer::refresh_active_applied_filters`] on redraw-only
    /// frames; dead entries are pruned by strong count.
    node_applied_filters: Vec<Rc<RefCell<AppliedFilterRuntime>>>,
    pub(crate) lazy: LazyState,
    pub(crate) navigation: NavigationState,
    #[cfg(feature = "accessibility")]
    accessibility: AccessibilityBuilder,
    /// The persistent window render tree (`tree::RenderNode`), built on a structural
    /// rebuild and re-flushed each frame. `None` before the first build.
    render_tree: Option<RenderNode>,
    /// Set when a widget-owned [`RetainedSubview`] applied a structural patch
    /// (a `Dynamic` swap or a collection membership reconcile) during a flush.
    /// The subview patch runs mid-flush — after the window pump's structural
    /// bookkeeping window — so the flag carries the change into the next refresh
    /// frame, which then runs the full animation-slot / measurement-cache prune
    /// cycle for the dropped subtrees.
    subview_structural_change: bool,
    /// The `OnKeyPress` scope chain enclosing the view currently flushing,
    /// innermost first. The `WrapperEffect::OnKeyPress` arm pushes one link
    /// around its child's walk (and the semantic walk does the same), so a
    /// target registered anywhere inside snapshots the whole ancestor chain —
    /// the order an unconsumed key bubbles through. As a parent-linked list
    /// a snapshot is a single `Rc` clone rather than a per-target `Vec` copy.
    key_handler_stack: Option<Rc<KeyHandlerNode>>,
    /// Physical codes of key presses the IME consumed while it owned input.
    /// Their releases must be swallowed too — wl_keyboard delivers the release
    /// of an IME-consumed press in a later batch, after the commit that ended
    /// the composition.
    ime_swallowed_codes: Vec<keyboard_types::Code>,
    /// `true` when this core is the semantic walk — it emits no pointer
    /// machinery, so focus liveness may fall back to the semantic focus
    /// link. The semantic runner marks it at construction; a rendered
    /// runtime never does, so a rendered frame with no pointer targets
    /// still applies the rendered-runtime rule.
    #[cfg(feature = "accessibility")]
    semantic_walk: bool,
}

/// Core hydrolysis renderer state: a [`SemanticCore`] plus the GPU-side scene,
/// compositor and surface state the layout/encode pass needs.
pub struct HydrolysisRenderer {
    core: SemanticCore,
    /// The widget theme the runtime's style supplies to layout and encode.
    /// Never installed into the environment: build and patch cannot reach it.
    theme: Rc<dyn crate::engine::WidgetTheme>,
    legacy_renderer: LegacyRenderer,
    scene: Recording,
    transient_scene: Option<Recording>,
    compositor: Compositor,
    window_bounds: kurbo::Rect,
    /// The transform the window's root content is flushed under: logical layout
    /// units onto the target's physical pixel grid. Stored alongside
    /// [`Self::window_bounds`] because the pair is what says where the viewport
    /// is in device pixels, which is what
    /// [`HydrolysisRenderer::push_gpu_surface_layer`] tests a full-window GPU
    /// surface against.
    window_root_transform: kurbo::Affine,
    /// Wake target supplied when this renderer itself is hosted by a
    /// `GpuSurface`. Async setup and renderer-owned redraws from nested surfaces
    /// use it to wake the parent host without polling frames.
    host_redraw_handle: Option<RedrawHandle>,
    /// Module cache for shaders that embedded GPU surfaces, view effects and
    /// filters assemble at runtime. Shared so identical WGSL compiles once.
    shader_cache: Arc<WgslModuleCache>,
    /// The scene renderer embedded GPU surfaces share, for the same reason: its
    /// pipelines belong to the device rather than to any one scene.
    scene_renderer: Arc<SharedSceneRenderer>,
    /// Engine window surfaces by GPU-context id: the `TextureTarget` surface,
    /// the stable mounts under it and the resource registrations its content
    /// names. Entries whose device was reported lost are pruned at the next
    /// presented frame.
    cherenkov_windows: rustc_hash::FxHashMap<u64, crate::renderer::render::CherenkovWindow>,
    /// The engine's frame scheduling answer from the last presented frame.
    /// The pump follows it: `Next::Idle` means no animation is running and
    /// the display link may sleep.
    engine_next: Option<cherenkov::Next>,
    frame_clip_layers: u32,
    frame_max_clip_depth: u32,
    /// Whether this frame's window pass was handed straight to a GPU surface
    /// instead of being composited. Recorded by
    /// [`HydrolysisRenderer::render_scene_to_surface`] as it decides, so the
    /// frame report says what happened rather than what was eligible.
    frame_direct_gpu_surfaces: u32,
    frame_applied_filter_count: u32,
    frame_applied_filter_capture: Duration,
    frame_applied_filter_effect: Duration,
    /// The per-frame atlas every filtered subtree is captured through.
    subtree_captures: SubtreeCaptures,
    /// Bounded cache of CPU-rasterized blurred shadow silhouettes — shared
    /// `Blob`s let vello keep the atlas texture across frames; see
    /// `metadata::BlurredSilhouetteCache`.
    blurred_silhouettes: metadata::BlurredSilhouetteCache,
    navigation_captures: Vec<NavigationSceneCapture>,
    /// CPU stage times accumulated by `flush_window_tree`, plus the GPU spans
    /// the render pass resolves; drained per pump by `take_frame_stage_times`.
    /// `pub(crate)` so the runner's readback timing can add its stage in.
    #[cfg(feature = "frame-profile")]
    pub(crate) frame_stage_times: FrameStageTimes,
    /// Timestamp-query state for the frame's GPU spans; `None` when the device
    /// lacks `TIMESTAMP_QUERY` — GPU stages then report absent, never a guess.
    #[cfg(feature = "frame-profile")]
    gpu_profiler: Option<GpuFrameProfiler>,
    /// Digest of the last layout pass's placed bounds; the frame-profile
    /// example compares it across runs to prove a change left layout output
    /// byte-identical.
    #[cfg(feature = "frame-profile")]
    last_layout_signature: Option<u64>,
    /// The device-wide pipeline cache persisted between launches; `None` where
    /// the adapter or platform has no persistent cache (see
    /// `pipeline_cache.rs`). Held here rather than by the `LegacyRenderer` it
    /// was handed to so pooled renderers can share it and the renderer can
    /// write it back once early frames have run the pipelines.
    #[cfg(hydrolysis_pipeline_cache)]
    pipeline_cache_store: Option<Arc<crate::pipeline_cache::Store>>,
    /// Presented frames since this renderer was created; the pipeline cache is
    /// written back once the first frames have run the pipelines it serves.
    #[cfg(hydrolysis_pipeline_cache)]
    presented_frames: u32,
}

#[cfg(hydrolysis_pipeline_cache)]
impl Drop for HydrolysisRenderer {
    /// A clean exit writes back whatever the pipelines compiled since the
    /// last persist; a kill mid-write still leaves a usable file because the
    /// write renames over the previous one.
    fn drop(&mut self) {
        if let Some(store) = &self.pipeline_cache_store {
            store.persist();
        }
    }
}

impl core::ops::Deref for HydrolysisRenderer {
    type Target = SemanticCore;
    fn deref(&self) -> &SemanticCore {
        &self.core
    }
}

impl core::ops::DerefMut for HydrolysisRenderer {
    fn deref_mut(&mut self) -> &mut SemanticCore {
        &mut self.core
    }
}

const HIT_TEST_ALPHA_THRESHOLD: f32 = 0.01;

const TEXT_SELECTION_MULTI_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const TEXT_SELECTION_MULTI_CLICK_DISTANCE: f64 = 6.0;
const TEXT_CONTEXT_MENU_WINDOW_TITLE: &str = "";

impl SemanticCore {
    /// Assigns the identity the runner minted for the window this core
    /// renders — called once when the runner creates the window
    /// (water-rs/hydrolysis#247).
    pub(crate) fn set_window_id(&mut self, window_id: WindowId) {
        self.window_id = window_id;
    }

    /// Enters an `OnKeyPress` scope for the subtree now flushing; targets
    /// registered inside snapshot it as their bubble chain.
    pub(crate) fn push_key_handler_scope(
        &mut self,
        env: Environment,
        handler: Rc<RefCell<OnKeyPress>>,
    ) {
        self.key_handler_stack = Some(Rc::new(KeyHandlerNode {
            scope: KeyHandlerScope { env, handler },
            parent: self.key_handler_stack.take(),
        }));
    }

    /// Leaves the innermost `OnKeyPress` scope.
    pub(crate) fn pop_key_handler_scope(&mut self) {
        self.key_handler_stack = self
            .key_handler_stack
            .take()
            .and_then(|link| link.parent.clone());
    }

    pub(crate) fn new(frame_instant: Instant) -> Self {
        Self {
            state: HydroState::default(),
            hit_test: HitTestState::default(),
            gesture_engine: GestureEngine::default(),
            gesture_group_ids: BTreeMap::new(),
            next_gesture_group_id: 0,
            text_editing: TextEditingState::default(),
            popup_menu: PopupMenuState::default(),
            window_id: WindowId::Orphan,
            render_depth: 0,
            owner_stack: Vec::new(),
            signals: FrameSignals::new(frame_instant),
            lifecycle: LifecycleState::default(),
            animation_controller: AnimationController::default(),
            frame_instant,
            node_gpu_surfaces: Vec::new(),
            node_view_effects: Vec::new(),
            node_applied_filters: Vec::new(),
            lazy: LazyState::default(),
            navigation: NavigationState::default(),
            #[cfg(feature = "accessibility")]
            accessibility: AccessibilityBuilder::default(),
            render_tree: None,
            subview_structural_change: false,
            key_handler_stack: None,
            ime_swallowed_codes: Vec::new(),
            #[cfg(feature = "accessibility")]
            semantic_walk: false,
        }
    }

    /// Runs `f` with accessibility-node registration suppressed. For a control
    /// whose own node already carries an internal sub-view's semantics (a merged
    /// label, a numeric value): emitting that sub-view inside this scope keeps it
    /// visual-only, so the control stays a single accessibility node instead of
    /// double-exposing its label as a separate node.
    #[cfg(feature = "accessibility")]
    pub(crate) fn with_suppressed_accessibility<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        #[cfg(feature = "accessibility")]
        self.push_accessibility_suppression();
        let result = f(self);
        #[cfg(feature = "accessibility")]
        self.pop_accessibility_suppression();
        result
    }

    /// The modifier snapshot the window last reported — pointer targets read
    /// it at commit time because pointer events carry no modifier state of
    /// their own (toggle and Shift range selection).
    pub(crate) fn modifiers(&self) -> Modifiers {
        self.hit_test.modifiers
    }

    /// Records that a widget-owned sub-view applied a structural patch during
    /// this flush, and requests a refresh frame so the change's prune cycle (and
    /// any layout it invalidated in ancestors) settles on the next pump.
    pub(crate) fn note_subview_structural_change(&mut self) {
        self.subview_structural_change = true;
        self.signals.request_refresh();
    }

    /// Consumes the carried subview structural-change flag; the refresh pump
    /// folds it into this frame's `structural_change` so the prune cycle runs.
    pub(crate) fn take_subview_structural_change(&mut self) -> bool {
        core::mem::take(&mut self.subview_structural_change)
    }
}

impl HydrolysisRenderer {
    /// A renderer for `device`, which `adapter` produced, drawing with `theme`.
    ///
    /// The adapter is not a formality: the scene renderer that embedded GPU
    /// surfaces share is built for the engine `adapter` can actually run, and
    /// an adapter without indirect execution aborts inside wgpu rather than
    /// degrading when asked to run the classic compute pipeline.
    #[must_use]
    pub fn new(
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        theme: Rc<dyn crate::engine::WidgetTheme>,
    ) -> Self {
        Self::new_with_options(
            adapter,
            device,
            theme,
            LegacyRendererOptions {
                use_cpu: false,
                num_init_threads: legacy_init_threads(adapter.get_info().backend),
                pipeline_cache: None,
                // Filled from the window viewport at `set_window_viewport`;
                // until then `None` sizes the bump buffers per render target.
                buffer_sizes: None,
            },
        )
    }

    /// As [`Self::new`], with the window renderer's Vello options spelled out.
    #[must_use]
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::arc_with_non_send_sync,
            reason = "`SharedSceneRenderer` and `WgslModuleCache` own wgpu handles, which the WebGPU backend makes neither `Send` nor `Sync` because they are JS objects. The renderer is shared by reference count on every target and is `Send + Sync` on all of them but this one, so the storage type is `Arc` everywhere rather than `Rc` here and `Arc` elsewhere."
        )
    )]
    pub fn new_with_options(
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        theme: Rc<dyn crate::engine::WidgetTheme>,
        options: LegacyRendererOptions,
    ) -> Self {
        #[cfg(hydrolysis_pipeline_cache)]
        let mut options = options;
        #[cfg(hydrolysis_pipeline_cache)]
        let pipeline_cache_store = if options.pipeline_cache.is_some() {
            None
        } else {
            crate::pipeline_cache::open(device, adapter).inspect(|store| {
                options.pipeline_cache = Some(store.cache());
            })
        };
        let legacy_renderer =
            LegacyRenderer::new(device, options).expect("failed to create hydrolysis renderer");
        let frame_instant = Instant::now();
        Self {
            core: SemanticCore::new(frame_instant),
            theme,
            legacy_renderer,
            scene: Recording::new(),
            transient_scene: None,
            compositor: Compositor::default(),
            window_bounds: kurbo::Rect::ZERO,
            window_root_transform: kurbo::Affine::IDENTITY,
            host_redraw_handle: None,
            shader_cache: Arc::new(WgslModuleCache::new()),
            scene_renderer: Arc::new(SharedSceneRenderer::new(SceneEngine::for_adapter(adapter))),
            cherenkov_windows: rustc_hash::FxHashMap::default(),
            engine_next: None,
            frame_clip_layers: 0,
            frame_max_clip_depth: 0,
            frame_direct_gpu_surfaces: 0,
            frame_applied_filter_count: 0,
            frame_applied_filter_capture: Duration::ZERO,
            frame_applied_filter_effect: Duration::ZERO,
            subtree_captures: SubtreeCaptures::default(),
            blurred_silhouettes: metadata::BlurredSilhouetteCache::new(),
            navigation_captures: Vec::new(),
            #[cfg(feature = "frame-profile")]
            frame_stage_times: FrameStageTimes::default(),
            #[cfg(feature = "frame-profile")]
            gpu_profiler: GpuFrameProfiler::new(device),
            #[cfg(feature = "frame-profile")]
            last_layout_signature: None,
            #[cfg(hydrolysis_pipeline_cache)]
            pipeline_cache_store,
            #[cfg(hydrolysis_pipeline_cache)]
            presented_frames: 0,
        }
    }

    /// The widget theme layout and encode draw with. Returned as a cloned
    /// `Rc` so callers may hold it across further `&mut self` calls.
    pub(crate) fn theme(&self) -> Rc<dyn crate::engine::WidgetTheme> {
        Rc::clone(&self.theme)
    }

    /// No pipeline cache exists on targets without the persistent store; the
    /// call site stays unconditioned.
    #[cfg(not(hydrolysis_pipeline_cache))]
    #[allow(dead_code)]
    pub(crate) fn note_frame_presented(&mut self) {}

    /// Record a presented frame. The pipeline cache is written back once the
    /// second frame has let the driver compile the pipelines the first frames
    /// needed — early enough that a killed process still leaves the cache
    /// behind for the next launch.
    #[cfg(hydrolysis_pipeline_cache)]
    pub(crate) fn note_frame_presented(&mut self) {
        self.presented_frames += 1;
        if self.presented_frames == 2
            && let Some(store) = &self.pipeline_cache_store
        {
            store.persist_on_worker();
        }
    }

    /// The pipeline cache pooled legacy renderers should compile against, when
    /// this device has one.
    #[cfg(hydrolysis_pipeline_cache)]
    pub(crate) fn pipeline_cache(&self) -> Option<wgpu::PipelineCache> {
        self.pipeline_cache_store
            .as_ref()
            .map(|store| store.cache())
    }

    /// No pipeline cache exists on targets without the persistent store; the
    /// call sites stay unconditioned.
    #[cfg(not(hydrolysis_pipeline_cache))]
    #[allow(dead_code)]
    pub(crate) fn pipeline_cache(&self) -> Option<wgpu::PipelineCache> {
        None
    }

    /// How many blurred shadow silhouettes have been CPU-rasterized so far —
    /// the hook a test uses to prove a moved caster hits the silhouette cache
    /// instead of re-rasterizing.
    #[cfg(test)]
    pub(crate) fn blurred_silhouette_rasterizations(&self) -> usize {
        self.blurred_silhouettes.rasterizations
    }

    /// Runs `f` with accessibility-node registration suppressed. For a control
    /// whose own node already carries an internal sub-view's semantics (a merged
    /// label, a numeric value): flushing that sub-view inside this scope keeps it
    /// visual-only, so the control stays a single accessibility node instead of
    /// double-exposing its label as a separate node. Compiles to a plain call
    /// without the `accessibility` feature, so call sites need no gating.
    ///
    /// This shadows [`SemanticCore::with_suppressed_accessibility`] so rendered
    /// callers flush through a `&mut HydrolysisRenderer`; the core method is the
    /// one the semantic runtime's emission walk uses.
    pub(crate) fn with_suppressed_accessibility<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        #[cfg(feature = "accessibility")]
        self.push_accessibility_suppression();
        let result = f(self);
        #[cfg(feature = "accessibility")]
        self.pop_accessibility_suppression();
        result
    }
}

pub use render::HydroState;
use render::HydroSubview;
pub use render::RenderContext;
pub(crate) use render::{HydrolysisTextContextMenuMode, HydrolysisWindowOrigin};