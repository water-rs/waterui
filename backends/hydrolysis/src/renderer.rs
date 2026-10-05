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
pub mod accessibility;
mod bindings;
mod effects;
mod frame;
mod frame_work;
#[cfg(feature = "frame-profile")]
mod gpu_profile;
mod identity;
mod input;
mod interaction_layers;
mod lifecycle;
mod metadata;
mod native_measure;
mod navigation;
pub mod recording;
mod render;
mod retained;

mod signals;
#[cfg(test)]
pub mod tests;
mod tree;
mod views;

pub use effects::*;
pub use frame::*;
pub use frame_work::FrameWorkCounters;
#[cfg(feature = "frame-profile")]
pub use gpu_profile::GpuFrameProfiler;
#[cfg(feature = "frame-profile")]
pub use gpu_profile::{FrameStageTimes, GpuIdentity};
pub use identity::*;
pub use native_measure::*;
#[cfg(test)]
pub use recording::assert_well_formed_image;
pub use recording::{Glyph, GlyphRun, Recording, working_color};
pub use retained::*;
pub use tree::*;
pub use views::*;
pub use waterui_backend_core::frame_signals::FrameSignals;

#[cfg(feature = "accessibility")]
use accessibility::{
    ACCESSIBILITY_ROOT_NODE_ID, AccessibilityBuilder, AccessibilityNameFromContents,
    ScopedAccessibilityIdentifier,
};
use core::num::NonZeroUsize;
use core::time::Duration;
pub use input::*;
pub use interaction_layers::*;
pub use lifecycle::lazy;
pub use lifecycle::*;
pub use navigation::*;
pub use render::FrameRenderTarget;
pub use render::HydrolysisRenderTarget;
pub use render::WidgetRenderContext;
pub use render::*;
pub use render::{
    anchor_point, circle_arc_path, estimate_layout_intrinsic, gesture_group_identity,
    normalize_layout_view, normalize_view_for_render, path_commands_to_path,
    resolved_morph_shape_to_path, resolved_shape_to_path, transformed_rect,
};
use rustc_hash::FxHashSet;
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

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
use waterkit_clipboard::Clipboard;
use waterui::ViewExt;
use waterui::accessibility::{
    AccessibilityChildren, AccessibilityHidden, AccessibilityIdentifier, AccessibilityLabel,
    AccessibilityRole, AccessibilityState, AccessibilityStateSignal, AccessibilityValue,
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
use waterui_controls::button::{Button, ButtonConfig};
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
use waterui_graphics::color::Color;
use waterui_graphics::draw::{Paint, WorkingColor};
use waterui_graphics::gpu::RedrawHandle;
use waterui_graphics::{ExternalFrameView, FilteredView, GpuContentView, Gradient, SceneView};

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
    RadioIndicatorState, RadioSelectionMotion, TextCaretMotion, TextContextMenuMetrics,
};
use crate::gesture::GestureEngine;
use crate::platform::{
    KeyCode, Modifiers, PointerButton, PointerKind, TextInputPurpose, TextInputState, TouchPhase,
};
#[cfg(feature = "accessibility")]
use crate::scroll::ScrollHandle;
use crate::time::Instant;
use crate::widgets::inset_rect;

const OPACITY_ANIMATION_KEY: usize = 0x0100_0001;
const SCALE_X_ANIMATION_KEY: usize = 0x0100_0002;
const SCALE_Y_ANIMATION_KEY: usize = 0x0100_0003;
const ROTATION_ANIMATION_KEY: usize = 0x0100_0004;
const OFFSET_X_ANIMATION_KEY: usize = 0x0100_0005;
const OFFSET_Y_ANIMATION_KEY: usize = 0x0100_0006;
const MORPH_PROGRESS_ANIMATION_KEY: usize = 0x0100_0007;

#[cfg(feature = "accessibility")]
pub use accessibility::{
    AccessibilityActionTarget, AccessibilityActivation, NodePlacement,
    ScopedAccessibilitySemantics, accessibility_container_child_environment, slider_step_for_range,
};
pub use input::{
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
    /// Input targets, selection slots and pre-edit live here — `pub(crate)`
    /// because the runner's editing session and headless tests read them
    /// from outside the renderer module tree.
    pub(crate) text_editing: TextEditingState,
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
    /// Their releases must be swallowed too — `wl_keyboard` delivers the release
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

// The state members are engine internals (gesture/hit-test/executor state)
// with nothing useful to print; a name-only non-exhaustive form keeps the
// impl honest.
impl std::fmt::Debug for SemanticCore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SemanticCore").finish_non_exhaustive()
    }
}

/// Core hydrolysis renderer state: a [`SemanticCore`] plus the GPU-side scene,
/// compositor and surface state the layout/encode pass needs.
pub struct HydrolysisRenderer {
    core: SemanticCore,
    /// The widget theme the runtime's style supplies to layout and encode.
    /// Never installed into the environment: build and patch cannot reach it.
    theme: Rc<dyn crate::engine::WidgetTheme>,
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
    /// Wake target supplied when this renderer itself is hosted inside
    /// another GPU host. Renderer-owned redraws use it to wake the parent host
    /// without polling frames.
    host_redraw_handle: Option<RedrawHandle>,
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
    frame_filtered_count: u32,
    /// Per-frame applied-filter telemetry the render thread's `EngineEffect`
    /// calls accumulate into; reset before each `Engine::render` and read back
    /// into the `frame_applied_filter_*` fields after it.
    applied_filter_metrics: Arc<crate::renderer::effects::AppliedFilterMetrics>,
    frame_applied_filter_count: u32,
    frame_applied_filter_effect: Duration,
    /// In-flight navigation scene captures (screenshots of outgoing pages
    /// during a transition).
    navigation_captures: Vec<NavigationSceneCapture>,
    /// CPU stage times accumulated by `flush_window_tree`, plus the GPU spans
    /// the render pass resolves; drained per pump by `take_frame_stage_times`.
    /// `pub(crate)` so the runner's readback timing can add its stage in.
    #[cfg(feature = "frame-profile")]
    pub(crate) frame_stage_times: FrameStageTimes,
    /// Digest of the last layout pass's placed bounds; the frame-profile
    /// example compares it across runs to prove a change left layout output
    /// byte-identical.
    #[cfg(feature = "frame-profile")]
    last_layout_signature: Option<u64>,
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
    pub(crate) const fn set_window_id(&mut self, window_id: WindowId) {
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

    pub(crate) fn new(frame_instant: Instant, family_resolution: FontFamilyResolution) -> Self {
        Self {
            state: HydroState::new(family_resolution),
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
    pub(crate) const fn modifiers(&self) -> Modifiers {
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
    /// A renderer drawing with `theme`. `family_resolution` decides whether a
    /// named font family the collection cannot resolve is skipped
    /// ([`FontFamilyResolution::Lenient`], applications) or fails the shape
    /// naming it ([`FontFamilyResolution::Strict`], test hosts).
    #[must_use]
    pub fn new(
        theme: Rc<dyn crate::engine::WidgetTheme>,
        family_resolution: FontFamilyResolution,
    ) -> Self {
        let frame_instant = Instant::now();
        Self {
            core: SemanticCore::new(frame_instant, family_resolution),
            theme,
            scene: Recording::new(),
            transient_scene: None,
            compositor: Compositor::default(),
            window_bounds: kurbo::Rect::ZERO,
            window_root_transform: kurbo::Affine::IDENTITY,
            host_redraw_handle: None,

            cherenkov_windows: rustc_hash::FxHashMap::default(),
            engine_next: None,
            frame_clip_layers: 0,
            frame_max_clip_depth: 0,
            frame_filtered_count: 0,
            applied_filter_metrics: Arc::default(),
            frame_applied_filter_count: 0,
            frame_applied_filter_effect: Duration::ZERO,
            navigation_captures: Vec::new(),
            #[cfg(feature = "frame-profile")]
            frame_stage_times: FrameStageTimes::default(),
            #[cfg(feature = "frame-profile")]
            last_layout_signature: None,
        }
    }

    /// The widget theme layout and encode draw with. Returned as a cloned
    /// `Rc` so callers may hold it across further `&mut self` calls.
    pub(crate) fn theme(&self) -> Rc<dyn crate::engine::WidgetTheme> {
        Rc::clone(&self.theme)
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
pub use render::RenderContext;
pub use render::{HydrolysisTextContextMenuMode, HydrolysisWindowOrigin};
