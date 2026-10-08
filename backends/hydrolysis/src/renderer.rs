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
pub mod material;
mod metadata;
pub mod mount;
mod native_measure;
mod navigation;
pub mod recording;
mod render;

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
use mount::{HitClasses, HitGate, ScopeDelta};
pub use native_measure::*;
#[cfg(test)]
pub use recording::assert_well_formed_image;
pub use recording::{Glyph, GlyphRun, Recording, working_color};
pub use tree::safe_area::{
    ChromeBar, Edge, EdgeOffsets, SafeAreaLayout, ScrollSurfaceArea, grow_rect,
};
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
use waterui::background::{Background, MaterialBackground, MaterialGroup};
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
use waterui::widget::Divider;
use waterui::window::{Window, WindowState, WindowStyle};
use waterui_controls::button::{Button, ButtonConfig};
use waterui_controls::label::Label as SemanticLabel;
use waterui_controls::menu::{CommandRole, ResolvedCommand, ResolvedMenu, ResolvedMenuItem};
use waterui_controls::slider::SliderConfig;
use waterui_controls::stepper::StepperConfig;
#[cfg(feature = "accessibility")]
use waterui_controls::text_field::ContentType;
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
use waterui_text::styled::StyledStr;
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
use crate::text::SessionTextEngine;
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
    /// Whether this record drives the accessibility emit walk and so leaves
    /// an emitted-node set the unit rule diffs. Probe records — build-time
    /// validation and measure reads — run the same record mechanics but emit
    /// no accessibility set of their own.
    #[cfg(feature = "accessibility")]
    pub(crate) emits_a11y: bool,
    /// The record sequence this reader entered under (`0` for `Layout`
    /// readers and probes) — `leave_reader` retires the reader's subviews
    /// whose placement links predate it.
    pub(crate) seq: u64,
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

/// §D's presentation hosts: one permanent cell per transient emit path,
/// attached under the window anchor in the pump's emission order. Each
/// host records its presentation content like a node — its registrations
/// (occluders, targets, a11y) purge at the start of each of its records,
/// so a closed or re-recorded presentation's entries retire in O(its
/// entries), and its placement slot keeps all presentation content ranked
/// above the window's own.
pub struct PresentationHosts {
    /// The overlay-mode text context menu's host.
    pub(crate) text_overlay: NodeCore,
    /// The `.context_menu` presentation's host.
    pub(crate) context_menu: NodeCore,
    /// `.anchored_overlay` presentations' host.
    pub(crate) anchored: NodeCore,
}

impl PresentationHosts {
    /// §F's teardown before a remount: every host subtree drops its
    /// `NodeLayers` with the outgoing engine window, each cell marked
    /// `PAINT|COMMIT` like [`RenderNode::unmount`] leaves the window's own
    /// tree.
    pub(crate) fn unmount(&self) {
        for host in [&self.text_overlay, &self.context_menu, &self.anchored] {
            host.cell.unmount_subtree();
        }
    }
}

/// One platform-view sink record resolved at materialization: the table
/// the placement publishes into plus the window-space frame and clip its
/// paint chain computed — staged so `record` calls run once per frame in
/// one frame-end step.
pub struct MaterializedPlatformView {
    /// The sink table the view records into.
    pub(crate) table: Rc<RefCell<crate::platform_view::PlatformViewTable>>,
    /// The resolved placement (window-space frame, clip, paint-order
    /// rank, visibility).
    pub(crate) placement: crate::platform_view::PlatformViewPlacement,
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
    /// A second [`NodeCore`] naming the root cell — the pump wraps the
    /// window flush in its `Record` so registrations that arrive with no
    /// enclosing node (window-level payloads) still have a recording owner.
    root_core: NodeCore,
    /// The presentation hosts — §D's host cells brought forward so each
    /// transient emit path owns its registrations: they retire when the
    /// host re-records or its presentation closes. Their placements sit
    /// under `window_placement` above the content root, in pump order.
    presentation_hosts: PresentationHosts,
    /// Platform-view sink records staged at materialization — resolved
    /// window-space placements in paint order, consumed once by
    /// [`Self::record_platform_views`] at frame end rather than written
    /// mid-walk.
    materialized_platform_views: Vec<MaterializedPlatformView>,
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
    /// A whole-tree emit pass is in progress — the rendered flush, the
    /// semantic emit walk, or a build-time capture: every a11y-emitting
    /// record runs inside it, so a unit diff never needs to schedule
    /// coverage the pass already provides. A standalone re-record outside a
    /// pass (the dirty-guided descents of commit 4) leaves the flag clear —
    /// the unit mark then wakes the ancestor the set diff escalates to.
    emit_pass_active: Cell<bool>,
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
    /// The retained registries every record emits into, owned per node
    /// and resolved through placements; `hit_test`'s flat lists are the
    /// materialized view consumers read.
    retained: mount::RetainedRegistry,
    /// The placement scopes the open clip scopes created —
    /// child placements of the placement they were pushed under.
    /// Registrations resolve through the top of the stack.
    placement_scope_stack: Vec<Rc<Placement>>,
    /// Saved `placement_scope_stack`s of enclosing records: a nested
    /// record starts with an empty stack, restored on the way out.
    saved_scope_stacks: Vec<Vec<Rc<Placement>>>,
    /// `emit_owner` values saved across nested records, riding the same
    /// stack discipline as `saved_scope_stacks`.
    #[cfg(feature = "accessibility")]
    saved_emit_owners: Vec<Weak<NodeCell>>,
    /// The sequence the current record runs under — `enter_reader`
    /// bumps it; a registration's [`mount::PaintOrder`] carries it.
    record_seq: u64,
    /// The placement epoch `hit_test`'s flat lists last materialized
    /// against — they rebuild when it lags or `retained.stale` is set.
    materialized_epoch: u64,
    /// The window material request the current frame presents: written
    /// when the window's backdrop lands and cleared after the commit
    /// consumes it, so a material view flush sees the scope the window
    /// declares (always `None` — the window's own backdrop mounts solo).
    pub(crate) window_material: Option<mount::program::MaterialRequest>,
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
    window_bounds: kurbo::Rect,
    /// The transform the window's root content is flushed under: logical layout
    /// units onto the target's physical pixel grid. Stored alongside
    /// [`Self::window_bounds`] because the pair is what says where the viewport
    /// is in device pixels, which is what
    /// [`HydrolysisRenderer::push_gpu_surface_layer`] tests a full-window GPU
    /// surface against.
    /// The window's display transform: the window layer's (§A), the only
    /// place the display scale is applied.
    window_display_transform: kurbo::Affine,
    /// Wake target supplied when this renderer itself is hosted inside
    /// another GPU host. Renderer-owned redraws use it to wake the parent host
    /// without polling frames.
    host_redraw_handle: Option<RedrawHandle>,
    /// Engine window surfaces by GPU-context id: the `TextureTarget` surface,
    /// the stable mounts under it and the resource registrations its content
    /// names. Entries whose device was reported lost are pruned at the next
    /// presented frame.
    cherenkov_window: Option<crate::renderer::render::CherenkovWindow>,
    /// The engine work the last commit did, with the live mounted counts.
    last_mount_stats: mount::MountStats,
    /// The tests' [`MirrorTarget`](tests::mirror::MirrorTarget) mount.
    #[cfg(test)]
    mirror: Option<tests::mirror::MirrorWindow>,
    /// The node programs open while recording, innermost last (§C).
    program: Vec<mount::ProgramBuilder>,
    /// The engine's frame scheduling answer from the last presented frame.
    /// The pump follows it: `Next::Idle` means no animation is running and
    /// the display link may sleep.
    engine_next: Option<cherenkov::Next>,
    /// Per-frame applied-filter telemetry the render thread's `EngineEffect`
    /// calls accumulate into; reset before each `Engine::render` and read back
    /// into the `frame_applied_filter_*` fields after it.
    applied_filter_metrics: Arc<crate::renderer::effects::AppliedFilterMetrics>,
    frame_applied_filter_count: u32,
    frame_applied_filter_effect: Duration,
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
    /// The within-window material the window's background names, which the
    /// window's root is mounted over; `None` for any other background.
    window_backdrop: Option<crate::renderer::material::WindowBackdrop>,
    /// The `.material_group()` scope stack the flush keeps.
    pub(crate) compositor: render::Compositor,
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

pub const HIT_TEST_ALPHA_THRESHOLD: f32 = 0.01;

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
        self.record_scope(|scopes| scopes.push_key_handler(&handler));
        self.key_handler_stack = Some(Rc::new(KeyHandlerNode {
            scope: KeyHandlerScope { env, handler },
            parent: self.key_handler_stack.take(),
        }));
    }

    /// Leaves the innermost `OnKeyPress` scope.
    pub(crate) fn pop_key_handler_scope(&mut self) {
        self.record_scope(mount::RetainedScopes::pop_key_handler);
        self.key_handler_stack = self
            .key_handler_stack
            .take()
            .and_then(|link| link.parent.clone());
    }

    pub(crate) fn new(frame_instant: Instant, text: SessionTextEngine) -> Self {
        let signals = FrameSignals::new(frame_instant);
        let placement_clock = PlacementClock::new();
        let root = NodeCell::new(signals.clone(), Placement::new(&placement_clock));
        // The window anchor ranks every registered entry by placement
        // path: the content root takes slot 0 and each presentation host
        // the next slot in the pump's emission order, so a host's entries
        // always sort above the window's own content, and a later-emitted
        // host above an earlier one. Dynamic anchors draw positions from
        // `take_item` starting past these fixed slots.
        let window_placement = Placement::new(&placement_clock);
        root.placement()
            .set_parent(Some(Rc::clone(&window_placement)));
        root.placement().set_index(window_placement.take_item());
        let root_core = NodeCore::for_cell(&root);
        let cells = RefCell::new(vec![Rc::downgrade(&root)]);
        // The window's item space is pinned, not cursor-dealt: slot 0 is
        // the root content's, slots 1..=3 the three presentation hosts'
        // (hosts rank above all content — their path compares greater),
        // and the cursor advances past them so a window-anchored
        // registration ranks above the fixed set as "registered last".
        for _ in 0..4 {
            let _ = window_placement.take_item();
        }
        let mut host_index = 1;
        let mut host = || {
            let core = NodeCore::new(signals.clone(), Placement::new(&placement_clock));
            core.cell.set_parent(&root);
            core.cell
                .placement()
                .set_parent(Some(Rc::clone(&window_placement)));
            core.cell.placement().set_index(host_index);
            host_index += 1;
            cells.borrow_mut().push(Rc::downgrade(&core.cell));
            core
        };
        let presentation_hosts = PresentationHosts {
            text_overlay: host(),
            context_menu: host(),
            anchored: host(),
        };
        Self {
            state: HydroState::new(text),
            hit_test: HitTestState::default(),
            gesture_engine: GestureEngine::default(),
            gesture_group_ids: BTreeMap::new(),
            next_gesture_group_id: 0,
            text_editing: TextEditingState::default(),
            popup_menu: PopupMenuState::default(),
            window_id: WindowId::Orphan,
            render_depth: 0,
            owner_stack: Vec::new(),
            root_core,
            presentation_hosts,
            materialized_platform_views: Vec::new(),
            signals,
            animation_controller: AnimationController::default(),
            frame_instant,
            lazy: LazyState::default(),
            navigation: NavigationState::default(),
            #[cfg(feature = "accessibility")]
            accessibility: AccessibilityBuilder::default(),
            render_tree: None,
            rebuild_active: Rc::new(Cell::new(false)),
            emit_pass_active: Cell::new(false),
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
            retained: mount::RetainedRegistry::new(),
            placement_scope_stack: Vec::new(),
            saved_scope_stacks: Vec::new(),
            #[cfg(feature = "accessibility")]
            saved_emit_owners: Vec::new(),
            record_seq: 0,
            materialized_epoch: 0,
            window_material: None,
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

    /// Opens a whole-tree emit pass — [`SemanticCore::emit_pass_active`].
    /// The whole-tree drivers (`flush_window_tree`, the semantic emit walk)
    /// bracket their emit regions with this pair; a standalone re-record
    /// outside a pass is what the unit rule's mark exists for.
    pub(crate) fn begin_emit_pass(&self) {
        self.emit_pass_active.set(true);
    }

    /// Closes the pass [`Self::begin_emit_pass`] opened.
    pub(crate) fn finish_emit_pass(&self) {
        self.emit_pass_active.set(false);
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
    /// Clears the placement and commit bits on every live cell once the
    /// mount has lowered the frame's programs and placements. Bridge sweep
    /// like [`Self::clear_all_marks`]; the dirty-guided commit consumes them
    /// per node (commit 4).
    pub(crate) fn clear_commit_marks(&self) {
        let bits = Dirty::PLACE | Dirty::COMMIT;
        self.root.clear_bits(bits);
        self.cells.borrow_mut().retain(|weak| {
            weak.upgrade().is_some_and(|cell| {
                cell.clear_bits(bits);
                true
            })
        });
    }

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
    /// reader after — the accessibility emit walk's entry; the record and
    /// layout descents take the renderer-level `with_reader`.
    #[cfg(feature = "accessibility")]
    pub(crate) fn with_reader<R>(
        &mut self,
        core: &NodeCore,
        phase: ReaderPhase,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.with_reader_kind(core, phase, true, f)
    }

    /// A probe record: the same reader mechanics as a record —
    /// subscription capture, scope-stack save, emit-owner swap — but the
    /// record drives no accessibility emit walk, so it leaves no emitted
    /// set for the unit rule. Build-time validation and measure prebuilds
    /// run here.
    pub(crate) fn with_probe_reader<R>(
        &mut self,
        core: &NodeCore,
        phase: ReaderPhase,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.with_reader_kind(core, phase, false, f)
    }

    /// Shared enter/run/leave behind [`Self::with_probe_reader`] and the
    /// renderer-level `with_reader`; `emits_a11y` rides on the reader so
    /// `leave_reader` can gate the a11y unit-diff bookkeeping on it.
    fn with_reader_kind<R>(
        &mut self,
        core: &NodeCore,
        phase: ReaderPhase,
        emits_a11y: bool,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        if phase == ReaderPhase::Record && self.record_is_current(core) {
            return f(self);
        }
        let outer = self.enter_reader(core, phase, emits_a11y);
        let out = f(self);
        self.leave_reader(outer);
        out
    }

    /// Whether `core`'s own record is the one currently open — a record
    /// nested inside the same cell's record is that record: re-entering
    /// would wipe the subscriptions and registrations the outer record
    /// already collected. The window flush nests a root record inside the
    /// frame's own root record this way.
    pub(crate) fn record_is_current(&self, core: &NodeCore) -> bool {
        self.reader.as_ref().is_some_and(|reader| {
            reader.phase == ReaderPhase::Record && Rc::ptr_eq(&reader.cell, &core.cell)
        })
    }

    /// Installs `core` as the current reader in `phase`, returning the
    /// reader it displaced. The store the phase reads is cleared on entry —
    /// guards are replaced on each record and each layout. A record also
    /// bumps the record sequence, purges the node's stale registrations
    /// from the retained registries, and saves the open placement scopes
    /// so the node starts on a clean scope stack.
    pub(crate) fn enter_reader(
        &mut self,
        core: &NodeCore,
        phase: ReaderPhase,
        emits_a11y: bool,
    ) -> Option<Reader> {
        let store = match phase {
            ReaderPhase::Record => Rc::clone(&core.subscriptions),
            ReaderPhase::Layout => Rc::clone(&core.layout_subscriptions),
        };
        store.borrow_mut().clear();
        // Only a record that emits and places retires unplaced subviews at
        // leave — the sequence is its yardstick. Probes (measure reads,
        // build validation) run the record mechanics for their
        // subscriptions but neither place nor emit: they enter with
        // `seq == 0`, which retires nothing.
        let seq = if phase == ReaderPhase::Record && emits_a11y {
            self.record_seq = self
                .record_seq
                .checked_add(1)
                .expect("hydrolysis renderer: record sequence overflow");
            self.record_seq
        } else {
            0
        };
        if phase == ReaderPhase::Record {
            core.cell.placement().reset_props();
            self.purge_registrations(&core.cell);
            self.saved_scope_stacks
                .push(std::mem::take(&mut self.placement_scope_stack));
            #[cfg(feature = "accessibility")]
            {
                self.saved_emit_owners
                    .push(self.accessibility.emit_owner.clone());
                self.accessibility.emit_owner = Rc::downgrade(&core.cell);
                // The new record re-arms the cell's emission bookkeeping:
                // `a11y_emitted` fills as it runs and the unit claim resets
                // until a scope claim actually lands. A probe record touches
                // none of it — the emitted set and unit flag belong to the
                // enclosing real record.
                if emits_a11y {
                    core.cell.a11y_emitted.borrow_mut().clear();
                    core.cell.a11y_unit.set(false);
                    core.cell.a11y_recording.set(true);
                }
            }
            // A nested record: the entering node's scope pushes are
            // ancestor state for it — freeze the outgoing reader's open
            // pushes, then reset the incoming node's scope list.
            if let Some(outer) = &self.reader {
                outer.cell.scopes.borrow_mut().freeze_for_descendant();
            }
            core.cell.scopes.borrow_mut().begin_record();
        }
        self.reader.replace(Reader {
            cell: Rc::clone(&core.cell),
            store,
            phase,
            #[cfg(feature = "accessibility")]
            emits_a11y,
            seq,
        })
    }

    /// The current record's sequence — `link_placement` stamps it on
    /// placed subviews, and the a11y emit walk stamps it on every subview
    /// it descends into (a semantic frame has no placements to stamp
    /// otherwise).
    #[cfg(feature = "accessibility")]
    pub(crate) const fn record_seq(&self) -> u64 {
        self.record_seq
    }

    /// Restores the reader `enter_reader` displaced. The saved stacks —
    /// placement scopes, emit owner — belong to `Record` readers: a
    /// `Layout` reader nested inside a record restores nothing.
    pub(crate) fn leave_reader(&mut self, outer: Option<Reader>) {
        let leaving = std::mem::replace(&mut self.reader, outer);
        if !leaving
            .as_ref()
            .is_some_and(|leaving| leaving.phase == ReaderPhase::Record)
        {
            return;
        }
        let leaving =
            leaving.expect("hydrolysis renderer: record reader left with the stack empty");
        let saved = self
            .saved_scope_stacks
            .pop()
            .expect("hydrolysis renderer: saved scope stack underflow");
        self.placement_scope_stack = saved;
        // A subview this record attached but never placed is unplaced: its
        // whole subtree's registrations and a11y nodes retire now (§D).
        self.retire_unplaced_subviews(&leaving.cell, leaving.seq);
        #[cfg(feature = "accessibility")]
        {
            if let Some(saved) = self.saved_emit_owners.pop() {
                self.accessibility.emit_owner = saved;
            }
            if leaving.emits_a11y {
                self.check_a11y_unit_diff(&leaving.cell);
                leaving.cell.a11y_recording.set(false);
            }
        }
        #[cfg(not(feature = "accessibility"))]
        drop(leaving);
    }

    /// Retires the whole subtrees under `roots`: every descendant cell's
    /// retained registrations purge, its emitted a11y nodes leave the
    /// shared accessibility state in one batch, and its `NodeLayers` drop
    /// (decision 3 — the cell keeps its node while the engine memory
    /// follows what is on screen). A cell already retired and not placed
    /// since is skipped with its subtree — nothing under it can have
    /// registered without placing it — so a hidden subtree costs nothing
    /// on later frames. O(newly retired nodes).
    #[cfg_attr(
        not(feature = "accessibility"),
        expect(
            clippy::needless_pass_by_ref_mut,
            reason = "the a11y node retire that needs the mutable borrow is compiled out"
        )
    )]
    fn retire_subtrees(&mut self, roots: Vec<Rc<NodeCell>>) {
        let mut stack = roots;
        #[cfg(feature = "accessibility")]
        let mut ids: Vec<AccessibilityNodeId> = Vec::new();
        while let Some(cell) = stack.pop() {
            if cell.retired.replace(true) {
                continue;
            }
            self.purge_registrations(&cell);
            cell.unmount();
            #[cfg(feature = "accessibility")]
            {
                ids.append(&mut cell.a11y_emitted.borrow_mut());
                *cell.a11y_retired.borrow_mut() = None;
                cell.a11y_unit.set(false);
            }
            cell.children(&mut stack);
        }
        #[cfg(feature = "accessibility")]
        self.accessibility.retire_nodes(&ids);
    }

    /// §D's unplaced rule, run at `Record` leave: every `RetainedSubview`
    /// the record owns that it did not place while it ran has its subtree
    /// retired — its registrations and a11y nodes leave through
    /// [`Self::retire_subtrees`].
    fn retire_unplaced_subviews(&mut self, cell: &Rc<NodeCell>, seq: u64) {
        let mut unplaced: Vec<Rc<NodeCell>> = Vec::new();
        cell.subviews.borrow_mut().retain(|weak| {
            weak.upgrade().is_some_and(|subview| {
                // A link stamped while this record ran — at its own
                // sequence or any nested record's — counts as placed.
                if subview.placed_seq() < seq {
                    unplaced.push(subview);
                }
                true
            })
        });
        if !unplaced.is_empty() {
            self.retire_subtrees(unplaced);
        }
    }

    /// The §B.4 unit rule: a node whose record emitted a11y nodes compares
    /// them with the set its previous record left behind; any difference
    /// repaints the nearest a11y-unit ancestor (the root when none) so the
    /// collapse and claim decisions recompute over the whole unit.
    #[cfg(feature = "accessibility")]
    fn check_a11y_unit_diff(&self, cell: &Rc<NodeCell>) {
        // The unit rule diffs the semantic payload (role, label, actions,
        // children). Bounds resolve at publish — after both the mid-flush
        // node and its stored baseline — so they strip out of the compare;
        // a bounds-only change travels the layout marks instead.
        fn semantic_payload(node: &AccessibilityNode) -> AccessibilityNode {
            let mut node = node.clone();
            node.clear_bounds();
            node
        }
        // The emitted set lives on the cell: `a11y_emitted` collected the
        // ids this record pushed, and `node` looks each up by id rather
        // than scanning the node list.
        let emitted: std::collections::BTreeMap<AccessibilityNodeId, AccessibilityNode> = cell
            .a11y_emitted
            .borrow()
            .iter()
            .filter_map(|id| {
                self.accessibility
                    .node(*id)
                    .map(|node| (*id, semantic_payload(node)))
            })
            .collect();
        // The retired set is the node *as the record left it* — bounds and
        // claimed labels resolve after the record, so the baseline is
        // stored at record end on the cell itself: it lives and dies with
        // the node, never in a map keyed by a reusable address.
        // The rule compares a *re-record* against the set its previous
        // record left: a cell that never recorded has no baseline and
        // nothing to mark (its first record is painting anyway). Order is
        // not significant — `retired` rides the emit order of the previous
        // record — so both sides compare through the id-keyed map.
        let differ = cell.a11y_retired.borrow().as_ref().is_some_and(|retired| {
            retired
                .iter()
                .map(|(id, node)| (*id, semantic_payload(node)))
                .collect::<std::collections::BTreeMap<AccessibilityNodeId, AccessibilityNode>>()
                != emitted
        });
        *cell.a11y_retired.borrow_mut() = Some(emitted.into_iter().collect());
        if differ {
            // The mark exists to wake a unit ancestor this re-record did
            // not cover: while an emit pass (or the ancestor's own record)
            // is in flight its emitted set is already refreshing, so an
            // extra PAINT mark would only arm a redundant frame — and
            // leave a false unapplied-work signal behind.
            let unit = cell.nearest_a11y_unit_ancestor();
            if !(unit.a11y_recording.get() || self.emit_pass_active.get()) {
                unit.mark(Dirty::PAINT);
            }
        }
    }

    /// Records a scope push/pop against the active reader's
    /// [`RetainedScopes`]; calls outside a record do nothing (the owner
    /// stacks still move — the replay list simply has no record to file
    /// under).
    pub(crate) fn record_scope(&self, f: impl FnOnce(&mut crate::renderer::mount::RetainedScopes)) {
        if let Some(reader) = &self.reader {
            f(&mut reader.cell.scopes.borrow_mut());
        }
    }

    /// §B.4: the active reader's record reached an accessibility unit
    /// boundary — flag its cell so a descendant's emission diff escalates
    /// here.
    #[cfg(feature = "accessibility")]
    pub(crate) fn mark_a11y_unit(&self) {
        if let Some(reader) = &self.reader {
            reader.cell.a11y_unit.set(true);
        }
    }

    /// The current reader's cell, if a node is recording or measuring.
    pub(crate) fn reader_cell(&self) -> Option<Rc<NodeCell>> {
        self.reader.as_ref().map(|reader| Rc::clone(&reader.cell))
    }

    /// The cell a registration emitted right now belongs to — the
    /// recording node. Every registration has a recording owner: the pump
    /// wraps the window flush in the root cell's own record and each
    /// transient emit path in its presentation host's, so a call reaching
    /// here with no reader is a bug in the emit path, not a fallback case.
    pub(crate) fn registration_owner(&self) -> Rc<NodeCell> {
        self.reader_cell()
            .expect("hydrolysis renderer: registration emitted with no recording owner")
    }

    /// The placement a registration emitted right now resolves through:
    /// the topmost open clip/alpha scope, else the recording node's own
    /// placement. Every emit path runs under a reader, so a request with
    /// neither is a bug in the emit path.
    pub(crate) fn current_placement(&self) -> Rc<Placement> {
        self.placement_scope_stack
            .last()
            .cloned()
            .or_else(|| self.reader_cell().map(|cell| Rc::clone(cell.placement())))
            .expect("hydrolysis renderer: placement requested with no current reader")
    }

    /// Opens a clip/alpha scope as a child placement of the current one;
    /// registrations inside resolve through it. `scope.transform` is the
    /// scope's delta in the recording node's space, stored as given — it
    /// is never derived from the paint transform, which carries the
    /// display scale and restarts inside a navigation capture. `clip` is a
    /// rect in the scope's own space; `scope.hit_alpha` folds into the hit
    /// gate.
    pub(crate) fn push_placement_scope(&mut self, scope: ScopeDelta, clip: Option<kurbo::Rect>) {
        let parent = self.current_placement();
        let placement = Placement::new(&self.placement_clock);
        placement.set_transform(scope.transform);
        placement.set_parent(Some(parent.clone()));
        placement.set_index(parent.take_item());
        placement.set_clip(clip);
        placement.set_alpha(scope.hit_alpha);
        self.placement_scope_stack.push(placement);
    }

    /// Opens a hit-gate scope: registrations inside keep their paint chain
    /// (the content still draws) while `gate` removes or alpha-gates
    /// exactly the kinds dev's corresponding mechanism did —
    /// `Hittable(false)`, inactive navigation pages and exiting overlays,
    /// suppressed context-menu previews.
    pub(crate) fn push_hit_gate_scope(&mut self, gate: HitGate) {
        let parent = self.current_placement();
        let scope = Placement::new(&self.placement_clock);
        scope.set_parent(Some(parent.clone()));
        scope.set_index(parent.take_item());
        scope.set_alpha(gate.alpha());
        scope.set_removes(gate.removes());
        self.placement_scope_stack.push(scope);
    }

    /// Closes the scope `push_placement_scope` opened.
    pub(crate) fn pop_placement_scope(&mut self) {
        self.placement_scope_stack
            .pop()
            .expect("hydrolysis renderer: placement scope underflow");
    }

    /// Links `core`'s placement to the current placement — the innermost
    /// open scope, or the recording node's own — and writes `delta`, the
    /// transform from that frame to the node's own. The child claims the
    /// parent's next item index, so it orders under its parent exactly at
    /// its emission position. Called at the child boundary of the walk
    /// before the node records. A node linking under itself (the window
    /// root) keeps its existing parent and index; the window content tree
    /// is the root cell's only item, so its link keeps the pinned slot
    /// zero rather than a cursor the sentinel never resets.
    pub(crate) fn link_placement(&self, core: &NodeCore, delta: kurbo::Affine) {
        let placement = core.cell.placement();
        let parent = self.current_placement();
        if !Rc::ptr_eq(&parent, placement) {
            let index = if Rc::ptr_eq(&parent, self.root.placement()) {
                0
            } else {
                parent.take_item()
            };
            placement.set_parent(Some(parent));
            placement.set_index(index);
        }
        if placement.transform() != delta {
            core.cell.mark_quiet(Dirty::PLACE | Dirty::COMMIT);
        }
        placement.set_transform(delta);
        core.cell.mark_placed(self.record_seq);
    }

    /// [`Self::link_placement`] anchored at an explicit placement — the
    /// subtree-capture and overlay path, whose nodes record outside the
    /// frame they display in. The caller supplies `index`, the position
    /// the grafted subtree occupies among the anchor's items (a navigation
    /// page takes its stack position; no `u32::MAX` shortcut — every
    /// entry's rank is a real slot).
    pub(crate) fn link_placement_to(
        &self,
        core: &NodeCore,
        anchor: Option<Rc<Placement>>,
        delta: kurbo::Affine,
        index: u32,
    ) {
        let placement = core.cell.placement();
        placement.set_parent(anchor);
        if placement.transform() != delta {
            core.cell.mark_quiet(Dirty::PLACE | Dirty::COMMIT);
        }
        placement.set_transform(delta);
        placement.set_index(index);
        core.cell.mark_placed(self.record_seq);
    }

    /// The IDENTITY anchor for payloads that are already in window space —
    /// the parent of the root cell's own placement.
    /// Test-only today: seeds stage entries through it.
    #[cfg(test)]
    pub(crate) fn window_placement(&self) -> Rc<Placement> {
        self.root
            .placement()
            .parent()
            .expect("hydrolysis renderer: root placement detached")
    }

    /// Resolves a node-local rect to window space through the current
    /// placement chain — the retained replacement for the deleted
    /// `ctx.hit_transform` wherever a registration still wants a window-space
    /// answer immediately (accessibility bounds, overlay anchors). Like dev's
    /// `transformed_rect(hit_transform, _)`: transform only — no clip fold,
    /// since these consumers keep their own clip semantics.
    pub(crate) fn resolve_window_rect(&self, local: kurbo::Rect) -> kurbo::Rect {
        crate::renderer::transformed_rect(self.current_placement().resolved_transform(false), local)
    }

    /// The current placement's composed hit transform — the replacement for
    /// the deleted `ctx.hit_transform` in code that maps coordinates both
    /// ways (interaction wave origins).
    pub(crate) fn current_hit_transform(&self) -> kurbo::Affine {
        self.current_placement().resolved_transform(true)
    }

    /// The flat-list view of the retained registries, rebuilt when a
    /// registry write or a placement write made it stale.
    pub(crate) fn registries(&mut self) -> &HitTestState {
        self.materialize_registries();
        &self.hit_test
    }

    /// The focused text-input target's identity and window-space frame
    /// when its retained registration lives in `scope`'s subtree — the
    /// §7.1 clearance's "a field this subtree's own registrations
    /// reported" test. Dev cut the frame's emission list at the count the
    /// surface's content began with; per-owner buckets carry no such
    /// boundary, so the subtree test walks the owner chain and the frame
    /// resolves through the entry's own paint chain (unclipped, as dev's
    /// `frame` was).
    pub(crate) fn focused_field_frame_in_scope(
        &self,
        scope: &Rc<NodeCell>,
    ) -> Option<(crate::renderer::input::InteractionKey, kurbo::Rect)> {
        let key = self.text_editing.focused_key()?;
        let (owner, local, placement) = self.retained.live_owners().find_map(|cell| {
            let found = cell.registrations.borrow().as_ref().and_then(|regs| {
                regs.text_input_targets
                    .iter()
                    .find(|entry| entry.payload.interaction_key == key)
                    .map(|entry| (entry.region.local, Rc::clone(&entry.region.placement)))
            });
            found.map(|(local, placement)| (cell, local, placement))
        })?;
        // Dev's flat list only held the fields the hit gate admitted.
        if !placement.admits(HitClasses::INPUT) {
            return None;
        }
        let mut ancestor = Some(owner);
        loop {
            let current = ancestor?;
            if Rc::ptr_eq(&current, scope) {
                break;
            }
            ancestor = current.parent();
        }
        let (transform, _, _) = placement.resolved_chain(false);
        Some((key, crate::renderer::transformed_rect(transform, local)))
    }

    /// Registers `payload` under the current placement — one retained
    /// entry in the recording owner's bucket, ordering exactly where its
    /// emission ranks under that anchor. Registering with no recording
    /// owner is a bug in the emit path and panics.
    pub(crate) fn register_retained<T: mount::SetRegistrationOwner>(
        &mut self,
        payload: T,
        local: kurbo::Rect,
        pick: impl FnOnce(&mut mount::OwnerRegistrations) -> &mut Vec<mount::RetainedEntry<T>>,
    ) {
        let anchor = self.current_placement();
        self.register_retained_at(payload, local, &anchor, pick);
    }

    /// [`Self::register_retained`] with the anchor supplied — payloads
    /// already in window space (occluder frames) anchor at the placement
    /// that resolves identity for them.
    pub(crate) fn register_retained_at<T: mount::SetRegistrationOwner>(
        &mut self,
        payload: T,
        local: kurbo::Rect,
        anchor: &Rc<Placement>,
        pick: impl FnOnce(&mut mount::OwnerRegistrations) -> &mut Vec<mount::RetainedEntry<T>>,
    ) {
        let owner = self.registration_owner();
        self.retained.enlist(&owner);
        let entry =
            mount::RetainedEntry::at(payload, local, anchor, &owner, self.retained.next_seq());
        {
            let mut slot = owner.registrations.borrow_mut();
            pick(slot.get_or_insert_with(|| Box::new(mount::OwnerRegistrations::default())))
                .push(entry);
        }
        self.retained.stale.set(true);
    }

    /// The one frame-end platform-view record step: every view staged at
    /// materialization writes its resolved placement into its table in
    /// paint order. The staged list persists until the next materialization
    /// rebuilds it, so the record runs on every presented frame — an idle
    /// pump re-presenting retained layers included — and is idempotent
    /// within a frame: each table's pending set is rewritten, not appended.
    pub(crate) fn record_platform_views(&self) {
        let mut per_table: Vec<(
            Rc<RefCell<crate::platform_view::PlatformViewTable>>,
            Vec<crate::platform_view::PlatformViewPlacement>,
        )> = Vec::new();
        for staged in &self.materialized_platform_views {
            match per_table
                .iter_mut()
                .find(|(table, _)| Rc::ptr_eq(table, &staged.table))
            {
                Some((_, placements)) => placements.push(staged.placement.clone()),
                None => per_table.push((Rc::clone(&staged.table), vec![staged.placement.clone()])),
            }
        }
        for (table, placements) in per_table {
            table.borrow_mut().record_frame(placements);
        }
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

    /// A change attributed to the node that registered `owner` — the
    /// target's cell. A dead owner has no registrations left (they retire
    /// with its record), so there is nothing to wake.
    pub(crate) fn mark_owner(owner: &Weak<NodeCell>, bits: Dirty) {
        if let Some(cell) = owner.upgrade() {
            cell.mark(bits);
        }
    }

    /// A change attributed to the node that owns `key`.
    pub(crate) fn mark_key_owner(&self, key: &crate::renderer::input::InteractionKey, bits: Dirty) {
        Self::mark_owner(&self.key_owner(key), bits);
    }

    /// The node that owns `key`: the node that bound its interaction
    /// state, or — for a key that never binds interaction state, a text or
    /// embedded input — the owner of the target registered under it. A
    /// key with neither is a bug in the caller: every key a mark names
    /// came from a live registration.
    pub(crate) fn key_owner(&self, key: &crate::renderer::input::InteractionKey) -> Weak<NodeCell> {
        self.try_key_owner(key)
            .unwrap_or_else(|| panic!("hydrolysis renderer: interaction key {key:?} has no owner"))
    }

    /// [`Self::key_owner`] for a caller holding a second ownership source
    /// (the focused semantic node) to consult before it panics.
    pub(crate) fn try_key_owner(
        &self,
        key: &crate::renderer::input::InteractionKey,
    ) -> Option<Weak<NodeCell>> {
        let live = |owner: &Weak<NodeCell>| owner.strong_count() > 0;
        self.hit_test
            .interaction
            .owner_of(key)
            .filter(|owner| live(owner))
            .or_else(|| {
                self.text_editing
                    .text_input_targets
                    .iter()
                    .find(|target| &target.interaction_key == key)
                    .map(|target| target.owner.clone())
            })
            .or_else(|| {
                self.hit_test
                    .embedded_input_targets
                    .iter()
                    .find(|target| &target.interaction_key == key)
                    .map(|target| target.owner.clone())
            })
    }

    /// A change driven by a `ScrollHandle` write (a scrollbar drag, a
    /// fling tick, an accessibility scroll): the mark belongs to the
    /// owner of the `ScrollTarget` that holds that handle — looked up
    /// through the registry, never the root.
    pub(crate) fn mark_scroll_owner(&self, handle: &crate::scroll::ScrollHandle, bits: Dirty) {
        Self::mark_owner(&self.scroll_target_owner(handle.cache_key()), bits);
    }

    /// The owner of the retained `ScrollTarget` holding the handle keyed
    /// `key` — read from the retained registries, so a target the hit gate
    /// keeps out of the materialized list still resolves. A handle no
    /// target holds is a bug: only a registered scroll surface drives one.
    pub(crate) fn scroll_target_owner(&self, key: usize) -> Weak<NodeCell> {
        self.try_scroll_target_owner(key).unwrap_or_else(|| {
            panic!("hydrolysis renderer: scroll handle {key} has no registered scroll target")
        })
    }

    /// An accessibility-driven scroll of the semantic node `node`: the
    /// owner of the `ScrollTarget` holding `handle`, or — under the
    /// semantic runtime, which registers no hit targets — the cell that
    /// emitted `node`. Neither is a bug: the action reached a node the
    /// tree emitted.
    #[cfg(feature = "accessibility")]
    pub(crate) fn mark_scroll_owner_of_node(
        &self,
        handle: &crate::scroll::ScrollHandle,
        node: AccessibilityNodeId,
        bits: Dirty,
    ) {
        let owner = self
            .try_scroll_target_owner(handle.cache_key())
            .or_else(|| self.live_node_owner(node))
            .unwrap_or_else(|| panic!("hydrolysis renderer: scrolled node {node:?} has no owner"));
        Self::mark_owner(&owner, bits);
    }

    /// The live cell that emitted the semantic node `node`.
    #[cfg(feature = "accessibility")]
    pub(crate) fn live_node_owner(&self, node: AccessibilityNodeId) -> Option<Weak<NodeCell>> {
        self.accessibility
            .node_owners
            .get(&node)
            .filter(|owner| owner.strong_count() > 0)
            .cloned()
    }

    fn try_scroll_target_owner(&self, key: usize) -> Option<Weak<NodeCell>> {
        self.retained.scroll_owner(key)
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
    /// A record on the full renderer — the same enter/run/leave mechanics
    /// as [`SemanticCore::with_probe_reader`], but the record drives the
    /// accessibility emit walk. The record and layout descents take it.
    pub(crate) fn with_reader<R>(
        &mut self,
        core: &NodeCore,
        phase: ReaderPhase,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        if phase == ReaderPhase::Record && self.core.record_is_current(core) {
            return f(self);
        }
        let outer = self.core.enter_reader(core, phase, true);
        let out = f(self);
        self.core.leave_reader(outer);
        out
    }

    /// The flat-list view of the retained registries — see
    /// [`SemanticCore::registries`].
    pub(crate) fn registries(&mut self) -> &HitTestState {
        self.core.registries()
    }

    /// [`SemanticCore::record_platform_views`].
    pub(crate) fn record_platform_views(&self) {
        self.core.record_platform_views();
    }

    /// [`SemanticCore::current_placement`].
    pub(crate) fn current_placement(&self) -> Rc<Placement> {
        self.core.current_placement()
    }

    /// [`SemanticCore::registration_owner`].
    pub(crate) fn registration_owner(&self) -> Rc<NodeCell> {
        self.core.registration_owner()
    }

    /// [`SemanticCore::push_placement_scope`].
    pub(crate) fn push_placement_scope(&mut self, scope: ScopeDelta, clip: Option<kurbo::Rect>) {
        self.core.push_placement_scope(scope, clip);
    }

    /// [`SemanticCore::pop_placement_scope`].
    pub(crate) fn pop_placement_scope(&mut self) {
        self.core.pop_placement_scope();
    }

    /// [`SemanticCore::link_placement`].
    pub(crate) fn link_placement(&self, core: &NodeCore, delta: kurbo::Affine) {
        self.core.link_placement(core, delta);
    }

    /// The window's fixed host frames in paint order: the content root,
    /// then the context menu, anchored overlay and text overlay hosts.
    pub(crate) fn mount_roots(&self) -> [Rc<NodeCell>; 4] {
        let hosts = &self.core.presentation_hosts;
        [
            Rc::clone(&self.core.root_core.cell),
            Rc::clone(&hosts.context_menu.cell),
            Rc::clone(&hosts.anchored.cell),
            Rc::clone(&hosts.text_overlay.cell),
        ]
    }

    /// [`SemanticCore::link_placement_to`].
    pub(crate) fn link_placement_to(
        &self,
        core: &NodeCore,
        anchor: Option<Rc<Placement>>,
        delta: kurbo::Affine,
        index: u32,
    ) {
        self.core.link_placement_to(core, anchor, delta, index);
    }

    /// [`SemanticCore::window_placement`].
    #[cfg(test)]
    pub(crate) fn window_placement(&self) -> Rc<Placement> {
        self.core.window_placement()
    }

    /// Binds `key`'s interaction state to the root cell: the owner of a
    /// focusable a test seeds outside any node's record.
    #[cfg(test)]
    pub(crate) fn bind_root_owned_key(&mut self, key: &crate::renderer::input::InteractionKey) {
        let root = Rc::clone(&self.core.root);
        let _ = self.core.hit_test.interaction.bind_hover(key, &root);
    }

    /// [`SemanticCore::resolve_window_rect`].
    pub(crate) fn resolve_window_rect(&self, local: kurbo::Rect) -> kurbo::Rect {
        self.core.resolve_window_rect(local)
    }

    /// [`SemanticCore::current_hit_transform`].
    pub(crate) fn current_hit_transform(&self) -> kurbo::Affine {
        self.core.current_hit_transform()
    }

    /// A renderer drawing with `theme`, shaping text against the system font
    /// collection under `family_resolution`.
    #[must_use]
    pub fn new(
        theme: Rc<dyn crate::engine::WidgetTheme>,
        family_resolution: FontFamilyResolution,
    ) -> Self {
        Self::with_engine(theme, SessionTextEngine::system(family_resolution))
    }

    /// A renderer drawing with `theme`, shaping text against `fonts` — the
    /// collection [`crate::native_collection`] builds — under
    /// `family_resolution`.
    #[must_use]
    pub fn with_fonts(
        theme: Rc<dyn crate::engine::WidgetTheme>,
        fonts: &waterui_text::FontCollection,
        family_resolution: FontFamilyResolution,
    ) -> Self {
        Self::with_engine(
            theme,
            SessionTextEngine::from_collection(fonts, family_resolution),
        )
    }

    /// A renderer drawing with `theme`, shaping through `text` — the
    /// session's text engine the runner built it with.
    pub(crate) fn with_engine(
        theme: Rc<dyn crate::engine::WidgetTheme>,
        text: SessionTextEngine,
    ) -> Self {
        let frame_instant = Instant::now();
        Self {
            core: SemanticCore::new(frame_instant, text),
            theme,
            window_bounds: kurbo::Rect::ZERO,
            window_display_transform: kurbo::Affine::IDENTITY,
            host_redraw_handle: None,

            cherenkov_window: None,
            last_mount_stats: mount::MountStats::default(),
            #[cfg(test)]
            mirror: None,
            program: Vec::new(),
            engine_next: None,
            applied_filter_metrics: Arc::default(),
            frame_applied_filter_count: 0,
            frame_applied_filter_effect: Duration::ZERO,
            #[cfg(feature = "frame-profile")]
            frame_stage_times: FrameStageTimes::default(),
            #[cfg(feature = "frame-profile")]
            last_layout_signature: None,
            window_backdrop: None,
            compositor: render::Compositor::default(),
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
