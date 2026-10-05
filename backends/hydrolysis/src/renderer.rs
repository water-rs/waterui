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
mod material;
mod metadata;
mod mount;
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
pub use mount::{Dirty, NodeCell, NodeCore, Placement, PlacementClock, ProducerKey};
pub use native_measure::*;
#[cfg(test)]
pub use recording::assert_well_formed_image;
pub use recording::{Glyph, GlyphRun, Recording, working_color};
pub use tree::safe_area::{Edge, EdgeOffsets, SafeAreaLayout, ScrollSurfaceArea, grow_rect};
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
use signals::LayoutDependencies;
use std::any::Any;
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};
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
    Rect as LayoutRect, Size as LayoutSize, StretchAxis, SubView, SubviewPlacement,
    VerticalAlignment, ViewDimensions,
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

/// Which pass attributed the current [`Reader`]: a signal guard's mark
/// follows the phase — record reads mark `PAINT`, measure/layout reads
/// mark `LAYOUT` through the cell chain.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ReaderPhase {
    /// Inside a node's record (its `sync` arm): reads subscribe to
    /// `subscriptions` and mark `PAINT`.
    Record,
    /// Inside a node's measure or layout call: reads subscribe to
    /// `layout_subscriptions` and `mark_layout`.
    Layout,
}

/// The node a signal read attributes to while `f` runs.
pub struct Reader {
    /// The reading node's cell — watcher closures mark it on update.
    pub(crate) cell: Rc<NodeCell>,
    /// The store the phase pushes watcher guards into: the node's
    /// `subscriptions` while it records, `layout_subscriptions` while it
    /// measures or lays out.
    pub(crate) store: Rc<RefCell<Vec<Retain>>>,
    /// The phase the read happens in.
    pub(crate) phase: ReaderPhase,
}

/// An outside-reader subscription: the read was made with no node recording
/// or measuring (window-signal polls, driver reads), so its marks land on
/// the root cell. The signal clone pins the identity allocation so its
/// address cannot resurface as a different signal under the same key.
///
/// Lifetime is one flush: `last_seen` carries the flush generation the
/// entry was last read in, and the finish-prune drops an entry the flush
/// did not re-read — the [`SignalWatchRegistry`](waterui_backend_core)
/// semantics outside reads had before owner attribution.
pub struct OutsideWatch {
    /// Clone of the watched signal; pins the identity allocation.
    pub _signal: Box<dyn Any>,
    /// Concrete type of the subscribed signal, used to detect identity-key
    /// collisions between different signal types.
    pub signal_type: core::any::TypeId,
    /// The watcher subscription, cancelled on drop.
    pub _guard: Retain,
    /// The flush generation this watch was last read in.
    pub last_seen: u64,
}

/// Slot-owner attribution for the animation controller: the cell a slot's
/// tick marks and the [`Dirty`] it raises.
#[derive(Clone)]
pub struct AnimationOwner {
    /// The cell marked while the slot is active.
    pub(crate) cell: Weak<NodeCell>,
    /// The dirty bit the slot raises.
    pub(crate) dirty: Dirty,
}

/// A cross-thread wake handle for a GPU or external-frame producer: posts
/// its `ProducerKey` through the renderer's wake channel and wakes the
/// host, so the next frame update marks exactly the owning cell `PRODUCER`.
#[derive(Clone)]
pub struct ProducerWake {
    /// Identity of the owning cell (from the renderer's key counter).
    key: ProducerKey,
    /// The channel the frame update drains.
    tx: std::sync::mpsc::Sender<ProducerKey>,
    /// The host wake (an engine surface's redraw handle), when the host
    /// provides one.
    redraw: Option<RedrawHandle>,
}

impl ProducerWake {
    /// Posts the producer key and wakes the host for a frame.
    pub(crate) fn request_redraw(&self) {
        // A producer thread can outlive the renderer at window close: a
        // closed channel means the owner is gone, so the post is a no-op.
        if self.tx.send(self.key).is_err() {
            return;
        }
        if let Some(redraw) = &self.redraw {
            redraw.request_redraw();
        }
    }
}

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
    /// The window's root cell: the tree's root node and the presentation
    /// hosts attach to it. The pump reads `own|below` on it to see pending
    /// marks without walking the tree.
    root: Rc<NodeCell>,
    /// Epoch every placement in the window caches resolutions against;
    /// any placement write bumps it.
    placement_clock: Rc<PlacementClock>,
    /// The node currently recording or measuring, and the phase — signal
    /// reads inside attribute their watcher guards (and marks) to it.
    reader: Option<Reader>,
    /// Slot-owner attribution for the animation controller: a bound key
    /// maps to the cell that re-records when the slot advances, plus the
    /// [`Dirty`] it raises.
    animation_owners: rustc_hash::FxHashMap<AnimationKey, AnimationOwner>,
    /// Identity-keyed subscriptions for reads no node owns (window-signal
    /// polls and driver reads outside a record or layout): one guard per
    /// signal identity, marking the root cell, kept only while the flush
    /// keeps reading it — `outside_watch_generation`/`last_seen` prune the
    /// rest.
    outside_watches: rustc_hash::FxHashMap<usize, OutsideWatch>,
    /// The flush generation `OutsideWatch::last_seen` stamps against —
    /// bumped at the start of every flush.
    outside_watch_generation: u64,
    /// Identity-less guards from reads no node owns — fresh each flush,
    /// dropped at the next flush's start.
    outside_frame_retains: Vec<Retain>,
    /// Mirror of `FrameSignals::rebuild_in_progress` for watch closures: marks
    /// the rebuild subsumes must not re-arm a frame — the generation gate the
    /// dirty-collection/dynamic flags already enforce on the flag side.
    rebuild_active: Rc<Cell<bool>>,
    /// Weak handles to every live cell in the window: `mark`'s early-exit is
    /// only sound while `below` bits clear on every cell, and the tree walk
    /// misses `RetainedSubview` subtrees (navigation pages, lazy items,
    /// overlays) whose roots attach off the render tree. The flush clears
    /// marks through this registry instead, pruning dead handles as it goes.
    /// Bridge state — the per-frame sweep over `cells` goes away with the
    /// dirty-guided descent (commit 4).
    cells: RefCell<Vec<Weak<NodeCell>>>,
    /// Producer wakes posted (possibly cross-thread) by GPU/external-frame
    /// content callbacks; the frame update drains them and marks the
    /// owning cells `PRODUCER`.
    producer_wakes: (
        std::sync::mpsc::Sender<ProducerKey>,
        std::sync::mpsc::Receiver<ProducerKey>,
    ),
    /// Key counter for producer registrations.
    next_producer_key: Cell<u64>,
    /// Which cell each producer key reports to.
    producer_owners: rustc_hash::FxHashMap<ProducerKey, std::rc::Weak<NodeCell>>,
    /// Owner cell address to its producer key: an owner re-binds the same
    /// key across re-installs so the map does not grow per frame.
    producer_keys_by_owner: rustc_hash::FxHashMap<usize, ProducerKey>,
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
    /// The clip/opacity scopes the captures in progress set aside: content a
    /// capture records is presented under them, so they count toward its
    /// clip depth.
    captured_clip_depth: usize,
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
        let signals = FrameSignals::new(frame_instant);
        let placement_clock = PlacementClock::new();
        let root = NodeCell::new(signals.clone(), Placement::new(&placement_clock));
        let cells = RefCell::new(vec![Rc::downgrade(&root)]);
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
            signals,
            animation_controller: AnimationController::default(),
            frame_instant,
            lazy: LazyState::default(),
            navigation: NavigationState::default(),
            #[cfg(feature = "accessibility")]
            accessibility: AccessibilityBuilder::default(),
            render_tree: None,
            rebuild_active: Rc::new(Cell::new(false)),
            cells,
            subview_structural_change: false,
            key_handler_stack: None,
            ime_swallowed_codes: Vec::new(),
            #[cfg(feature = "accessibility")]
            semantic_walk: false,
            root,
            placement_clock,
            reader: None,
            animation_owners: rustc_hash::FxHashMap::default(),
            outside_watches: rustc_hash::FxHashMap::default(),
            outside_watch_generation: 1,
            outside_frame_retains: Vec::new(),
            producer_wakes: std::sync::mpsc::channel(),
            next_producer_key: Cell::new(1),
            producer_owners: rustc_hash::FxHashMap::default(),
            producer_keys_by_owner: rustc_hash::FxHashMap::default(),
        }
    }

    /// The window's root cell — what `mark`s escalate to when no node is
    /// reading, and what the pump checks for pending work.
    pub(crate) const fn root_cell(&self) -> &Rc<NodeCell> {
        &self.root
    }

    /// Whether any node in the window carries a mark — the pump's
    /// frame-work trigger.
    pub(crate) fn root_is_dirty(&self) -> bool {
        !self.root_marks().is_empty()
    }

    /// The mark bits pending anywhere in the window — `own|below` on the
    /// root cell. Tests read it for the same "work armed" answer the pump
    /// reads `root_is_dirty` for.
    pub(crate) fn root_marks(&self) -> Dirty {
        self.root.own() | self.root.below()
    }

    /// A fresh [`NodeCore`] for a node being built: a loose cell on this
    /// window's signals and placement clock. The caller attaches it
    /// (`set_parent` / `attach_subtree`) when the node lands in the tree.
    pub(crate) fn new_core(&self) -> NodeCore {
        let core = NodeCore::new(self.signals.clone(), Placement::new(&self.placement_clock));
        self.cells.borrow_mut().push(Rc::downgrade(&core.cell));
        core
    }

    /// Whether a structural rebuild is capturing right now — the gate
    /// [`FrameSignals::mark_collection_dirty`] applies to the dirty flag: a
    /// `views.watch` fire while a rebuild covers the whole tree must not mark
    /// its owner's cell either.
    pub(crate) fn rebuild_active_flag(&self) -> Rc<Cell<bool>> {
        Rc::clone(&self.rebuild_active)
    }

    /// Enters the rebuild scope: `FrameSignals::begin_rebuild` plus the
    /// hydrolysis-side mirror the watch closures read (`rebuild_active`).
    /// The pair stays one call so no site can arm one without the other.
    pub(crate) fn begin_rebuild(&self) {
        self.rebuild_active.set(true);
        self.signals.begin_rebuild();
    }

    /// Leaves the rebuild scope — the [`Self::begin_rebuild`] pair.
    pub(crate) fn finish_rebuild(&self) {
        self.signals.finish_rebuild();
        self.rebuild_active.set(false);
    }

    /// Reports whether a patch request is pending, without consuming it —
    /// the flag [`NodeCell`](mount::NodeCell) marks raise through
    /// `request_refresh`.
    pub(crate) fn has_patch_request(&self) -> bool {
        self.signals.has_patch_request()
    }

    /// Reports whether the root cell carries a `STRUCTURE` mark — the
    /// scheduling input the deleted `take_rebuild_request` flags carried.
    /// A mark persists until the flush clears it, so peeking is correct.
    pub(crate) fn has_structure_marks(&self) -> bool {
        self.root_marks().contains(Dirty::STRUCTURE)
    }

    /// Consumes the window's patch request — a mark asked for another frame.
    /// Marks still wake the pump exactly once each, as `request_refresh`
    /// always did.
    pub(crate) fn take_patch_request(&mut self) -> bool {
        let requested = self.signals.take_patch_request();
        if requested {
            self.state.counters.host_wakeups += 1;
        }
        requested
    }

    /// Clears `own`/`below` on every live cell in the window, pruning dead
    /// handles: the flush's "the marks that brought this frame are consumed"
    /// step. The registry — not a tree walk — is what reaches `RetainedSubview`
    /// subtrees (navigation pages, lazy items, overlays) whose roots attach
    /// off the render tree, so `mark`'s early-exit can trust `below`.
    /// Bridge sweep — the dirty-guided descent consumes marks per node and
    /// this per-frame walk is deleted with it (commit 4).
    pub(crate) fn clear_all_marks(&self) {
        self.cells.borrow_mut().retain(|weak| {
            weak.upgrade().is_some_and(|cell| {
                cell.clear_marks();
                true
            })
        });
    }

    /// Runs `f` with `core` as the current reader in `phase`: signal reads
    /// inside attribute their watcher guards to it. The store the phase
    /// reads is cleared on entry — guards are replaced on each record and
    /// each layout.
    /// Installs `core` as the reader for `f`, restoring the displaced
    /// reader after. The semantic emit path and build-time prebuilds run
    /// here; the record and layout descents take the renderer-level
    /// `with_reader`.
    pub(crate) fn with_reader<R>(
        &mut self,
        core: &NodeCore,
        phase: ReaderPhase,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let outer = self.enter_reader(core, phase);
        let out = f(self);
        self.reader = outer;
        out
    }

    /// Installs `core` as the current reader in `phase`, returning the
    /// reader it displaced. The store the phase reads is cleared on entry —
    /// guards are replaced on each record and each layout.
    pub(crate) fn enter_reader(&mut self, core: &NodeCore, phase: ReaderPhase) -> Option<Reader> {
        let store = match phase {
            ReaderPhase::Record => Rc::clone(&core.subscriptions),
            ReaderPhase::Layout => Rc::clone(&core.layout_subscriptions),
        };
        store.borrow_mut().clear();
        self.reader.replace(Reader {
            cell: Rc::clone(&core.cell),
            store,
            phase,
        })
    }

    /// Restores the reader `enter_reader` displaced.
    pub(crate) fn leave_reader(&mut self, outer: Option<Reader>) {
        self.reader = outer;
    }

    /// The current reader's cell, if a node is recording or measuring.
    pub(crate) fn reader_cell(&self) -> Option<Rc<NodeCell>> {
        self.reader.as_ref().map(|reader| Rc::clone(&reader.cell))
    }

    /// A layout-affecting change attributed to the node now reading (its
    /// `mark_layout`), or to the root cell when no node is reading.
    pub(crate) fn context_mark_layout(&self) {
        match &self.reader {
            Some(reader) => reader.cell.mark_layout(),
            None => self.root.mark_layout(),
        }
    }

    /// A paint-only change attributed to the node now reading (a `PAINT`
    /// mark), or to the root cell when no node is reading.
    pub(crate) fn context_mark_paint(&self) {
        match &self.reader {
            Some(reader) => reader.cell.mark(Dirty::PAINT),
            None => self.root.mark(Dirty::PAINT),
        }
    }

    /// A [`ProducerWake`] for the producer owned by `cell`: the wake posts
    /// the cell's producer key through the wake channel and redraws the
    /// host through `redraw` when one exists.
    pub(crate) fn producer_wake(
        &mut self,
        cell: &Rc<NodeCell>,
        redraw: Option<RedrawHandle>,
    ) -> ProducerWake {
        let addr = Rc::as_ptr(cell) as usize;
        let key = match self.producer_keys_by_owner.get(&addr) {
            Some(&key)
                if self
                    .producer_owners
                    .get(&key)
                    .and_then(std::rc::Weak::upgrade)
                    .is_some_and(|owner| Rc::ptr_eq(&owner, cell)) =>
            {
                key
            }
            _ => {
                let key = self.next_producer_key.get();
                self.next_producer_key.set(
                    key.checked_add(1)
                        .expect("hydrolysis producer key counter overflow"),
                );
                self.producer_owners.insert(key, Rc::downgrade(cell));
                self.producer_keys_by_owner.insert(addr, key);
                key
            }
        };
        ProducerWake {
            key,
            tx: self.producer_wakes.0.clone(),
            redraw,
        }
    }

    /// Drains queued producer wakes, marking each live owner `PRODUCER`.
    /// Dead producers and dead owners are dropped from the map.
    pub(crate) fn drain_producer_wakes(&mut self) {
        let wakes: Vec<ProducerKey> = self.producer_wakes.1.try_iter().collect();
        if wakes.is_empty() {
            return;
        }
        self.producer_owners
            .retain(|_key, owner| owner.upgrade().is_some());
        self.producer_keys_by_owner.retain(|_addr, key| {
            self.producer_owners
                .get(key)
                .and_then(std::rc::Weak::upgrade)
                .is_some()
        });
        for key in wakes {
            if let Some(cell) = self
                .producer_owners
                .get(&key)
                .and_then(std::rc::Weak::upgrade)
            {
                cell.mark(Dirty::PRODUCER);
            }
        }
    }

    /// Attributes the animation slot `key` to `cell` with `dirty`, so its
    /// ticks mark the owning node. A rebind under the same key replaces the
    /// entry — one owner per slot.
    pub(crate) fn bind_animation_owner(
        &mut self,
        key: AnimationKey,
        cell: &Rc<NodeCell>,
        dirty: Dirty,
    ) {
        self.animation_owners.insert(
            key,
            AnimationOwner {
                cell: Rc::downgrade(cell),
                dirty,
            },
        );
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
        self.context_mark_layout();
    }

    /// Consumes the carried subview structural-change flag; the refresh pump
    /// folds it into this frame's `structural_change` so the prune cycle runs.
    pub(crate) fn take_subview_structural_change(&mut self) -> bool {
        core::mem::take(&mut self.subview_structural_change)
    }
}

impl HydrolysisRenderer {
    /// The renderer-level reader wrap: same contract as
    /// [`SemanticCore::with_reader`] but the closure runs on the full
    /// renderer — the record and layout descents take it.
    pub(crate) fn with_reader<R>(
        &mut self,
        core: &NodeCore,
        phase: ReaderPhase,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let outer = self.core.enter_reader(core, phase);
        let out = f(self);
        self.core.leave_reader(outer);
        out
    }

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
            captured_clip_depth: 0,
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
