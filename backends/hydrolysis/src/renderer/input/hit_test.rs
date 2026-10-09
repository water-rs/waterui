// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use nami::Signal as _;
use std::path::PathBuf;
use waterui::Url;
use waterui::drag_drop::{DragPayload, Files};
use waterui::gesture::PointerButton as WuiPointerButton;
use waterui_backend_core::gesture::LONG_PRESS_SLOP;
use waterui_backend_core::widget::{
    InteractionFocusBinding, ModalInteraction, WidgetInteractionState,
};
use waterui_core::interaction::{InteractionReport, InteractionState, Selected};
use waterui_graphics::input::ScrollUnit;
use waterui_layout::scroll::Axis as ScrollAxis;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DropTargetKey {
    depth: usize,
    order: usize,
}

#[derive(Clone)]
pub struct DropTarget {
    pub(crate) bounds: kurbo::Rect,
    /// The node cell that registered the target.
    pub(crate) owner: std::rc::Weak<crate::renderer::NodeCell>,
    pub(crate) key: DropTargetKey,
    pub(crate) env: Environment,
    pub(crate) destination: Rc<RefCell<DropDestination>>,
}

/// A shareable drop destination. The [`DropDestination`] is wrapped in
/// `Rc<RefCell<…>>` once (the form the hit-test stores), so a retained
/// `Wrapper` node can hold it by value and re-register the same handle on
/// every flush.
#[derive(Clone)]
pub struct DropDestinationHandles {
    destination: Rc<RefCell<DropDestination>>,
}

impl DropDestinationHandles {
    pub(crate) fn from_destination(destination: DropDestination) -> Self {
        Self {
            destination: Rc::new(RefCell::new(destination)),
        }
    }
}

pub struct ActiveDrag {
    pub(crate) payload: DragPayload,
    pub(crate) hovered_target: Option<DropTargetKey>,
}

/// An in-flight OS file drag. The path list lives here rather than in the
/// event batch: winit reports the files of a drag one `HoveredFile` /
/// `DroppedFile` event at a time, and nothing guarantees they all arrive in
/// one batch, so the list must survive the lifetime of the drag — from the
/// first `HoveredFile` until the drop is delivered or the hover cancelled.
pub struct OsFileDrag {
    /// Every path winit has reported for this drag.
    pub(crate) paths: Vec<PathBuf>,
    /// Where the drag is in its lifetime — see [`OsFileDragPhase`].
    pub(crate) phase: OsFileDragPhase,
}

/// How far an [`OsFileDrag`] has progressed. Once a `DroppedFile` arrives the
/// drag can only be `Dropping`, so the impossible "collected a file while not
/// dropping" state is unwritable.
pub enum OsFileDragPhase {
    /// Only `HoveredFile`s have arrived; no drop has begun.
    Hovering,
    /// A `DroppedFile` arrived and the drop is collecting its files.
    /// `collected_this_drain` stays set while the drain that received the
    /// last `DroppedFile` is open; the first drain that closes without
    /// adding one ends the drop and delivers.
    Dropping { collected_this_drain: bool },
}

#[derive(Clone)]
/// A gesture hit region registered this frame, mirrored from the gesture
/// engine's target list: the press path weighs a press candidate against the
/// gesture regions that outrank it, and the engine does not hand its bounds
/// back out.
pub struct GestureRegion {
    /// Hit-test rectangle in window coordinates.
    pub(crate) bounds: kurbo::Rect,
    /// The shared hit-test order taken at registration — comparable to a
    /// pointer target's `order`, so a gesture region registered after a press
    /// outranks it.
    pub(crate) order: usize,
    /// The owner chain the registration ran under — the retained nodes whose
    /// subtrees were flushing, innermost last. The ancestry the press path
    /// reads to tell a gesture registered inside a view from one attached to
    /// the view itself.
    pub(crate) owners: Vec<RetainedIdentity>,
}

#[derive(Clone)]
pub struct PointerTarget {
    pub bounds: kurbo::Rect,
    pub(crate) captures_drag: bool,
    pub(crate) depth: usize,
    pub(crate) order: usize,
    pub(crate) press_slot: Option<PressSlot>,
    /// The owner chain the registration ran under — the retained nodes whose
    /// subtrees were flushing, innermost last. The ancestry the press path
    /// reads to tell a gesture registered inside a view's subtree from one
    /// attached to a disjoint subtree painted above or below it. A gesture
    /// registered inside a strict descendant of the press's owner claims the
    /// press; one attached to the same view coexists; a target whose last
    /// owner is in a gesture region's chain is an ancestor of it and never
    /// occludes it. An empty chain — a press registered outside any retained
    /// node — is never an ancestor and is never claimed.
    pub(crate) owners: Vec<RetainedIdentity>,
    /// The node cell that registered this target — dispatch marks it (not
    /// the root) when press/drag state changes under it.
    pub(crate) owner: std::rc::Weak<crate::renderer::NodeCell>,
    /// Replayable state-layer handles for the widget owning this target, so
    /// press feedback animates without a structural rebuild.
    pub(crate) interaction: Option<Rc<InteractionLayerHandles>>,
    pub(crate) action: PointerAction,
    pub(crate) keyboard_step: Option<KeyboardStepAction>,
    pub(crate) keyboard_focusable: bool,
    pub(crate) modal: bool,
    /// The `OnKeyPress` scopes enclosing the view this target was registered
    /// from, innermost first — the chain an unconsumed key bubbles through
    /// while this target holds keyboard focus.
    pub(crate) key_handlers: Option<Rc<KeyHandlerNode>>,
}

/// An in-flight scrollbar-thumb drag: which scroll slot owns it (the handle's
/// cache key) and where inside the thumb the pointer grabbed it, in hit-space
/// pixels along the scroll axis. Held on [`HitTestState`] rather than in the
/// drag closure so a mid-drag re-registration continues seamlessly.
#[derive(Clone)]
pub struct ScrollbarDrag {
    pub key: usize,
    pub(crate) grab: f64,
    /// The cell that owns the scroll slot the drag widens — attribution
    /// for the ending mark.
    pub(crate) owner: std::rc::Weak<crate::renderer::NodeCell>,
}

#[derive(Clone)]
pub struct PendingPointerPress {
    pub slot: PressSlot,
    pub(crate) origin: kurbo::Point,
    pub(crate) starts_at: Instant,
    pub(crate) chrome_state_dependent: bool,
}

/// An armed press-and-hold on a `.context_menu` region — the context-menu
/// gesture the platforms reserve for pointers that carry no secondary
/// button. A touch or pen primary press that stays within
/// [`LONG_PRESS_SLOP`] for [`CONTEXT_MENU_HOLD_DURATION`] opens the
/// region's menu at the press point and consumes the press, so no tap fires
/// on release. The timing shares `LongPressDetector`'s semantics in
/// waterui-backend-core's gesture engine (gesture.rs) — the engine's own
/// slop constant, and the same start-instant-plus-duration deadline; the
/// detector itself is private to that crate.
#[derive(Clone, Copy)]
pub struct PendingContextMenuHold {
    /// The press origin in window hit-test space — the menu anchors there.
    pub(crate) point: kurbo::Point,
    /// The press's start instant in frame time.
    pub(crate) started_at: Instant,
}

/// How long a touch or pen press must hold to earn the gesture — the
/// hold threshold Android, iOS, GTK and Windows all share.
pub const CONTEXT_MENU_HOLD_DURATION: Duration = Duration::from_millis(500);

#[derive(Clone)]
pub struct CursorTarget {
    pub(crate) bounds: kurbo::Rect,
    pub style: CursorStyle,
    /// The node cell that registered the target.
    pub(crate) owner: std::rc::Weak<crate::renderer::NodeCell>,
}

#[derive(Clone)]
pub struct HoverTarget {
    pub(crate) bounds: kurbo::Rect,
    pub slot: HoverSlot,
    /// Replayable state-layer handles for the widget owning this target, so
    /// hover feedback animates without a structural rebuild.
    pub(crate) handles: Option<Rc<InteractionLayerHandles>>,
    pub(crate) on_enter: Option<HoverAction>,
    pub(crate) on_move: Option<HoverMoveAction>,
    pub(crate) on_exit: Option<HoverAction>,
    /// The node cell that registered the target.
    pub(crate) owner: std::rc::Weak<crate::renderer::NodeCell>,
}

#[derive(Clone)]
pub struct ScrollTarget {
    pub(crate) bounds: kurbo::Rect,
    pub action: ScrollAction,
    /// The scroll view's offset handle, ticked per frame while a smoothed
    /// wheel scroll glides toward its target.
    pub(crate) handle: crate::scroll::ScrollHandle,
    /// The node cell owning this target: a smoothed tick marks it `PAINT`
    /// so the next frame re-records this scroll view's content, nothing
    /// else's. Weak — the target and its owner drop together.
    pub(crate) owner: Weak<crate::renderer::NodeCell>,
    /// The stacking key every target carries: a scroll region painted above
    /// an embedded surface wins the wheel through the same (order, depth)
    /// priority pointer and text targets already sort by.
    pub(crate) depth: usize,
    pub(crate) order: usize,
}

/// A native subview the host platform hit-tests for itself, together with the
/// `WaterUI`-drawn content that has to take clicks away from it.
///
/// A `WKWebView` is an `AppKit` view, so it answers a click before anything
/// Hydrolysis painted over it hears about one: a snackbar, dialog or menu drawn
/// above a web view rendered correctly and was completely inert. Only the
/// renderer knows what it drew on top, so every frame it intersects the
/// interactive targets registered *after* the subview with the subview's own
/// rect and publishes the result through [`Self::sink`]; the platform's view
/// host reads that and refuses those hits.
///
/// Only targets carrying a hit-test order take part — pointer targets, text
/// inputs and other embedded browsers. Scroll and trackpad-pan targets are
/// registered without one, so their position relative to the subview cannot be
/// decided here.
#[derive(Clone)]
pub struct NativeViewOcclusion {
    /// The subview's rect in window hit-test space.
    pub bounds: kurbo::Rect,
    /// The hit-test order the subview was flushed at. Anything registered later
    /// paints above it.
    pub(crate) order: usize,
    /// Shared with the platform's view host. Rects are in window hit-test space.
    pub(crate) sink: Rc<RefCell<Vec<kurbo::Rect>>>,
}

/// Outcome of synchronizing hover targets against a pointer position.
///
/// `visual_changed` means a replayable state layer's target changed (a redraw
/// replays it); `handler_changed` means a user hover handler reported a state
/// change (schedules a retained-tree refresh like any other action).
#[derive(Debug, Default)]
pub struct HoverSync {
    pub(crate) visual_changed: bool,
    pub handler_changed: bool,
    /// Keys whose chrome or handlers actually changed — LAYOUT marks.
    /// `handler_changed` is their non-emptiness plus any action flag.
    pub(crate) chrome_keys: Vec<InteractionKey>,
    /// Keys whose hover only repainted (replayable state layers) — PAINT
    /// marks. Marks touch the changed owners, never every active key.
    pub(crate) paint_keys: Vec<InteractionKey>,
}

pub type PointerAction =
    Rc<RefCell<dyn FnMut(&mut SemanticCore, kurbo::Point, &Environment) -> bool>>;
pub type KeyboardStepAction = Rc<RefCell<dyn FnMut(bool) -> bool>>;
pub type HoverAction = Rc<RefCell<dyn FnMut(&Environment) -> bool>>;
pub type HoverMoveAction = Rc<RefCell<dyn FnMut(kurbo::Point, &Environment) -> bool>>;
pub type ScrollAction = Rc<RefCell<dyn FnMut(f32, f32, bool) -> bool>>;

/// How Enter/Space activates a keyboard-focused control — a per-runtime
/// contract, not a feature one.
#[cfg(feature = "accessibility")]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum KeyboardActivation {
    /// Press on key-down and activate on key-up — the rendered contract, with
    /// the pressed affordance held between the two.
    #[default]
    PressRelease,
    /// Dispatch `Click` on key-down — the semantic runtime's contract: with no
    /// presentation there is no pressed affordance to hold.
    Semantic,
}

// The flags track independent input facts (composition in flight, window
// blur, an armed press); they are orthogonal bits, not states to sequence,
// so a state-machine refactor would invent transitions that do not exist.
#[allow(clippy::struct_excessive_bools)]
#[derive(Default)]
pub struct HitTestState {
    /// Surfaces that own the input landing on them — embedded browsers and
    /// `GpuSurface`s whose view asked for input. Re-emitted every frame.
    pub(crate) embedded_input_targets: Vec<EmbeddedInputTarget>,
    /// The target holding the pointer capture, kept across frames.
    pub(crate) active_embedded_target: Option<EmbeddedInputTarget>,
    /// The sink holding keyboard focus, kept across frames.
    pub(crate) focused_embedded_sink: Option<Rc<dyn EmbeddedInputSink>>,
    /// The interaction identity of the surface holding embedded focus —
    /// `None` when no surface does. Kept alongside `focused_embedded_sink`
    /// so keyboard traversal and `.focused(binding)` writes resolve the
    /// same owner the pointer focused.
    pub(crate) focused_embedded_key: Option<InteractionKey>,
    /// The `.focused()` binding of the surface holding embedded focus,
    /// captured while the target is emitted so unfocus writes still reach
    /// it after the target has been truncated or unmounted.
    pub(crate) focused_embedded_binding: Option<Binding<bool>>,
    /// Whether an input-method composition session is open on the focused
    /// sink, so pre-edit updates and commits form a well-formed W3C session.
    pub(crate) embedded_composing: bool,
    /// Whether the window itself is unfocused. While it is, no surface is
    /// told it holds focus — element-focus moves stay silent until the
    /// window is reactivated, when the holder hears a single `Focus(true)`.
    pub(crate) window_blurred: bool,
    pub(crate) native_view_occlusions: Vec<NativeViewOcclusion>,
    pub(crate) pointer_targets: Vec<PointerTarget>,
    pub(crate) active_pointer_target: Option<PointerTarget>,
    pub(crate) active_pointer: Option<(u64, PointerKind)>,
    /// The button that opened the active pointer sequence. Moves and
    /// releases report it to the gesture engine, which keeps a sequence to
    /// exactly one button (water-rs/waterui#1290).
    pub(crate) active_pointer_button: Option<PointerButton>,
    pub(crate) pending_pointer_press: Option<PendingPointerPress>,
    /// A touch/pen press-and-hold running toward the context-menu
    /// threshold, armed where a secondary press would resolve a menu.
    /// Kept across frames; its deadline joins
    /// [`SemanticCore::next_gesture_deadline`].
    pub(crate) pending_context_menu_hold: Option<PendingContextMenuHold>,
    pub(crate) keyboard_focus: Option<InteractionKey>,
    /// The cell owning `keyboard_focus`'s target, captured when focus
    /// arrived so the move away marks it even once the target is gone.
    pub(crate) keyboard_focus_owner: Weak<crate::renderer::NodeCell>,
    pub(crate) keyboard_focus_binding: Option<Binding<bool>>,
    pub(crate) keyboard_focus_visible: bool,
    /// The `OnKeyPress` scopes enclosing every registration this frame —
    /// the longest common ancestor of the snapshot chains, and the chain an
    /// unconsumed key bubbles through while nothing holds keyboard focus at
    /// all. `None` both before the first registration and when no scope
    /// encloses them all; `root_key_chain_seen` tells the two apart.
    pub(crate) root_key_handlers: Option<Rc<KeyHandlerNode>>,
    /// Whether any registration has contributed a snapshot this frame.
    pub(crate) root_key_chain_seen: bool,
    /// Embedded surfaces that received a bubbled press, keyed by the key's
    /// W3C identity, so the matching release reaches the same sink rather
    /// than the focused one.
    pub(crate) bubbled_key_sinks: Vec<BubbledKeySink>,
    pub(crate) active_keyboard_target: Option<PointerTarget>,
    /// Keyboard activation semantics for this runtime — see
    /// [`KeyboardActivation`]. Only the semantic runtime switches it from the
    /// rendered default.
    #[cfg(feature = "accessibility")]
    pub(crate) keyboard_activation: KeyboardActivation,
    pub(crate) modal_interaction: Option<ModalInteraction>,
    pub(crate) active_pointer_drag_target: Option<PointerAction>,
    /// The node cell that registered `active_pointer_drag_target` —
    /// attribution for move-time marks.
    pub(crate) active_pointer_drag_owner: std::rc::Weak<crate::renderer::NodeCell>,
    pub(crate) active_pointer_drag_signature: Option<(usize, usize)>,
    /// System-back targets registered this frame, in paint order. The last
    /// entry is the frontmost stack. Cleared with the other per-frame target
    /// lists.
    pub(crate) back_targets: Vec<NavigationBackTarget>,
    /// The system-back gesture in flight. Survives `reset_scene`, the same
    /// way `active_pointer_drag_target` does.
    pub(crate) system_back: SystemBackGesture,
    /// The in-flight scrollbar-thumb drag, if any.
    pub(crate) active_scrollbar_drag: Option<ScrollbarDrag>,
    pub(crate) cursor_targets: Vec<CursorTarget>,
    pub(crate) hover_targets: Vec<HoverTarget>,
    pub(crate) drop_targets: Vec<DropTarget>,
    pub(crate) active_drag: Option<ActiveDrag>,
    /// The in-flight OS file drag, if any — see [`OsFileDrag`].
    pub(crate) os_file_drag: Option<OsFileDrag>,
    pub(crate) context_menu_targets: Vec<ContextMenuTarget>,
    pub(crate) interaction: InteractionEngine,
    pub(crate) active_press_bounds: Option<kurbo::Rect>,
    pub(crate) active_press_origin: Option<kurbo::Point>,
    /// Last observed pointer position in window hit-test space, kept across
    /// frames so embedded GPU surfaces can derive their surface-local
    /// [`PointerState`](waterui_graphics::PointerState) at composite time.
    pub(crate) pointer_position: Option<kurbo::Point>,
    /// Where the current press started, tracked independently of widget press
    /// slots: a bare `GpuSurface` has no interaction slot, but its renderer
    /// still receives hit state through `GpuFrame::pointer`.
    pub(crate) pointer_press_origin: Option<kurbo::Point>,
    pub(crate) scroll_targets: Vec<ScrollTarget>,
    /// The host's touch-drag scroll parameters, pushed by the runner each
    /// input dispatch — `None` on platforms that carry none, where a
    /// [`PointerKind::Touch`] drag never claims a scroll view.
    pub(crate) touch_scroll_config: Option<crate::platform::TouchScrollConfig>,
    /// The touch-drag scroll gesture's in-flight claim, kept across frames
    /// like `active_pointer` — driven only by `PointerKind::Touch`
    /// sequences; mouse and pen pointers never arm it.
    pub(crate) touch_scroll: TouchScrollGesture,
    /// A fling running after a claimed touch released — it outlives the
    /// gesture state and stops only when it settles or a new touch down
    /// grabs the content.
    pub(crate) touch_fling: Option<TouchFling>,
    /// The tree order of the candidate keyboard focus last rested on —
    /// kept after that focusable went hidden or unmounted so the next Tab
    /// resumes from the nearest still-visible focusable instead of
    /// restarting traversal from scratch.
    pub(crate) traversal_anchor: Option<usize>,
    /// Whether this frame's flush dropped the focused view's targets —
    /// the view went hidden or unmounted while holding focus. Read at the
    /// end of the frame: focus then relocates to the next focusable rather
    /// than dying until a pointer press re-grants it.
    pub(crate) focus_dropped_this_frame: bool,
    /// The modifier snapshot `InputEvent::ModifiersChanged` last reported —
    /// pointer targets read it at commit time because pointer events carry
    /// no modifier state of their own (toggle and Shift range selection).
    pub(crate) modifiers: Modifiers,
    /// Gesture regions live this frame, parallel to `pointer_targets` — the
    /// gesture engine owns the recognizers but not a bounds query the press
    /// path needs.
    pub(crate) gesture_regions: Vec<GestureRegion>,
    /// Overlay occluders live this frame, parallel to `pointer_targets` —
    /// `(bounds, order)` of each painted overlay panel. A press inside an
    /// occluder must not arm a content gesture beneath it, whatever kind of
    /// recognizer it carries: when the gesture engine picks its candidates
    /// at pointer-down, registrations with an order below the highest
    /// covering occluder's do not exist (water-rs/hydrolysis#260).
    pub(crate) gesture_occluders: Vec<(kurbo::Rect, usize)>,
    /// The clip stack of the paint layers currently open, in window hit-test
    /// space. Every entry is already intersected with the ones below it, so
    /// the top is the effective clip. A clip scope a program opens pushes the
    /// same rect it clips paint to and its close pops it, so a
    /// hit region flushed inside a scroll viewport can't outlive the paint
    /// clip (water-rs/hydrolysis#252).
    /// The window's logical bounds — the outermost clip every hittable point
    /// must land in. `set_window_viewport` mirrors it here each frame so the
    /// activation-point projection bounds its result the same way pointer
    /// input is bounded (water-rs/hydrolysis#27); `ZERO` before the first
    /// viewport assignment, which projects every point to an empty fragment.
    pub(crate) window_bounds: kurbo::Rect,
}

impl SemanticCore {
    /// Drops every retained registration `cell` emitted — run when the
    /// cell enters a new record and when `retire_subtree` sweeps a dead
    /// subtree. Taking the cell's bucket is the whole purge: entries live
    /// per owner, so the retire is O(its count) with no registry sweep.
    pub(crate) fn purge_registrations(&self, cell: &Rc<crate::renderer::NodeCell>) {
        let Some(regs) = cell.registrations.borrow_mut().take() else {
            return;
        };
        for entry in &regs.scroll_targets {
            self.retained
                .release_scroll_owner(entry.payload.handle.cache_key(), cell);
        }
        self.retained.stale.set(true);
    }

    /// Rebuilds the flat lists consumers read from the retained
    /// registries: every live owner's bucket contributes its entries, each
    /// resolves its region through the placement chain it was anchored at,
    /// and the merged paint sort assigns the `order` rank — the same
    /// globally comparable sequence dev's per-frame registration counter
    /// minted. Each kind passes the gate of its [`HitClasses`] on the
    /// structural chain, keeping dev's two mechanisms distinct:
    ///
    /// - a *removal* — `Hittable(false)` truncated the input kinds
    ///   (pointer targets with their occluder and scrollbar presses,
    ///   gestures, cursor, hover, scroll, text and embedded inputs); the
    ///   context-menu preview truncated those plus drop, context-menu and
    ///   native-occlusion entries. A hovered target a removal drops has its
    ///   hover cleared here, as the truncation did.
    /// - the *alpha gate* — dev's `hit_test_opacity` (inactive pages,
    ///   exiting overlays, entries mid-transition) withheld the input
    ///   kinds, drop and context-menu targets and gesture occluders.
    ///
    /// Back targets, modal scopes and platform views pass no gate. Drop,
    /// context-menu, occluder, occlusion and platform-view geometry resolves
    /// through the paint chain; platform views stage into
    /// `materialized_platform_views` for the one frame-end `record` step.
    /// No-op while neither a registry write nor a placement write has
    /// landed since the last materialization.
    #[expect(
        clippy::too_many_lines,
        reason = "each kind materializes with the same resolve-and-rank pattern; splitting it would hide that the lists are one merged order"
    )]
    pub(crate) fn materialize_registries(&mut self) {
        // Staged = resolved window-space geometry plus the paint-order key
        // that ranks it; `order` fields on the flat records are only
        // comparable because the materialized rank is assigned after a
        // global sort across every kind.
        struct Staged<T> {
            entry_order: mount::PaintOrder,
            payload: T,
            window: kurbo::Rect,
        }

        // Platform-view records defer their `table.record` to the frame-end
        // step — the staged sort keeps the one-write-per-frame contract.
        struct StagedPlatform {
            order: mount::PaintOrder,
            table: Rc<RefCell<crate::platform_view::PlatformViewTable>>,
            placement: crate::platform_view::PlatformViewPlacement,
            frame: kurbo::Rect,
            clip: kurbo::Rect,
        }

        let epoch = self.placement_clock.epoch();
        let stale = self.retained.stale.replace(false);
        if !stale && epoch == self.materialized_epoch {
            return;
        }
        // A kind the structural chain admits resolves its geometry through
        // the paint chain.
        let paint_gate = |placement: &Placement,
                          class: HitClasses|
         -> Option<(kurbo::Affine, Option<kurbo::Rect>)> {
            placement.admits(class).then(|| {
                let (transform, clip, _) = placement.resolved_chain(false);
                (transform, clip)
            })
        };
        let now = self.frame_instant();
        let mut cleared_hover: Vec<Weak<crate::renderer::NodeCell>> = Vec::new();

        self.materialized_epoch = epoch;

        let mut orders: Vec<mount::PaintOrder> = Vec::new();
        let mut staged_pointer: Vec<Staged<PointerTarget>> = Vec::new();
        let mut staged_gestures: Vec<Staged<mount::RegisteredGesture>> = Vec::new();
        let mut staged_occluders: Vec<(mount::PaintOrder, kurbo::Rect)> = Vec::new();
        let mut staged_cursor: Vec<Staged<CursorTarget>> = Vec::new();
        let mut staged_hover: Vec<Staged<HoverTarget>> = Vec::new();
        let mut staged_drop: Vec<Staged<DropTarget>> = Vec::new();
        let mut staged_menus: Vec<Staged<ContextMenuTarget>> = Vec::new();
        let mut staged_scroll: Vec<Staged<ScrollTarget>> = Vec::new();
        let mut staged_text: Vec<Staged<TextInputTarget>> = Vec::new();
        let mut staged_embedded: Vec<Staged<EmbeddedInputTarget>> = Vec::new();
        let mut staged_occlusions: Vec<Staged<NativeViewOcclusion>> = Vec::new();
        let mut staged_platform: Vec<StagedPlatform> = Vec::new();
        let mut staged_back: Vec<Staged<crate::renderer::NavigationBackTarget>> = Vec::new();
        let mut modal: Option<(
            mount::PaintOrder,
            waterui_backend_core::widget::ModalInteraction,
        )> = None;

        for cell in self.retained.owners() {
            let mut slot = cell.registrations.borrow_mut();
            let Some(regs) = slot.as_mut() else {
                continue;
            };

            for entry in &regs.pointer_targets {
                if let Some(resolved) = entry.region.placement.resolve_hit(entry.region.local) {
                    let mut payload = entry.payload.clone();
                    payload.bounds = resolved.rect;
                    orders.push(entry.order.clone());
                    staged_pointer.push(Staged {
                        entry_order: entry.order.clone(),
                        payload,
                        window: resolved.rect,
                    });
                }
            }

            for entry in &regs.gesture_regions {
                if let Some(resolved) = entry.region.placement.resolve_hit(entry.region.local) {
                    orders.push(entry.order.clone());
                    staged_gestures.push(Staged {
                        entry_order: entry.order.clone(),
                        payload: mount::RegisteredGesture {
                            owner: entry.payload.owner.clone(),
                            target: entry.payload.target.clone(),
                            owners: entry.payload.owners.clone(),
                        },
                        window: resolved.rect,
                    });
                }
            }

            for entry in &regs.gesture_occluders {
                if let Some((transform, clip)) =
                    paint_gate(&entry.region.placement, HitClasses::GESTURE_OCCLUDER)
                {
                    let rect = crate::renderer::transformed_rect(transform, entry.region.local);
                    let rect = clip.map_or(rect, |clip| rect.intersect(clip));
                    orders.push(entry.order.clone());
                    staged_occluders.push((entry.order.clone(), rect));
                }
            }

            for entry in &regs.cursor_targets {
                if let Some(resolved) = entry.region.placement.resolve_hit(entry.region.local) {
                    let mut payload = entry.payload.clone();
                    payload.bounds = resolved.rect;
                    orders.push(entry.order.clone());
                    staged_cursor.push(Staged {
                        entry_order: entry.order.clone(),
                        payload,
                        window: resolved.rect,
                    });
                }
            }

            // A hover target outliving its widget's bind is dead weight:
            // the engine drops the slot's state at frame end, so staging
            // an unbound slot would leave a target that can only panic on
            // read. Entries gated out by the hit chain simply miss the
            // flat list; a target a removal gate dropped has its hover
            // cleared below, matching dev's truncation.
            regs.hover_targets.retain(|entry| {
                self.hit_test
                    .interaction
                    .key_active(&entry.payload.slot.key)
            });
            for entry in &regs.hover_targets {
                if entry.region.placement.removes(HitClasses::INPUT) {
                    let slot = &entry.payload.slot;
                    let handles = entry.payload.handles.as_ref();
                    if self.hit_test.interaction.hovering(slot)
                        || handles.is_some_and(|handles| handles.hovering())
                    {
                        self.hit_test.interaction.set_hovering(slot, false);
                        if let Some(handles) = handles {
                            handles.set_hovering(false, now);
                        }
                        cleared_hover.push(entry.owner.clone());
                    }
                    continue;
                }
                if let Some(resolved) = entry.region.placement.resolve_hit(entry.region.local) {
                    let mut payload = entry.payload.clone();
                    payload.bounds = resolved.rect;
                    orders.push(entry.order.clone());
                    staged_hover.push(Staged {
                        entry_order: entry.order.clone(),
                        payload,
                        window: resolved.rect,
                    });
                }
            }

            for entry in &regs.drop_targets {
                if let Some((transform, clip)) =
                    paint_gate(&entry.region.placement, HitClasses::DROP)
                {
                    let mut payload = entry.payload.clone();
                    payload.bounds =
                        crate::renderer::transformed_rect(transform, entry.region.local);
                    if let Some(clip) = clip {
                        payload.bounds = payload.bounds.intersect(clip);
                    }
                    orders.push(entry.order.clone());
                    let window = payload.bounds;

                    staged_drop.push(Staged {
                        entry_order: entry.order.clone(),
                        payload,
                        window,
                    });
                }
            }

            for entry in &regs.context_menu_targets {
                if let Some((transform, clip)) =
                    paint_gate(&entry.region.placement, HitClasses::CONTEXT_MENU)
                {
                    let mut payload = entry.payload.clone();
                    payload.bounds =
                        crate::renderer::transformed_rect(transform, entry.region.local);
                    if let Some(clip) = clip {
                        payload.bounds = payload.bounds.intersect(clip);
                    }
                    orders.push(entry.order.clone());
                    let window = payload.bounds;

                    staged_menus.push(Staged {
                        entry_order: entry.order.clone(),
                        payload,
                        window,
                    });
                }
            }

            for entry in &regs.scroll_targets {
                if let Some(resolved) = entry.region.placement.resolve_hit(entry.region.local) {
                    let mut payload = entry.payload.clone();
                    payload.bounds = resolved.rect;
                    orders.push(entry.order.clone());
                    staged_scroll.push(Staged {
                        entry_order: entry.order.clone(),
                        payload,
                        window: resolved.rect,
                    });
                }
            }

            // Text inputs keep four rectangles (field, cursor area, text
            // bounds, text clip) under the hit chain plus the clip
            // intersect, behind the input gate.
            for entry in &regs.text_input_targets {
                if !entry.region.placement.admits(HitClasses::INPUT) {
                    continue;
                }
                let (transform, clip, _) = entry.region.placement.resolved_chain(true);
                let mut target = entry.payload.clone();
                let local = entry.region.local;
                target.bounds = crate::renderer::transformed_rect(transform, local);
                // The laid-out frame resolves in the same window
                // coordinates without the clip: the §7.1 clearance reads
                // it for the field's real rectangle once `bounds` has
                // degenerated to a clip edge.
                target.frame = target.bounds;
                if let Some(clip) = clip {
                    target.bounds = target.bounds.intersect(clip);
                }
                target.cursor_area =
                    crate::renderer::transformed_rect(transform, target.cursor_area);
                target.text_bounds =
                    crate::renderer::transformed_rect(transform, target.text_bounds);
                target.text_clip_bounds =
                    crate::renderer::transformed_rect(transform, target.text_clip_bounds);
                orders.push(entry.order.clone());
                staged_text.push(Staged {
                    entry_order: entry.order.clone(),
                    payload: target,
                    window: local,
                });
            }

            // Embedded input targets resolve to the inverse transform +
            // hit clip.
            for entry in &regs.embedded_input_targets {
                if !entry.region.placement.admits(HitClasses::INPUT) {
                    continue;
                }
                let (transform, clip, _) = entry.region.placement.resolved_chain(true);
                let mut target = entry.payload.clone();
                target.inverse_transform = transform.inverse();
                target.hit_clip = clip;
                orders.push(entry.order.clone());
                staged_embedded.push(Staged {
                    entry_order: entry.order.clone(),
                    payload: target,
                    window: entry.region.local,
                });
            }

            for entry in &regs.native_view_occlusions {
                if let Some((transform, clip)) =
                    paint_gate(&entry.region.placement, HitClasses::NATIVE_OCCLUSION)
                {
                    let mut payload = entry.payload.clone();
                    payload.bounds =
                        crate::renderer::transformed_rect(transform, entry.region.local);
                    if let Some(clip) = clip {
                        payload.bounds = payload.bounds.intersect(clip);
                    }
                    orders.push(entry.order.clone());
                    let window = payload.bounds;

                    staged_occlusions.push(Staged {
                        entry_order: entry.order.clone(),
                        payload,
                        window,
                    });
                }
            }

            // Platform views place exactly as dev's per-frame record did:
            // the paint chain supplies transform and clip with no hit or
            // alpha gate — a view under an unhittable scope still lands in
            // its table, and `visible` follows the clip.
            for entry in &regs.platform_view_placements {
                let (transform, clip, _alpha) = entry.region.placement.resolved_chain(false);
                let frame = crate::renderer::transformed_rect(transform, entry.region.local);
                let clipped = clip.map_or(frame, |clip| frame.intersect(clip));
                orders.push(entry.order.clone());
                staged_platform.push(StagedPlatform {
                    order: entry.order.clone(),
                    table: Rc::clone(&entry.payload.table),
                    placement: entry.payload.placement.clone(),
                    frame,
                    clip: clipped,
                });
            }

            // Back targets carry no geometry — order is their only
            // resolved datum, so they stage into the rank sort like
            // everything else. No gate applies: dev registered them under
            // neither its truncation nor its hit opacity.
            for entry in &regs.back_targets {
                orders.push(entry.order.clone());
                staged_back.push(Staged {
                    entry_order: entry.order.clone(),
                    payload: entry.payload.clone(),
                    window: entry.region.local,
                });
            }

            // Modal scope: the frontmost entry, ungated as on dev.
            for entry in &regs.modal_scope {
                if modal.as_ref().is_none_or(|(order, _)| entry.order > *order) {
                    modal = Some((entry.order.clone(), entry.payload.clone()));
                }
            }
        }

        // A hover a removal gate cleared repaints its owner, the same mark
        // `HoverSync` raises for a hover change.
        for owner in cleared_hover {
            Self::mark_owner(&owner, Dirty::LAYOUT);
        }

        // The merged paint sort gives every materialized `order` — the same
        // globally comparable sequence the per-frame counter minted.
        orders.sort();
        orders.dedup();
        let rank = |order: &mount::PaintOrder| -> usize {
            orders
                .binary_search(order)
                .expect("hydrolysis retained order missing from merged sort")
        };

        let pointer_targets = &mut self.hit_test.pointer_targets;
        pointer_targets.clear();
        for staged in staged_pointer {
            let mut target = staged.payload;
            target.order = rank(&staged.entry_order);
            pointer_targets.push(target);
        }

        // Rebuild the gesture engine's target list in the same rank: the
        // registered recognizers persist in the retained payloads, so an
        // in-flight gesture survives both re-records and reparents.
        let gesture_regions = &mut self.hit_test.gesture_regions;
        gesture_regions.clear();
        let mut targets = Vec::new();
        self.gesture_engine.swap_targets(&mut targets);
        targets.clear();
        for staged in staged_gestures {
            let order = rank(&staged.entry_order);
            let mut target = staged.payload.target.with_bounds_depth_and_group(
                staged.window,
                staged.payload.target.depth,
                staged.payload.target.group_id,
            );
            target.order = order;
            self.gesture_engine.register_existing_target(target);
            gesture_regions.push(crate::renderer::GestureRegion {
                bounds: staged.window,
                order,
                owners: staged.payload.owners.clone(),
            });
        }

        let gesture_occluders = &mut self.hit_test.gesture_occluders;
        gesture_occluders.clear();
        for (order, rect) in staged_occluders {
            gesture_occluders.push((rect, rank(&order)));
        }

        let cursor_targets = &mut self.hit_test.cursor_targets;
        cursor_targets.clear();
        cursor_targets.extend(staged_cursor.into_iter().map(|staged| staged.payload));

        let hover_targets = &mut self.hit_test.hover_targets;
        hover_targets.clear();
        hover_targets.extend(staged_hover.into_iter().map(|staged| staged.payload));

        let drop_targets = &mut self.hit_test.drop_targets;
        drop_targets.clear();
        for staged in staged_drop {
            let mut target = staged.payload;
            target.key.order = rank(&staged.entry_order);
            drop_targets.push(target);
        }

        let context_menu_targets = &mut self.hit_test.context_menu_targets;
        context_menu_targets.clear();
        for staged in staged_menus {
            let mut target = staged.payload;
            target.order = rank(&staged.entry_order);
            context_menu_targets.push(target);
        }

        let scroll_targets = &mut self.hit_test.scroll_targets;
        scroll_targets.clear();
        for staged in staged_scroll {
            let mut target = staged.payload;
            target.order = rank(&staged.entry_order);
            scroll_targets.push(target);
        }

        let text_input_targets = &mut self.text_editing.text_input_targets;
        text_input_targets.clear();
        for staged in staged_text {
            let mut target = staged.payload;
            target.order = rank(&staged.entry_order);
            text_input_targets.push(target);
        }

        let embedded_input_targets = &mut self.hit_test.embedded_input_targets;
        embedded_input_targets.clear();
        for staged in staged_embedded {
            let mut target = staged.payload;
            target.order = rank(&staged.entry_order);
            embedded_input_targets.push(target);
        }

        let native_view_occlusions = &mut self.hit_test.native_view_occlusions;
        native_view_occlusions.clear();
        for staged in staged_occlusions {
            let mut occlusion = staged.payload;
            occlusion.order = rank(&staged.entry_order);
            native_view_occlusions.push(occlusion);
        }

        // Platform-view sink records in paint order — staged, then written
        // once per frame by `record_platform_views` at frame end.
        staged_platform.sort_by(|a, b| a.order.cmp(&b.order));
        self.materialized_platform_views.clear();
        self.materialized_platform_views
            .extend(staged_platform.into_iter().map(|staged| {
                let clipped = staged.clip;
                let mut placement = staged.placement;
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "window coordinates fit f32 — same cast dev's placement math used"
                )]
                {
                    placement.x = staged.frame.x0 as f32;
                    placement.y = staged.frame.y0 as f32;
                    placement.width = staged.frame.width() as f32;
                    placement.height = staged.frame.height() as f32;
                    placement.clip = (clipped != staged.frame).then(|| {
                        [
                            clipped.x0 as f32,
                            clipped.y0 as f32,
                            clipped.width() as f32,
                            clipped.height() as f32,
                        ]
                    });
                    placement.order = rank(&staged.order) as u32;
                }
                placement.visible = !clipped.is_zero_area();
                crate::renderer::MaterializedPlatformView {
                    table: staged.table,
                    placement,
                }
            }));

        self.hit_test.back_targets = staged_back
            .into_iter()
            .map(|staged| staged.payload)
            .collect();

        self.hit_test.modal_interaction = modal.map(|(_, payload)| payload);

        let mut chain_seen = false;
        let mut chain: Option<Rc<KeyHandlerNode>> = None;
        let mut fold = |key_handlers: &Option<Rc<KeyHandlerNode>>| {
            if chain_seen {
                chain = common_key_handler_scope(chain.take(), key_handlers.clone());
            } else {
                chain_seen = true;
                chain.clone_from(key_handlers);
            }
        };
        for target in &self.hit_test.pointer_targets {
            fold(&target.key_handlers);
        }
        for target in &self.hit_test.embedded_input_targets {
            fold(&target.key_handlers);
        }
        for target in &self.text_editing.text_input_targets {
            fold(&target.key_handlers);
        }
        // The semantic walk emits no pointer/text targets — its key scopes
        // live on the focus links instead, so they fold in too. Same
        // common-ancestor fold as dev's per-registration one.
        #[cfg(feature = "accessibility")]
        for chain_snapshot in self.accessibility.focus_key_handlers.values() {
            fold(chain_snapshot);
        }
        drop(fold);
        self.hit_test.root_key_chain_seen = chain_seen;
        self.hit_test.root_key_handlers = chain;

        // The pointer-drag liveness check runs here, once per
        // materialization: a target whose owner retired or gated out has
        // left the flat list, so the stale drag is dropped then.
        self.ensure_active_pointer_drag_target_is_live();
        self.end_orphaned_touch_input();
    }

    /// Ends the touch input that outlives its target: a fling whose scroll
    /// view no registered `ScrollTarget` holds any more, and a delayed
    /// press whose key no live node binds. Both end with their owner, so
    /// the next fling tick or release never resolves a retired owner; the
    /// press drops the tap it armed with it, as a touch move cancels one.
    fn end_orphaned_touch_input(&mut self) {
        if self.hit_test.touch_fling.as_ref().is_some_and(|fling| {
            self.try_scroll_target_owner(fling.handle().cache_key())
                .is_none()
        }) {
            self.hit_test.touch_fling = None;
        }
        if self
            .hit_test
            .pending_pointer_press
            .as_ref()
            .is_some_and(|pending| {
                !self.hit_test.interaction.key_active(&pending.slot.key)
                    || self.try_key_owner(&pending.slot.key).is_none()
            })
        {
            self.hit_test.pending_pointer_press = None;
            self.hit_test.active_pointer_target = None;
            self.hit_test.active_press_bounds = None;
            self.hit_test.active_press_origin = None;
        }
    }
}

impl HitTestState {
    /// The flat lists are materialized views of the retained registries —
    /// `reset_scene` no longer clears them; the interaction engine's own
    /// per-frame begin stays, since its handles live outside the registries.
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.focus_dropped_this_frame = false;
        self.interaction.begin_rebuild_frame();
    }

    pub(crate) fn finish_rebuild_frame(&mut self, text_inputs: &[TextInputTarget]) {
        self.interaction.finish_rebuild_frame();
        self.retire_absent_embedded_focus();
        self.publish_native_view_occlusion(text_inputs);
    }

    /// Moves embedded focus to `index` — the single transition every
    /// focus path (pointer press, keyboard traversal, `.focused(binding)`,
    /// structural retirement) shares. Sends `Focus(false)` to the outgoing
    /// sink — cancelling an open composition first — then `Focus(true)` to
    /// the incoming one, and mirrors the move into the surfaces'
    /// `.focused()` bindings where they exist. `None` releases focus
    /// entirely. Returns whether the focused owner changed.
    pub(crate) fn set_embedded_focus_index(&mut self, index: Option<usize>) -> bool {
        let next_key = index.map(|i| self.embedded_input_targets[i].interaction_key.clone());
        if self.focused_embedded_key == next_key {
            return false;
        }
        if let Some(binding) = self.focused_embedded_binding.take() {
            binding.set(false);
        }
        if let Some(sink) = self.focused_embedded_sink.take() {
            if self.embedded_composing {
                self.embedded_composing = false;
                sink.composition_cancel();
            }
            if !self.window_blurred {
                sink.set_focus(false);
            }
        }
        self.focused_embedded_key = next_key;
        self.focused_embedded_sink = index.map(|i| Rc::clone(&self.embedded_input_targets[i].sink));
        self.focused_embedded_binding =
            index.and_then(|i| self.embedded_input_targets[i].focus_binding.clone());
        if let Some(binding) = self.focused_embedded_binding.as_ref() {
            binding.set(true);
        }
        if let Some(sink) = self.focused_embedded_sink.as_ref()
            && !self.window_blurred
        {
            sink.set_focus(true);
        }
        true
    }

    /// Drops keyboard focus and pointer capture held by an embedded surface
    /// that is no longer in the scene.
    ///
    /// `embedded_input_targets` is emitted afresh every frame, so a focused
    /// sink missing from it has left the tree. Nothing else clears it: focus
    /// was only released by a pointer-down that missed every surface, and the
    /// sink kept the pruned page alive, so after navigating away from a web
    /// view every keystroke went to an invisible document for the rest of the
    /// session and no shortcut, text field or Escape received anything again.
    ///
    /// Presence is decided by [`EmbeddedInputSink::identity`], not by the sink
    /// allocation: a frame registers freshly built sinks for the same
    /// surfaces, so comparing allocations would retire focus every frame.
    fn retire_absent_embedded_focus(&mut self) {
        let present = |sink: &Rc<dyn EmbeddedInputSink>| {
            self.embedded_input_targets
                .iter()
                .any(|target| target.sink.identity() == sink.identity())
        };
        let focus_left = self
            .focused_embedded_sink
            .as_ref()
            .is_some_and(|sink| !present(sink));
        let capture_left = self
            .active_embedded_target
            .as_ref()
            .is_some_and(|target| !present(&target.sink));
        if focus_left {
            // Only the embedded slot retires here: the surface's
            // `InteractionKey` is also dead to
            // `validate_focused_text_input_after_flush`, which clears the
            // keyboard-focus slot through the shared setter so the focus
            // binding and the semantic focus node release together. The
            // end of the frame relocates the freed focus.
            self.focus_dropped_this_frame = true;
            self.set_embedded_focus_index(None);
        }
        if capture_left {
            self.active_embedded_target = None;
        }
    }

    /// Publishes, for each registered native subview, the rects where
    /// `WaterUI`-drawn interactive content sits above it.
    fn publish_native_view_occlusion(&self, text_inputs: &[TextInputTarget]) {
        for occlusion in &self.native_view_occlusions {
            let above = |order: usize, bounds: kurbo::Rect| {
                (order > occlusion.order)
                    .then(|| bounds.intersect(occlusion.bounds))
                    .filter(|overlap| !overlap.is_zero_area())
            };
            let rects: Vec<kurbo::Rect> = self
                .pointer_targets
                .iter()
                .filter_map(|target| above(target.order, target.bounds))
                .chain(
                    text_inputs
                        .iter()
                        .filter_map(|target| above(target.order, target.bounds)),
                )
                .chain(self.embedded_input_targets.iter().filter_map(|target| {
                    above(
                        target.order,
                        target
                            .inverse_transform
                            .inverse()
                            .transform_rect_bbox(target.local_bounds),
                    )
                }))
                .collect();
            occlusion.sink.replace(rects);
        }
    }

    pub(crate) fn cursor_style_at(&self, point: kurbo::Point) -> CursorStyle {
        self.cursor_targets
            .iter()
            .rev()
            .find(|target| target.bounds.contains(point))
            .map_or(CursorStyle::Arrow, |target| target.style)
    }

    pub(crate) fn sync_hover_targets(
        &mut self,
        point: kurbo::Point,
        env: &Environment,
        dispatch_move: bool,
        now: Instant,
    ) -> HoverSync {
        let mut sync = HoverSync::default();
        for target in &mut self.hover_targets {
            // The slot's bind can lapse after the target staged (a frame-end
            // prune that ran between materialization and this dispatch):
            // an unbound slot reads as not hovered.
            if !self.interaction.key_active(&target.slot.key) {
                continue;
            }
            let contains = target.bounds.contains(point);
            let slot_hovering = self.interaction.hovering(&target.slot);
            if contains != slot_hovering {
                self.interaction.set_hovering(&target.slot, contains);
                let mut chrome = false;
                if let Some(handles) = &target.handles {
                    // Replayable state layer: the hover alpha animates through
                    // its retained draw, no structural rebuild needed unless
                    // the widget's chrome samples interaction state directly.
                    handles.set_hovering(contains, now);
                    if handles.chrome_state_dependent() {
                        chrome = true;
                    } else {
                        sync.visual_changed = true;
                    }
                }
                if contains {
                    if let Some(on_enter) = target.on_enter.as_mut() {
                        chrome |= (on_enter.borrow_mut())(env);
                    }
                } else if let Some(on_exit) = target.on_exit.as_mut() {
                    chrome |= (on_exit.borrow_mut())(env);
                }
                if chrome {
                    sync.chrome_keys.push(target.slot.key.clone());
                } else {
                    sync.paint_keys.push(target.slot.key.clone());
                }
            }
            if contains
                && dispatch_move
                && let Some(on_move) = target.on_move.as_mut()
                && (on_move.borrow_mut())(point, env)
            {
                sync.chrome_keys.push(target.slot.key.clone());
            }
            sync.handler_changed = !sync.chrome_keys.is_empty();
        }
        sync
    }

    /// The topmost drop target under `point` that accepts `payload`. A
    /// destination that does not accept the drag's payload is skipped — it is
    /// neither hovered nor delivered.
    fn topmost_drop_target_index_at_point(
        &self,
        point: kurbo::Point,
        payload: &DragPayload,
    ) -> Option<usize> {
        self.drop_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| {
                target.bounds.contains(point) && target.destination.borrow().accepts(payload)
            })
            .max_by(|(left_index, left), (right_index, right)| {
                SemanticCore::target_hit_priority(left.key.depth, left.key.order, *left_index).cmp(
                    &SemanticCore::target_hit_priority(
                        right.key.depth,
                        right.key.order,
                        *right_index,
                    ),
                )
            })
            .map(|(index, _)| index)
    }

    fn drop_target_with_key(&self, key: DropTargetKey) -> Option<DropTarget> {
        self.drop_targets
            .iter()
            .find(|target| target.key == key)
            .cloned()
    }
}

impl SemanticCore {
    /// The environment a drop destination's callbacks run in: the
    /// environment captured where the destination was declared, layered over
    /// the runtime's.
    fn drop_target_env(target: &DropTarget, runtime_env: &Environment) -> Environment {
        target.env.layered_on(runtime_env)
    }

    fn sync_active_drag_hover(&mut self, point: kurbo::Point, env: &Environment) -> bool {
        let Some(active_drag) = self.hit_test.active_drag.as_ref() else {
            return false;
        };
        let payload = active_drag.payload.clone();
        let previous_key = active_drag.hovered_target;
        let current_target = self
            .hit_test
            .topmost_drop_target_index_at_point(point, &payload)
            .map(|index| self.hit_test.drop_targets[index].clone());
        let current_key = current_target.as_ref().map(|target| target.key);
        if previous_key == current_key {
            return false;
        }

        let previous_target = previous_key.and_then(|key| self.hit_test.drop_target_with_key(key));
        if let Some(active_drag) = self.hit_test.active_drag.as_mut() {
            active_drag.hovered_target = current_key;
        }

        let exit_changed = if let Some(target) = previous_target {
            let action_env = Self::drop_target_env(&target, env);
            target.destination.borrow_mut().exit(&action_env);
            Self::mark_owner(&target.owner, Dirty::LAYOUT);
            true
        } else {
            previous_key.is_some() || current_key.is_some()
        };
        if let Some(target) = current_target {
            let action_env = Self::drop_target_env(&target, env);
            target.destination.borrow_mut().enter(&action_env);
            Self::mark_owner(&target.owner, Dirty::LAYOUT);
            true
        } else {
            exit_changed
        }
    }

    fn begin_or_update_drag(
        &mut self,
        payload: DragPayload,
        point: kurbo::Point,
        env: &Environment,
    ) -> bool {
        if let Some(active_drag) = self.hit_test.active_drag.as_mut() {
            active_drag.payload = payload;
        } else {
            self.hit_test.active_drag = Some(ActiveDrag {
                payload,
                hovered_target: None,
            });
        }
        self.sync_active_drag_hover(point, env)
    }

    fn finish_active_drag(&mut self, point: kurbo::Point, env: &Environment) -> bool {
        let Some(active_drag) = self.hit_test.active_drag.take() else {
            return false;
        };
        let drop_target = self
            .hit_test
            .topmost_drop_target_index_at_point(point, &active_drag.payload)
            .map(|index| self.hit_test.drop_targets[index].clone());
        let exit_target = active_drag
            .hovered_target
            .and_then(|key| self.hit_test.drop_target_with_key(key));

        let delivered = if let Some(target) = drop_target {
            let action_env = Self::drop_target_env(&target, env);
            target
                .destination
                .borrow_mut()
                .deliver(active_drag.payload, &action_env);
            Self::mark_owner(&target.owner, Dirty::LAYOUT);
            true
        } else {
            false
        };
        if let Some(target) = exit_target {
            let action_env = Self::drop_target_env(&target, env);
            target.destination.borrow_mut().exit(&action_env);
            Self::mark_owner(&target.owner, Dirty::LAYOUT);
            true
        } else {
            delivered
        }
    }

    fn cancel_active_drag(&mut self, env: &Environment) -> bool {
        self.hit_test.os_file_drag = None;
        let Some(active_drag) = self.hit_test.active_drag.take() else {
            return false;
        };
        let exit_target = active_drag
            .hovered_target
            .and_then(|key| self.hit_test.drop_target_with_key(key));
        if let Some(target) = exit_target {
            let action_env = Self::drop_target_env(&target, env);
            target.destination.borrow_mut().exit(&action_env);
            Self::mark_owner(&target.owner, Dirty::LAYOUT);
            return true;
        }
        false
    }

    /// The drag's collected files as a [`Files`] payload of `file://` URLs.
    /// `Url::from_file_path` is infallible on any bytes, so non-UTF-8 paths
    /// cannot panic here.
    fn os_file_payload(paths: &[PathBuf]) -> DragPayload {
        DragPayload::new(Files::new(paths.iter().map(Url::from_file_path)))
    }

    /// Appends `path` to the drag's collected files if it is not already
    /// there — winit reports every file of a drag twice, once as
    /// `HoveredFile` and again as `DroppedFile`, so a naive append would
    /// deliver each URL twice.
    fn collect_os_file_drag_path(&mut self, path: PathBuf) -> &mut OsFileDrag {
        let state = self
            .hit_test
            .os_file_drag
            .get_or_insert_with(|| OsFileDrag {
                paths: Vec::new(),
                phase: OsFileDragPhase::Hovering,
            });
        if !state.paths.contains(&path) {
            state.paths.push(path);
        }
        state
    }

    /// The pointer position the host reported to the runner outside the
    /// input-event stream.
    ///
    /// OS file events carry no coordinates, and a platform may keep cursor
    /// events out of the stream entirely while an external drag owns the
    /// pointer, so the runner seeds the drag's position with the host's live
    /// answer before dispatching them (water-rs/hydrolysis#127).
    pub fn note_pointer_position(&mut self, x: f32, y: f32) {
        self.hit_test.pointer_position = Some(kurbo::Point::new(f64::from(x), f64::from(y)));
    }

    /// A file of an OS drag hovered the window (winit `HoveredFile`, one
    /// event per file). The path joins the drag's collected files and the
    /// [`Files`] payload is rebuilt over all of them, so a destination that
    /// accepts [`Files`] sees the hover and one that does not is never
    /// entered.
    ///
    /// winit's file events carry no position; the hover resolves at the last
    /// position the window saw (`hit_test.pointer_position`), which the
    /// runner refreshes from the host's live answer before dispatching the
    /// event — see [`Self::note_pointer_position`].
    pub fn handle_file_hovered(&mut self, path: PathBuf, env: &Environment) -> bool {
        self.registries();
        let payload = {
            let state = self.collect_os_file_drag_path(path);
            Self::os_file_payload(&state.paths)
        };
        let Some(point) = self.hit_test.pointer_position else {
            self.hit_test.active_drag = Some(ActiveDrag {
                payload,
                hovered_target: None,
            });
            return false;
        };
        self.begin_or_update_drag(payload, point, env)
    }

    /// A file of an OS drag was dropped on the window (winit `DroppedFile`,
    /// one event per file). The path joins the drag's collected files; the
    /// drop itself is delivered once by [`Self::finish_os_file_drop`], at the
    /// end of the first drain that adds no more files.
    pub fn handle_file_dropped(&mut self, path: PathBuf) {
        self.registries();
        let state = self.collect_os_file_drag_path(path);
        state.phase = OsFileDragPhase::Dropping {
            collected_this_drain: true,
        };
    }

    /// Whether an OS file drop is collecting files and owes the runner one
    /// more drain to deliver it — the runner turns this into a scheduled
    /// follow-up pump so the drop lands even if no further input arrives.
    pub(crate) fn os_file_drop_pending(&self) -> bool {
        matches!(
            self.hit_test.os_file_drag.as_ref().map(|drag| &drag.phase),
            Some(OsFileDragPhase::Dropping { .. })
        )
    }

    /// Ends an OS file drag's drop if one has fully landed.
    ///
    /// winit emits no drop-end event — only `DroppedFile` per file — so a
    /// drop ends with the first drain that adds no file. That is sound
    /// because every backend emits a drop's whole file set inside a single
    /// platform callback, which lands entirely within one drain: on X11 the
    /// `XdndDrop` handler loops `for path in path_list` emitting one
    /// `DroppedFile` each (winit 0.30.13
    /// `platform_impl/linux/x11/event_processor.rs`), on macOS
    /// `performDragOperation:` queues every file at once
    /// (`platform_impl/macos/window_delegate.rs`), and on Windows
    /// `IDropTarget::Drop` iterates the HDROP's files in one call
    /// (`platform_impl/windows/drop_handler.rs`). Keeping the collection on
    /// the drag rather than the batch means a set that does straddle drains
    /// still delivers once, with every file it carried.
    ///
    /// The drain that ends a drop only exists because the runner asked for
    /// it: when this leaves the drag [`OsFileDragPhase::Dropping`], the
    /// caller requests one follow-up pump through
    /// `PlatformWindow::request_redraw` — the same wake a signal change
    /// triggers — so the drop lands even if no further input ever arrives.
    ///
    /// The drop resolves at the last position the window saw — refreshed by
    /// the runner's platform query before the file events were dispatched,
    /// see [`Self::note_pointer_position`]; if the pointer was never
    /// observed entering, the drop is discarded with an error — never
    /// silently.
    pub fn finish_os_file_drop(&mut self, env: &Environment) -> bool {
        let Some(state) = self.hit_test.os_file_drag.as_mut() else {
            return false;
        };
        match state.phase {
            OsFileDragPhase::Hovering => return false,
            OsFileDragPhase::Dropping {
                collected_this_drain: true,
            } => {
                // This drain just collected files — a drop may still be
                // landing; the runner schedules the follow-up drain that
                // delivers it.
                state.phase = OsFileDragPhase::Dropping {
                    collected_this_drain: false,
                };
                return false;
            }
            OsFileDragPhase::Dropping {
                collected_this_drain: false,
            } => {}
        }
        let state = self
            .hit_test
            .os_file_drag
            .take()
            .expect("os_file_drag presence checked above");
        let payload = Self::os_file_payload(&state.paths);
        match self.hit_test.active_drag.as_mut() {
            Some(active_drag) => active_drag.payload = payload,
            None => {
                self.hit_test.active_drag = Some(ActiveDrag {
                    payload,
                    hovered_target: None,
                });
            }
        }
        let Some(point) = self.hit_test.pointer_position else {
            tracing::error!(
                target: "waterui::hydrolysis::input",
                paths = ?state.paths,
                "OS file drop discarded: no pointer position has ever been reported"
            );
            return self.cancel_active_drag(env);
        };
        self.finish_active_drag(point, env)
    }

    /// An OS file drag left the window or ended without a drop (winit
    /// `HoveredFileCancelled`).
    pub fn handle_file_hover_cancelled(&mut self, env: &Environment) -> bool {
        self.registries();
        self.hit_test.os_file_drag = None;
        self.cancel_active_drag(env)
    }

    pub(crate) fn sync_active_pointer_drag_target_after_layout(
        &mut self,
        pointer: Option<kurbo::Point>,
    ) {
        let Some(active) = self.hit_test.active_pointer_drag_target.as_ref() else {
            return;
        };
        let alive = self
            .hit_test
            .pointer_targets
            .iter()
            .any(|target| Rc::ptr_eq(&target.action, active));
        if alive {
            return;
        }
        if let Some((depth, order)) = self.hit_test.active_pointer_drag_signature
            && let Some(target) = self.hit_test.pointer_targets.iter().find(|target| {
                target.captures_drag && target.depth == depth && target.order == order
            })
        {
            self.hit_test.active_pointer_drag_target = Some(Rc::clone(&target.action));
            self.hit_test.active_pointer_drag_owner = target.owner.clone();
            return;
        }
        let Some(point) = pointer else {
            self.hit_test.active_pointer_drag_target = None;
            self.hit_test.active_pointer_drag_signature = None;
            self.hit_test.active_pointer_drag_owner = Weak::new();
            self.clear_scrollbar_drag();
            return;
        };
        let mut indices: Vec<usize> = self
            .hit_test
            .pointer_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.captures_drag && target.bounds.contains(point))
            .map(|(index, _)| index)
            .collect();
        indices.sort_unstable_by(|left, right| {
            let left_target = &self.hit_test.pointer_targets[*left];
            let right_target = &self.hit_test.pointer_targets[*right];
            Self::target_hit_priority(right_target.depth, right_target.order, *right).cmp(
                &Self::target_hit_priority(left_target.depth, left_target.order, *left),
            )
        });
        if let Some(index) = indices.first().copied() {
            let target = &self.hit_test.pointer_targets[index];
            self.hit_test.active_pointer_drag_target = Some(Rc::clone(&target.action));
            self.hit_test.active_pointer_drag_signature = Some((target.depth, target.order));
            self.hit_test.active_pointer_drag_owner = target.owner.clone();
        } else {
            self.hit_test.active_pointer_drag_target = None;
            self.hit_test.active_pointer_drag_signature = None;
            self.hit_test.active_pointer_drag_owner = Weak::new();
            self.clear_scrollbar_drag();
        }
    }
}

/// The gesture engine's button vocabulary is a subset of the platform's:
/// `Other(u16)` has no counterpart and stays unrouted
/// (water-rs/waterui#1290).
const fn gesture_button(button: PointerButton) -> Option<WuiPointerButton> {
    Some(match button {
        PointerButton::Primary => WuiPointerButton::Primary,
        PointerButton::Secondary => WuiPointerButton::Secondary,
        PointerButton::Middle => WuiPointerButton::Middle,
        PointerButton::Back => WuiPointerButton::Back,
        PointerButton::Forward => WuiPointerButton::Forward,
        PointerButton::Other(_) => return None,
    })
}

impl HydrolysisRenderer {
    /// Dispatches a mouse-origin press at (`x`, `y`) and reports whether the
    /// hit consumed it.
    ///
    /// # Panics
    /// Panics on an internal ordering or arithmetic inconsistency.
    pub fn handle_pointer_down(
        &mut self,
        x: f32,
        y: f32,
        button: PointerButton,
        env: &Environment,
    ) -> bool {
        self.registries();
        self.handle_pointer_down_with_source(0, PointerKind::Mouse, x, y, button, env)
    }

    /// Dispatches a press from an arbitrary pointer source (`pointer_id`,
    /// `pointer_kind`) and reports whether the hit consumed it.
    ///
    /// # Panics
    /// Panics on an internal ordering or arithmetic inconsistency.
    #[expect(
        clippy::too_many_lines,
        reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
    )]
    pub fn handle_pointer_down_with_source(
        &mut self,
        pointer_id: u64,
        pointer_kind: PointerKind,
        x: f32,
        y: f32,
        button: PointerButton,
        env: &Environment,
    ) -> bool {
        self.registries();
        if self
            .hit_test
            .active_pointer
            .is_some_and(|active| active != (pointer_id, pointer_kind))
        {
            return false;
        }
        self.hit_test.active_pointer = Some((pointer_id, pointer_kind));
        self.hit_test.active_pointer_button = Some(button);
        let point = kurbo::Point::new(f64::from(x), f64::from(y));
        self.hit_test.pointer_position = Some(point);
        self.hit_test.pointer_press_origin = Some(point);
        let at = self.frame_instant();
        if pointer_kind == PointerKind::Touch {
            self.arm_touch_scroll(point, button, at);
        }
        let mut refresh_requested = false;
        let mut visual_changed = false;
        self.hit_test.active_pointer_drag_target = None;
        self.hit_test.active_pointer_drag_owner = Weak::new();
        self.hit_test.active_pointer_drag_signature = None;
        self.clear_scrollbar_drag();
        self.hit_test.active_pointer_target = None;
        self.hit_test.pending_pointer_press = None;
        self.hit_test.pending_context_menu_hold = None;
        self.hit_test.active_press_bounds = None;
        self.hit_test.active_press_origin = None;
        self.hit_test.pointer_press_origin = None;
        refresh_requested |= self.cancel_active_drag(env);
        let press_clear = self.hit_test.interaction.clear_all_presses(at);
        visual_changed |= press_clear.visual_changed;
        refresh_requested |= press_clear.chrome_changed;
        self.hit_test.interaction.mark_press_clear(&press_clear);
        self.text_editing.active_text_selection_drag = None;
        let overlay_hit = matches!(
            self.text_editing.active_text_context_menu,
            Some(ActiveTextContextMenu::Overlay { .. })
        );
        if overlay_hit {
            let changed = self.handle_text_context_menu_overlay_pointer_down(point);
            if changed || self.text_editing.active_text_context_menu.is_none() {
                return visual_changed || changed;
            }
        }
        // A press inside the open context-menu presentation — its drawn menu
        // or its accessory — belongs to the presentation's own targets: it
        // neither dismisses the menu nor opens a new one, because accessory
        // actions do not close the menu by themselves
        // (water-rs/waterui#1245).
        let in_context_menu_presentation = self.context_menu_presentation_contains(point);
        if button != PointerButton::Secondary {
            self.dismiss_active_text_context_menu();
        }
        if !in_context_menu_presentation {
            self.dismiss_active_popup_menu();
        }
        // Anchored overlays with `OutsideInteraction` dismissal write `false`
        // when the press lands outside them; the press still reaches its
        // target below, exactly like the menu dismissals above.
        self.dismiss_anchored_overlays_outside(point);
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            button = ?button,
            pointer_targets = self.hit_test.pointer_targets.len(),
            text_inputs = self.text_editing.text_input_targets.len(),
            gesture_targets = self.gesture_engine.target_count(),
            "pointer down begin"
        );

        let mut pointer_indices: Vec<usize> = self
            .hit_test
            .pointer_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.bounds.contains(point))
            .map(|(index, _)| index)
            .collect();
        pointer_indices.sort_unstable_by(|left, right| {
            let left_target = &self.hit_test.pointer_targets[*left];
            let right_target = &self.hit_test.pointer_targets[*right];
            SemanticCore::target_hit_priority(right_target.depth, right_target.order, *right).cmp(
                &SemanticCore::target_hit_priority(left_target.depth, left_target.order, *left),
            )
        });
        let focused = self.topmost_text_input_index_at_point(point);
        let top_pointer_priority = pointer_indices.first().map(|index| {
            let target = &self.hit_test.pointer_targets[*index];
            SemanticCore::target_hit_priority(target.depth, target.order, *index)
        });
        let focused_priority = focused.map(|index| {
            let target = &self.text_editing.text_input_targets[index];
            SemanticCore::target_hit_priority(target.depth, target.order, index)
        });
        let focus_wins = matches!(
            (focused_priority, top_pointer_priority),
            (Some(focus_priority), Some(pointer_priority)) if focus_priority > pointer_priority
        ) || matches!((focused_priority, top_pointer_priority), (Some(_), None));

        // A secondary press a context menu will claim — one enclosing an
        // embedded surface, the focused text input's own menu, or the
        // topmost `.context_menu` region at the point — opens that menu
        // instead of arming recognizers (water-rs/waterui#1290).
        let context_menu_claims_secondary = button == PointerButton::Secondary
            && !in_context_menu_presentation
            && if let Some((_, surface, _)) =
                self.embedded_target_wins_at(point, top_pointer_priority, focused_priority)
            {
                let surface_bounds = surface.to_window_rect(kurbo::Rect::from_origin_size(
                    kurbo::Point::ORIGIN,
                    surface.local_bounds.size(),
                ));
                self.topmost_context_menu_target_enclosing(point, surface_bounds)
                    .is_some_and(|target| {
                        !popup_menu_nodes(
                            &target.items.snapshot(),
                            &target.env,
                            self.window_closable,
                        )
                        .is_empty()
                    })
            } else {
                (focus_wins && focused.is_some())
                    || self
                        .topmost_context_menu_target_at_point(point)
                        .is_some_and(|target| {
                            !popup_menu_nodes(
                                &target.items.snapshot(),
                                &target.env,
                                self.window_closable,
                            )
                            .is_empty()
                        })
            };
        let gesture_changed = gesture_button(button).is_some_and(|mapped| {
            !context_menu_claims_secondary
                && self.with_unoccluded_gesture_targets(point, |engine| {
                    engine.handle_pointer_down(point, at, mapped, env)
                })
        });
        refresh_requested |= gesture_changed;
        // A touch or pen primary press on a `.context_menu` region is a
        // pending press-and-hold, armed exactly where a secondary press
        // would resolve a menu: an embedded surface yields only to a menu
        // enclosing its window rect, anything else to the topmost region
        // containing the point. A held primary *mouse* button never arms —
        // the platforms bind the gesture to touch and pen only.
        if button == PointerButton::Primary
            && matches!(pointer_kind, PointerKind::Touch | PointerKind::Pen)
            && !in_context_menu_presentation
        {
            let menu_present = if let Some((_, surface, _)) =
                self.embedded_target_wins_at(point, top_pointer_priority, focused_priority)
            {
                let surface_bounds = surface.to_window_rect(kurbo::Rect::from_origin_size(
                    kurbo::Point::ORIGIN,
                    surface.local_bounds.size(),
                ));
                self.topmost_context_menu_target_enclosing(point, surface_bounds)
                    .is_some()
            } else {
                self.topmost_context_menu_target_at_point(point).is_some()
            };
            if menu_present {
                self.hit_test.pending_context_menu_hold = Some(PendingContextMenuHold {
                    point,
                    started_at: at,
                });
            }
        }
        if !in_context_menu_presentation
            && let Some((_index, target, local_position)) =
                self.embedded_target_wins_at(point, top_pointer_priority, focused_priority)
        {
            // A press on a surface focuses it through the keyboard-focus
            // machinery — the same path Tab traversal and `.focused(binding)`
            // take — not a parallel one: keyboard focus lands on the
            // surface's own interaction identity, which drives the
            // `Focus(true)`/`Focus(false)` sink transition.
            self.set_keyboard_focus(Some(target.interaction_key.clone()), false);
            self.set_focused_text_input(None);
            if button == PointerButton::Primary {
                self.text_editing.note_activation();
            }
            target.sink.pointer_move(local_position);
            // A secondary press still focuses the surface, but a context menu
            // enclosing it claims the button: the menu's actions act on the
            // focused surface, so the button itself is not delivered. A surface
            // with no enclosing menu, or whose menu has no items, receives the
            // secondary button as before.
            if button == PointerButton::Secondary {
                let surface_bounds = target.to_window_rect(kurbo::Rect::from_origin_size(
                    kurbo::Point::ORIGIN,
                    target.local_bounds.size(),
                ));
                if let Some(menu_target) =
                    self.topmost_context_menu_target_enclosing(point, surface_bounds)
                {
                    let mut items = popup_menu_nodes(
                        &menu_target.items.snapshot(),
                        &menu_target.env,
                        self.window_closable,
                    );
                    if !items.is_empty() {
                        // The debug inspect entry extends a menu, it does not
                        // create one — an empty `.context_menu` must behave
                        // the same in every build (water-rs/hydrolysis#188).
                        self.append_inspect_element_item(&mut items, point);
                        // The menu opens in the declaring view's environment
                        // layered over this dispatch's, so `.state(&value)`
                        // overlays reach the item actions.
                        let menu_env = menu_target.env.layered_on(env);
                        let metrics = self.theme().text_context_menu_metrics();
                        self.show_context_menu(
                            items,
                            Some(&menu_target),
                            LayoutPoint::new(
                                crate::num_cast::f64_as_f32(point.x),
                                crate::num_cast::f64_as_f32(point.y),
                            ),
                            metrics,
                            &menu_env,
                            false,
                        );
                        return true;
                    }
                }
            }
            target.sink.pointer_button(true, button, local_position);
            self.hit_test.active_embedded_target = Some(target);
            return true;
        }
        if self.hit_test.focused_embedded_sink.is_some() {
            // The press landed off every surface: drop the surface's focus
            // through the shared path — its sink gets `Focus(false)` and the
            // surface's keyboard-focus slot clears with it.
            self.set_keyboard_focus(None, false);
        }
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
            if let Some(index) = focused {
                let key = self.text_editing.text_input_targets[index]
                    .interaction_key
                    .clone();
                self.set_keyboard_focus(Some(key), false);
            } else {
                self.set_keyboard_focus(None, false);
            }
            let mut changed = self.set_focused_text_input(focused);
            if let Some(index) = focused {
                match button {
                    PointerButton::Primary => {
                        self.text_editing.note_activation();
                        let click_count = self.next_text_selection_click_count(index, point, at);
                        if let Some((anchor, focus, gesture_changed)) =
                            self.apply_text_selection_click_gesture(index, point, click_count)
                        {
                            changed |= gesture_changed;
                            self.text_editing.active_text_selection_drag =
                                self.text_editing.key_at(index).cloned().map(|target| {
                                    ActiveTextSelectionDrag {
                                        target,
                                        click_count,
                                        anchor,
                                        focus,
                                    }
                                });
                        }
                    }
                    PointerButton::Secondary => {
                        let keep_selection = {
                            let target = &self.text_editing.text_input_targets[index];
                            let selection_index =
                                SemanticCore::text_selection_index_from_point(target, point);
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
            return refresh_requested || visual_changed || changed;
        }

        if button != PointerButton::Primary {
            if button == PointerButton::Secondary && !in_context_menu_presentation {
                let menu_target = self.topmost_context_menu_target_at_point(point);
                let mut items = menu_target
                    .as_ref()
                    .map(|target| {
                        popup_menu_nodes(
                            &target.items.snapshot(),
                            &target.env,
                            self.window_closable,
                        )
                    })
                    .unwrap_or_default();
                if !items.is_empty() {
                    // The debug inspect entry extends a menu, it does not
                    // create one — an empty `.context_menu` must behave the
                    // same in every build (water-rs/hydrolysis#188).
                    self.append_inspect_element_item(&mut items, point);
                    if self.set_focused_text_input(focused) {
                        refresh_requested = true;
                    }
                    // Same inheritance as the embedded-surface arm: the
                    // declaring view's environment layers over this
                    // dispatch's.
                    let menu_env = menu_target
                        .as_ref()
                        .map_or_else(|| env.clone(), |target| target.env.layered_on(env));
                    let metrics = self.theme().text_context_menu_metrics();
                    let changed = self.show_context_menu(
                        items,
                        menu_target.as_ref(),
                        LayoutPoint::new(
                            crate::num_cast::f64_as_f32(point.x),
                            crate::num_cast::f64_as_f32(point.y),
                        ),
                        metrics,
                        &menu_env,
                        false,
                    );
                    return refresh_requested || visual_changed || changed;
                }
            }
            if matches!(button, PointerButton::Other(_)) {
                if self.set_focused_text_input(focused) {
                    refresh_requested = true;
                }
                return refresh_requested || visual_changed;
            }
            // `Other` stays unrouted; a secondary press no menu claimed and
            // Middle/Back/Forward fall through to the pointer targets below
            // with the gesture engine already armed on the button.
        }

        for index in pointer_indices {
            let target = self.hit_test.pointer_targets[index].clone();
            if button != PointerButton::Primary && target.press_slot.is_some() {
                // Press slots are primary-only: a `Button` commits on a
                // primary release, never on a middle or secondary one
                // (water-rs/waterui#1290).
                continue;
            }
            if self.hit_test.gesture_regions.iter().any(|region| {
                region.order > target.order
                    && region.bounds.contains(point)
                    && !Self::gesture_region_encloses(region, &target)
                    && !Self::gesture_claims_press(region, &target)
            }) {
                // A gesture region painted above this target in a subtree it
                // does not belong to owns the press: hit-testing stops at the
                // topmost interactive target at the point, whichever engine
                // carries it. Regions enclosing the target (a wrapping
                // `.on_tap`) or nested inside it (claimed via
                // `gesture_claims_press`) are left to their own rules.
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    x,
                    y,
                    pointer_index = index,
                    bounds = ?target.bounds,
                    order = target.order,
                    "pointer target occluded by overlying gesture region"
                );
                continue;
            }
            if target.press_slot.is_some()
                && self
                    .hit_test
                    .gesture_regions
                    .iter()
                    .filter(|region| region.order > target.order && region.bounds.contains(point))
                    .max_by_key(|region| region.order)
                    .is_some_and(|region| Self::gesture_claims_press(region, &target))
            {
                // A gesture region nested inside this press's bounds claims
                // the sequence: the innermost interactive target under the
                // pointer wins. The recognizers were already activated above;
                // skip the commit so the press does not run, but still grant
                // the focus the press point earns — the claim takes the
                // activation, not the row the press sits in
                // (water-rs/hydrolysis#220).
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    x,
                    y,
                    pointer_index = index,
                    bounds = ?target.bounds,
                    order = target.order,
                    "nested gesture target claims press"
                );
                self.set_keyboard_focus_for_press(
                    target.press_slot.as_ref().map(|slot| slot.key.clone()),
                    point,
                );
                return refresh_requested || visual_changed;
            }
            let keyboard_key = target.press_slot.as_ref().map(|slot| slot.key.clone());
            self.set_keyboard_focus_for_press(keyboard_key, point);
            refresh_requested |= self.set_focused_text_input(None);
            if let Some(slot) = target.press_slot.as_ref() {
                let chrome_state_dependent = target
                    .interaction
                    .as_ref()
                    .is_some_and(|handles| handles.chrome_state_dependent());
                let touch_delay = target
                    .interaction
                    .as_ref()
                    .map_or(Duration::ZERO, |handles| handles.touch_delay());
                if pointer_kind == PointerKind::Mouse || touch_delay.is_zero() {
                    self.hit_test.interaction.begin_press(slot, point, at);
                    visual_changed = true;
                } else {
                    self.hit_test.pending_pointer_press = Some(PendingPointerPress {
                        slot: slot.clone(),
                        origin: point,
                        starts_at: at
                            .checked_add(touch_delay)
                            .expect("hydrolysis pointer press start time overflow"),
                        chrome_state_dependent,
                    });
                }
                self.hit_test.active_press_bounds = Some(target.bounds);
                self.hit_test.active_press_origin = Some(point);
                if chrome_state_dependent && visual_changed {
                    SemanticCore::mark_owner(&target.owner, Dirty::LAYOUT);
                    refresh_requested = true;
                } else if visual_changed {
                    SemanticCore::mark_owner(&target.owner, Dirty::PAINT);
                }
            }
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
            if target.captures_drag {
                let changed = (target.action.borrow_mut())(self, point, env);
                if changed {
                    SemanticCore::mark_owner(&target.owner, Dirty::LAYOUT);
                    refresh_requested = true;
                }
                self.hit_test.active_pointer_drag_target = Some(Rc::clone(&target.action));
                self.hit_test.active_pointer_drag_signature = Some((target.depth, target.order));
                self.hit_test.active_pointer_drag_owner = target.owner;
                return refresh_requested || visual_changed || changed;
            }
            if target.press_slot.is_some() {
                // Material controls commit on release, not on pointer-down. Keep
                // the topmost opaque target captured so a release inside its
                // bounds activates it exactly once; a release outside cancels.
                self.hit_test.active_pointer_target = Some(target);
                return refresh_requested || visual_changed;
            }
            let changed = (target.action.borrow_mut())(self, point, env);
            if changed {
                SemanticCore::mark_owner(&target.owner, Dirty::LAYOUT);
                refresh_requested = true;
            }
            // Slot-less utility targets stay transparent when unhandled,
            // allowing an overlapping target beneath them to receive the event.
            if !changed {
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
            return refresh_requested || visual_changed || changed;
        }
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            focused_candidate = ?focused,
            "pointer down text-input fallback"
        );
        if self.set_focused_text_input(focused) {
            refresh_requested = true;
        }
        self.set_keyboard_focus(None, false);
        refresh_requested || visual_changed
    }

    /// `region` claims `press` when the press's owning view is a strict
    /// ancestor of the region's owner — a gesture or tap registered by a
    /// descendant of the view the press belongs to takes the sequence
    /// (water-rs/hydrolysis#175). A handler attached to the press's own view
    /// lands on the region's own owner — the same node, not a descendant — so
    /// the press commits alongside it, as it does today.
    fn gesture_claims_press(region: &GestureRegion, press: &PointerTarget) -> bool {
        press.owners.last().is_some_and(|owner| {
            region.owners.last() != Some(owner) && region.owners.contains(owner)
        })
    }

    /// `region` encloses `press` when the view the gesture hangs on is an
    /// ancestor (or the very node) of the view the press belongs to: the
    /// region's owner — the last link of its chain — sits in the press's
    /// chain. A wrapping `.on_tap` around a control is such a region, and
    /// never shadows the control.
    fn gesture_region_encloses(region: &GestureRegion, press: &PointerTarget) -> bool {
        region
            .owners
            .last()
            .is_some_and(|owner| press.owners.contains(owner))
    }

    /// Handles a pointer release at `(x, y)`; returns whether anything consumed it.
    pub fn handle_pointer_up(
        &mut self,
        x: f32,
        y: f32,
        button: PointerButton,
        env: &Environment,
    ) -> bool {
        self.registries();
        self.handle_pointer_up_with_source(0, PointerKind::Mouse, x, y, button, env)
    }

    /// Handles a pointer release with the pointer's id, kind and button.
    pub fn handle_pointer_up_with_source(
        &mut self,
        pointer_id: u64,
        pointer_kind: PointerKind,
        x: f32,
        y: f32,
        button: PointerButton,
        env: &Environment,
    ) -> bool {
        self.registries();
        if self.hit_test.active_pointer != Some((pointer_id, pointer_kind)) {
            return false;
        }
        let point = kurbo::Point::new(f64::from(x), f64::from(y));
        if let Some(target) = self.hit_test.active_embedded_target.take() {
            let position = target.local_position_unclamped(point);
            target.sink.pointer_move(position);
            target.sink.pointer_button(false, button, position);
            self.hit_test.active_pointer = None;
            self.hit_test.active_pointer_button = None;
            self.hit_test.touch_scroll = TouchScrollGesture::Idle;
            return true;
        }
        let at = self.frame_instant();
        // Order matters: the move can arm `pending_pointer_press` for this same
        // release, so it runs before the pending press is drained.
        #[allow(clippy::useless_let_if_seq)]
        let mut changed = self.handle_pointer_move_inner(x, y, env, pointer_kind);
        if pointer_kind == PointerKind::Touch {
            changed |= self.finish_touch_scroll(at);
        }
        if let Some(pending) = self.hit_test.pending_pointer_press.take() {
            self.hit_test
                .interaction
                .begin_press(&pending.slot, pending.origin, at);
            let bits = if pending.chrome_state_dependent {
                Dirty::LAYOUT
            } else {
                Dirty::PAINT
            };
            self.mark_key_owner(&pending.slot.key, bits);
            changed = true;
        }
        if let Some(target) = self.hit_test.active_pointer_target.take()
            && target.bounds.contains(point)
        {
            let action_changed = (target.action.borrow_mut())(self, point, env);
            if action_changed {
                SemanticCore::mark_owner(&target.owner, Dirty::LAYOUT);
            }
            changed |= action_changed;
        }
        let drop_changed = self.finish_active_drag(point, env);
        changed |= drop_changed;
        changed |=
            self.finish_interactive_navigation_pop(NavigationInteractivePopOutcome::ByProgress);
        self.text_editing.active_text_selection_drag = None;
        self.hit_test.active_pointer_drag_target = None;
        self.hit_test.active_pointer_drag_owner = Weak::new();
        self.hit_test.active_pointer_drag_signature = None;
        self.clear_scrollbar_drag();
        self.hit_test.active_pointer_target = None;
        self.hit_test.pending_context_menu_hold = None;
        self.hit_test.active_press_bounds = None;
        self.hit_test.active_press_origin = None;
        self.hit_test.pointer_press_origin = None;
        self.hit_test.active_pointer = None;
        self.hit_test.active_pointer_button = None;
        let press_clear = self.hit_test.interaction.clear_all_presses(at);
        self.hit_test.interaction.mark_press_clear(&press_clear);
        changed |= press_clear.visual_changed || press_clear.chrome_changed;
        let gesture_changed = gesture_button(button).is_some_and(|mapped| {
            self.gesture_engine
                .handle_pointer_up(point, at, mapped, env)
        });
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

    /// Handles a pointer move to `(x, y)`; returns whether anything consumed it.
    pub fn handle_pointer_move(&mut self, x: f32, y: f32, env: &Environment) -> bool {
        self.registries();
        self.handle_pointer_move_with_source(0, PointerKind::Mouse, x, y, env)
    }

    /// Handles a pointer move with the pointer's id and kind.
    pub fn handle_pointer_move_with_source(
        &mut self,
        pointer_id: u64,
        pointer_kind: PointerKind,
        x: f32,
        y: f32,
        env: &Environment,
    ) -> bool {
        self.registries();
        if self
            .hit_test
            .active_pointer
            .is_some_and(|active| active != (pointer_id, pointer_kind))
        {
            return false;
        }
        if pointer_kind != PointerKind::Mouse
            && self.hit_test.pending_pointer_press.take().is_some()
        {
            self.hit_test.active_pointer_target = None;
            self.hit_test.active_press_bounds = None;
            self.hit_test.active_press_origin = None;
        }
        self.handle_pointer_move_inner(x, y, env, pointer_kind)
    }

    fn handle_pointer_move_inner(
        &mut self,
        x: f32,
        y: f32,
        env: &Environment,
        pointer_kind: PointerKind,
    ) -> bool {
        let point = kurbo::Point::new(f64::from(x), f64::from(y));
        self.hit_test.pointer_position = Some(point);
        let at = self.frame_instant();
        // A claimed touch-drag scroll owns the sequence: the move drives
        // its one-to-one tracking and nothing else sees it.
        if pointer_kind == PointerKind::Touch && self.handle_touch_scroll_move(point, at, env) {
            return true;
        }
        // The hold dies when the press leaves the recognizer's slop — the
        // same move check `LongPressDetector` applies.
        if let Some(hold) = self.hit_test.pending_context_menu_hold
            && (point.x - hold.point.x).hypot(point.y - hold.point.y) > LONG_PRESS_SLOP
        {
            self.hit_test.pending_context_menu_hold = None;
        }
        // Same first look the press path gives: an active recognizer
        // receives the move before any embedded surface can claim it, so a
        // gesture in flight is not starved by whatever sits under the pointer.
        let move_button = self
            .hit_test
            .active_pointer_button
            .and_then(gesture_button)
            .unwrap_or(WuiPointerButton::Primary);
        let gesture_changed = self
            .gesture_engine
            .handle_pointer_move(point, at, move_button, env);
        if self.handle_embedded_pointer_move(point) {
            return true;
        }
        let mut refresh_requested = gesture_changed;
        let mut drag_changed = false;
        if let Some(index) = self.text_editing.selection_drag_index() {
            let text_drag_changed = self.update_text_selection_drag(index, point);
            drag_changed |= text_drag_changed;
            refresh_requested |= text_drag_changed;
        }
        if let Some(action) = self.hit_test.active_pointer_drag_target.clone() {
            let pointer_drag_changed = (action.borrow_mut())(self, point, env);
            if pointer_drag_changed {
                let owner = self.hit_test.active_pointer_drag_owner.clone();
                SemanticCore::mark_owner(&owner, Dirty::LAYOUT);
            }
            drag_changed |= pointer_drag_changed;
            refresh_requested |= pointer_drag_changed;
        }
        // An OS file drag has no pointer target driving it — its hover sync
        // happens here. For an in-app drag this repeats the sync the
        // drag-target action just ran, a no-op while the target is unchanged.
        let drag_hover_changed = self.sync_active_drag_hover(point, env);
        drag_changed |= drag_hover_changed;
        refresh_requested |= drag_hover_changed;
        let hover = if pointer_kind == PointerKind::Mouse {
            self.hit_test.sync_hover_targets(point, env, true, at)
        } else {
            HoverSync::default()
        };
        refresh_requested |= hover.handler_changed;
        self.hit_test.interaction.mark_hover_sync(&hover);
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            changed = refresh_requested,
            drag_changed,
            gesture_changed,
            hover_visual = hover.visual_changed,
            dragging = self.hit_test.active_pointer_drag_target.is_some(),
            gesture_active = self.gesture_engine.has_active_recognizer(),
            "pointer move handled"
        );
        refresh_requested || hover.visual_changed
    }

    /// Re-runs hover for the pointer's current position; returns whether hover state changed.
    pub fn sync_pointer_hover_state(&mut self, x: f32, y: f32, env: &Environment) -> bool {
        let point = kurbo::Point::new(f64::from(x), f64::from(y));
        let at = self.frame_instant();
        let hover = self.hit_test.sync_hover_targets(point, env, false, at);
        let changed = hover.handler_changed;
        self.hit_test.interaction.mark_hover_sync(&hover);
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            changed,
            dragging = self.hit_test.active_pointer_drag_target.is_some(),
            gesture_active = self.gesture_engine.has_active_recognizer(),
            "pointer hover sync handled"
        );
        changed
    }
}

/// One stop in keyboard-focus traversal — the semantic node plus the
/// interaction identities it resolves to. `key` is `None` for a focusable
/// node that never registered a pointer target, which only happens when the
/// accessibility feature is off or the widget is pointer-inert.
struct KeyboardFocusCandidate {
    key: Option<InteractionKey>,
    #[cfg(feature = "accessibility")]
    node: AccessibilityNodeId,
    text_input: Option<usize>,
    /// The candidate's index in `embedded_input_targets`, when it names an
    /// input surface — the keyboard-focus slot such a surface occupies.
    embedded: Option<usize>,
    /// Emission order — the candidate's slot in the sequence traversal
    /// walks. The accessibility flavour counts the emitted node tree; the
    /// pointer/text-input flavour sorts by it.
    order: usize,
}

impl SemanticCore {
    /// The accessibility node `key` emitted this frame, when the widget
    /// stamped a focus link or owns a text-input target.
    ///
    /// `interaction_nodes` is cleared at `reset_scene`, so the map only ever
    /// holds this flush's links — a key whose widget emitted no node resolves
    /// to `None` here rather than to the id a previous flush registered.
    #[cfg(feature = "accessibility")]
    pub(crate) fn focus_node_for_key(&self, key: &InteractionKey) -> Option<AccessibilityNodeId> {
        self.accessibility
            .interaction_nodes
            .get(key)
            .copied()
            .or_else(|| {
                self.text_editing
                    .text_input_targets
                    .iter()
                    .find(|target| &target.interaction_key == key)
                    .and_then(|target| target.accessibility_node_id)
            })
            .or_else(|| {
                self.hit_test
                    .embedded_input_targets
                    .iter()
                    .find(|target| &target.interaction_key == key)
                    .and_then(|target| target.accessibility_node_id)
            })
    }

    /// The interaction identity behind `node` — the press slot, text-input
    /// target or embedded surface the widget linked its emitted node to.
    /// Linked keys that carry no live machinery this frame do not qualify:
    /// a list row's base identity exists so the subtree can resolve its
    /// node, while only the row's selection press can hold pointer-focus
    /// (water-rs/hydrolysis#220). A node linked only by such keys resolves
    /// to `None` — it still takes semantic focus, it simply has no
    /// interaction key for the pointer machinery to track.
    #[cfg(feature = "accessibility")]
    fn focus_key_for_node(&self, node: AccessibilityNodeId) -> Option<InteractionKey> {
        self.accessibility
            .interaction_nodes
            .iter()
            .filter(|(_, linked)| **linked == node)
            .map(|(key, _)| key.clone())
            .find(|key| {
                self.interaction_key_is_live(key)
                    // The semantic walk emits no pointer machinery to back a
                    // key — a key linked to a live node is live there, the
                    // same allowance `frame.rs` makes for focus liveness.
                    || (self.semantic_walk && self.emitted_node_is_live(node))
            })
            .or_else(|| {
                self.text_editing
                    .text_input_targets
                    .iter()
                    .find(|target| target.accessibility_node_id == Some(node))
                    .map(|target| target.interaction_key.clone())
            })
            .or_else(|| {
                self.hit_test
                    .embedded_input_targets
                    .iter()
                    .find(|target| target.accessibility_node_id == Some(node))
                    .map(|target| target.interaction_key.clone())
            })
    }

    /// Whether the modal shield is up this frame: an active
    /// `ModalInteraction` scope was emitted, or modal-flagged machinery
    /// exists. While it is up only modal targets answer input.
    pub(crate) fn modal_shield_active(&self) -> bool {
        self.hit_test.modal_interaction.is_some()
            || self
                .hit_test
                .pointer_targets
                .iter()
                .any(|target| target.modal)
            || self
                .text_editing
                .text_input_targets
                .iter()
                .any(|target| target.modal)
    }

    /// Whether `key` is backed by input machinery emitted this frame — a
    /// press slot, a text-input target or an embedded surface — under the
    /// modal shield's rule: while a modal scope is up only modal targets
    /// count. The single definition both `focus_key_for_node` and the
    /// post-flush liveness pass share (water-rs/hydrolysis#220).
    pub(crate) fn interaction_key_is_live(&self, key: &InteractionKey) -> bool {
        let modal_active = self.modal_shield_active();
        self.hit_test.pointer_targets.iter().any(|target| {
            (!modal_active || target.modal)
                && target
                    .press_slot
                    .as_ref()
                    .is_some_and(|slot| &slot.key == key)
        }) || self
            .text_editing
            .text_input_targets
            .iter()
            .any(|target| (!modal_active || target.modal) && &target.interaction_key == key)
            || self
                .hit_test
                .embedded_input_targets
                .iter()
                .any(|target| &target.interaction_key == key)
    }

    /// Moves keyboard focus to `node` — the semantic-tree identity every
    /// focus path converges on. The pointer machinery's `InteractionKey` is
    /// resolved from it where the widget linked one, so a focusable node
    /// needs no pointer target to take keyboard focus.
    ///
    /// The caret follows the semantic focus only onto a text input:
    /// landing on a text node focuses it for editing, while landing on a
    /// non-text node (or none) leaves the caret where it is — UI focus is
    /// independent of the tree's focus. Moves that carry keyboard focus —
    /// traversal and pointer presses — end editing on a non-text target
    /// through their own `set_focused_text_input` call (#95).
    #[cfg(feature = "accessibility")]
    pub(crate) fn set_keyboard_focus_node(
        &mut self,
        node: Option<AccessibilityNodeId>,
        visible: bool,
    ) -> bool {
        let key = node.and_then(|node| self.focus_key_for_node(node));
        let text_input = node.and_then(|node| {
            self.text_editing
                .text_input_targets
                .iter()
                .position(|target| target.accessibility_node_id == Some(node))
        });
        let embedded = node.and_then(|node| self.embedded_index_for_node(node));
        let mut changed = self.set_keyboard_focus_impl(key, node, visible);
        changed |= self.hit_test.set_embedded_focus_index(embedded);
        if let Some(index) = text_input {
            changed |= self.set_focused_text_input(Some(index));
        }
        changed
    }

    /// The currently focused semantic node — `None` while window-level focus
    /// rests on the tree root.
    #[cfg(feature = "accessibility")]
    pub(crate) fn keyboard_focus_node(&self) -> Option<AccessibilityNodeId> {
        (self.accessibility.focus != ACCESSIBILITY_ROOT_NODE_ID).then_some(self.accessibility.focus)
    }

    /// Enter/Space on a focused control dispatches `Click` on key-down — the
    /// semantic runtime's keyboard contract. Rendered runtimes keep the
    /// [`KeyboardActivation::PressRelease`] default.
    #[cfg(feature = "accessibility")]
    pub(crate) const fn use_semantic_keyboard_activation(&mut self) {
        self.hit_test.keyboard_activation = KeyboardActivation::Semantic;
    }

    /// This core is the semantic walk: it emits no pointer machinery, so
    /// the focus-liveness fallback may resolve a key through the semantic
    /// focus link. Rendered runtimes never call it — there a key backed by
    /// no target is dead even when a frame happens to emit no pointer
    /// targets at all.
    #[cfg(feature = "accessibility")]
    pub(crate) const fn use_semantic_walk(&mut self) {
        self.semantic_walk = true;
    }

    /// Keyboard focus where a pointer press landed: the node the pressed
    /// target's key is linked to, or — when the key carries no node, as a
    /// bare `on_tap` press slot does — the innermost node under the press
    /// point that advertises `Focus`. A tap inside a `List` row then still
    /// focuses the row's `ListItem` node, so the arrow navigation
    /// `list_row_context` drives has a row to start from
    /// (water-rs/hydrolysis#220).
    // the parameter is a small Copy value taken by value for a uniform call-site signature
    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn set_keyboard_focus_for_press(
        &mut self,
        key: Option<InteractionKey>,
        point: kurbo::Point,
    ) -> bool {
        #[cfg(feature = "accessibility")]
        {
            let node = key
                .as_ref()
                .and_then(|key| self.focus_node_for_key(key))
                .or_else(|| {
                    // Only a press slot's key earns the point lookup: a
                    // slot-less utility target keeps clearing focus when it
                    // lands outside every focusable node, exactly as before.
                    key.is_some()
                        .then(|| {
                            self.accessibility.node_at_point_where(point, |node| {
                                node.supports_action(AccessibilityAction::Focus)
                                    && !node.is_hidden()
                                    && !node.is_disabled()
                            })
                        })
                        .flatten()
                });
            self.set_keyboard_focus_node(node, false)
        }
        #[cfg(not(feature = "accessibility"))]
        {
            let _ = point;
            self.set_keyboard_focus(key, false)
        }
    }

    pub(crate) fn set_keyboard_focus(
        &mut self,
        focus: Option<InteractionKey>,
        visible: bool,
    ) -> bool {
        #[cfg(feature = "accessibility")]
        let node = focus.as_ref().and_then(|key| self.focus_node_for_key(key));
        let text_input = focus.as_ref().and_then(|key| {
            self.text_editing
                .text_input_targets
                .iter()
                .position(|target| &target.interaction_key == key)
        });
        let embedded = focus
            .as_ref()
            .and_then(|key| self.embedded_index_for_key(key));
        let mut changed = self.set_keyboard_focus_impl(
            focus,
            #[cfg(feature = "accessibility")]
            node,
            visible,
        );
        changed |= self.hit_test.set_embedded_focus_index(embedded);
        // This setter claims the caret only for a text-input key. Ending
        // editing is the caller's decision — traversal and pointer presses
        // clear it through `set_focused_text_input(None)`.
        if let Some(index) = text_input {
            changed |= self.set_focused_text_input(Some(index));
        }
        changed
    }

    pub(crate) fn set_keyboard_focus_impl(
        &mut self,
        focus: Option<InteractionKey>,
        #[cfg(feature = "accessibility")] node: Option<AccessibilityNodeId>,
        visible: bool,
    ) -> bool {
        let visible = focus.is_some() && visible;
        #[cfg(feature = "accessibility")]
        let node_changed = self.accessibility.focus != node.unwrap_or(ACCESSIBILITY_ROOT_NODE_ID);
        #[cfg(not(feature = "accessibility"))]
        let node_changed = false;
        if self.hit_test.keyboard_focus == focus
            && self.hit_test.keyboard_focus_visible == visible
            && !node_changed
        {
            return false;
        }
        let leaving = self.focused_candidate_order();
        if self.hit_test.keyboard_focus != focus || node_changed {
            // A `.focused` lens on the field still holding the caret is owned
            // by the text machinery: semantic focus moving to a non-text node
            // leaves UI focus — and the binding — on the field. Only a
            // transition that actually moves the caret (a new text target) or
            // ends text focus lets this write stand.
            let caret_keeps_binding = self.hit_test.keyboard_focus.is_some()
                && self.hit_test.keyboard_focus == self.text_editing.focused_key()
                && !self
                    .text_editing
                    .text_input_targets
                    .iter()
                    .any(|target| Some(&target.interaction_key) == focus.as_ref());
            if let Some(binding) = self.hit_test.keyboard_focus_binding.take()
                && !caret_keeps_binding
            {
                binding.set(false);
            }
            self.hit_test.keyboard_focus = focus;
            self.hit_test.keyboard_focus_binding = self
                .hit_test
                .pointer_targets
                .iter()
                .filter_map(|target| target.press_slot.as_ref())
                .find(|slot| {
                    self.hit_test
                        .keyboard_focus
                        .as_ref()
                        .is_some_and(|focused| &slot.key == focused)
                })
                .and_then(|slot| slot.focus_binding.clone());
            #[cfg(feature = "accessibility")]
            {
                if self.hit_test.keyboard_focus_binding.is_none() {
                    self.hit_test.keyboard_focus_binding = node.and_then(|node| {
                        self.accessibility
                            .focus_bindings
                            .get(&node)
                            .map(|binding| binding.focused().clone())
                    });
                }
                self.accessibility.focus = node.unwrap_or(ACCESSIBILITY_ROOT_NODE_ID);
            }
            if self.hit_test.keyboard_focus_binding.is_none() {
                self.hit_test.keyboard_focus_binding = self
                    .hit_test
                    .keyboard_focus
                    .as_ref()
                    .and_then(|focused| self.embedded_index_for_key(focused))
                    .and_then(|index| {
                        self.hit_test.embedded_input_targets[index]
                            .focus_binding
                            .clone()
                    });
            }
            if let Some(binding) = self.hit_test.keyboard_focus_binding.as_ref() {
                binding.set(true);
            }
        }
        // The anchor tracks whichever focusable focus rests on now, or the
        // slot it just left — the slot's order still points at the same
        // tree position once that focusable disappears.
        self.hit_test.traversal_anchor = self
            .focused_candidate_order()
            .or(leaving)
            .or(self.hit_test.traversal_anchor);
        self.hit_test.keyboard_focus_visible = visible;
        // Both ends of the move repaint: the owner focus left drops its
        // focus visuals, the one it reached takes them. The leaving owner
        // was captured when focus arrived, so a target removed since still
        // resolves (a dead owner has nothing left to wake).
        let left_owner = core::mem::take(&mut self.hit_test.keyboard_focus_owner);
        if let Some(key) = self.hit_test.keyboard_focus.clone() {
            // A semantic-only focusable (the semantic runtime registers no
            // hit targets) is owned by the cell that emitted its node.
            let owner = self.try_key_owner(&key);
            #[cfg(feature = "accessibility")]
            let owner = owner.or_else(|| node.and_then(|node| self.live_node_owner(node)));
            let owner = owner
                .unwrap_or_else(|| panic!("hydrolysis renderer: focused key {key:?} has no owner"));
            Self::mark_owner(&owner, Dirty::LAYOUT);
            self.hit_test.keyboard_focus_owner = owner;
        }
        Self::mark_owner(&left_owner, Dirty::LAYOUT);
        // The transition itself is window bookkeeping — same split as
        // `set_focused_text_input_key`: the key's owner takes the focus
        // visuals, the context takes the deadline and binding writes.
        self.context_mark_layout();
        true
    }

    /// The tree order of the candidate keyboard focus currently rests on —
    /// the focused node's slot in the emitted tree, matching the `order`
    /// accessibility candidates carry.
    #[cfg(feature = "accessibility")]
    fn focused_candidate_order(&self) -> Option<usize> {
        let node = self.keyboard_focus_node().or_else(|| {
            self.hit_test
                .keyboard_focus
                .as_ref()
                .and_then(|key| self.focus_node_for_key(key))
        });
        node.and_then(|node| {
            self.accessibility
                .nodes
                .iter()
                .position(|(id, _)| *id == node)
        })
    }

    /// The tree order of the candidate keyboard focus currently rests on —
    /// the emission order of the target behind it, matching the `order`
    /// pointer/text-input candidates carry.
    #[cfg(not(feature = "accessibility"))]
    fn focused_candidate_order(&self) -> Option<usize> {
        self.hit_test.keyboard_focus.as_ref().and_then(|focused| {
            self.hit_test
                .pointer_targets
                .iter()
                .find(|target| {
                    target
                        .press_slot
                        .as_ref()
                        .is_some_and(|slot| &slot.key == focused)
                })
                .map(|target| target.order)
                .or_else(|| {
                    self.text_editing
                        .text_input_targets
                        .iter()
                        .find(|target| &target.interaction_key == focused)
                        .map(|target| target.order)
                })
                .or_else(|| {
                    self.hit_test
                        .embedded_input_targets
                        .iter()
                        .find(|target| &target.interaction_key == focused)
                        .map(|target| target.order)
                })
        })
    }

    /// The traversal order is the semantic order of the emitted tree: the
    /// nodes that advertise `Focus`, in emission order. The rendered runtime
    /// emits the same tree, so both runtimes share this single source — the
    /// pointer-target list plays no part in it.
    #[cfg(feature = "accessibility")]
    fn keyboard_focus_candidates(&self) -> Vec<KeyboardFocusCandidate> {
        let modal_active = self.modal_shield_active();
        self.accessibility
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, (_, node))| {
                node.supports_action(AccessibilityAction::Focus) && !node.is_hidden()
            })
            .filter_map(|(order, (node_id, _))| {
                let node_id = *node_id;
                let text_input = self
                    .text_editing
                    .text_input_targets
                    .iter()
                    .position(|target| target.accessibility_node_id == Some(node_id));
                let key = self.focus_key_for_node(node_id);
                // A widget may bind its interaction identity out of keyboard
                // focus while still advertising `Focus` to assistive clients —
                // the pointer opt-out wins for keyboard traversal.
                if key.as_ref().is_some_and(|key| {
                    self.hit_test.pointer_targets.iter().any(|target| {
                        !target.keyboard_focusable
                            && target
                                .press_slot
                                .as_ref()
                                .is_some_and(|slot| &slot.key == key)
                    })
                }) {
                    return None;
                }
                if modal_active {
                    let modal = key.as_ref().is_some_and(|key| {
                        self.hit_test.pointer_targets.iter().any(|target| {
                            target.modal
                                && target
                                    .press_slot
                                    .as_ref()
                                    .is_some_and(|slot| &slot.key == key)
                        })
                    }) || text_input
                        .is_some_and(|index| self.text_editing.text_input_targets[index].modal);
                    if !modal {
                        return None;
                    }
                }
                let embedded = self.embedded_index_for_node(node_id);
                Some(KeyboardFocusCandidate {
                    key,
                    node: node_id,
                    text_input,
                    embedded,
                    order,
                })
            })
            .collect()
    }

    #[cfg(not(feature = "accessibility"))]
    fn keyboard_focus_candidates(&self) -> Vec<KeyboardFocusCandidate> {
        let modal_active = self.modal_shield_active();
        let mut candidates = Vec::<KeyboardFocusCandidate>::new();
        for target in &self.hit_test.pointer_targets {
            let Some(slot) = target.press_slot.as_ref() else {
                continue;
            };
            if (modal_active && !target.modal)
                || !target.keyboard_focusable
                || candidates
                    .iter()
                    .any(|candidate| candidate.key.as_ref() == Some(&slot.key))
            {
                continue;
            }
            candidates.push(KeyboardFocusCandidate {
                key: Some(slot.key.clone()),
                text_input: None,
                embedded: None,
                order: target.order,
            });
        }
        for (index, target) in self.text_editing.text_input_targets.iter().enumerate() {
            if (modal_active && !target.modal)
                || candidates
                    .iter()
                    .any(|candidate| candidate.key.as_ref() == Some(&target.interaction_key))
            {
                continue;
            }
            candidates.push(KeyboardFocusCandidate {
                key: Some(target.interaction_key.clone()),
                text_input: Some(index),
                embedded: None,
                order: target.order,
            });
        }
        for (index, target) in self.hit_test.embedded_input_targets.iter().enumerate() {
            if modal_active
                || candidates
                    .iter()
                    .any(|candidate| candidate.key.as_ref() == Some(&target.interaction_key))
            {
                continue;
            }
            candidates.push(KeyboardFocusCandidate {
                key: Some(target.interaction_key.clone()),
                text_input: None,
                embedded: Some(index),
                order: target.order,
            });
        }
        candidates.sort_unstable_by_key(|candidate| candidate.order);
        candidates
    }

    pub(crate) fn move_keyboard_focus(&mut self, reverse: bool) -> bool {
        let candidates = self.keyboard_focus_candidates();
        if candidates.is_empty() {
            #[cfg(feature = "accessibility")]
            {
                return self.set_keyboard_focus_node(None, false);
            }
            #[cfg(not(feature = "accessibility"))]
            {
                return self.set_keyboard_focus(None, false);
            }
        }
        #[cfg(feature = "accessibility")]
        let current = candidates
            .iter()
            .position(|candidate| Some(candidate.node) == self.keyboard_focus_node());
        #[cfg(not(feature = "accessibility"))]
        let current = self.hit_test.keyboard_focus.as_ref().and_then(|focused| {
            candidates
                .iter()
                .position(|candidate| candidate.key.as_ref() == Some(focused))
        });
        // With no live focus to start from, resume at the slot focus last
        // held — `traversal_anchor` survives the focusable that held it
        // going hidden or unmounted, so the next Tab lands on the nearest
        // still-visible candidate in tree order rather than restarting.
        let anchor = self.hit_test.traversal_anchor;
        let next = if reverse {
            current.map_or_else(
                || {
                    anchor.map_or(candidates.len() - 1, |anchor| {
                        candidates
                            .iter()
                            .rposition(|candidate| candidate.order < anchor)
                            .unwrap_or(candidates.len() - 1)
                    })
                },
                |index| index.checked_sub(1).unwrap_or(candidates.len() - 1),
            )
        } else {
            current.map_or_else(
                || {
                    anchor.map_or(0, |anchor| {
                        candidates
                            .iter()
                            .position(|candidate| candidate.order > anchor)
                            .unwrap_or(0)
                    })
                },
                |index| (index + 1) % candidates.len(),
            )
        };
        let candidate = &candidates[next];
        let text_input = candidate.text_input;
        let embedded = candidate.embedded;
        #[cfg(feature = "accessibility")]
        let changed =
            self.set_keyboard_focus_impl(candidate.key.clone(), Some(candidate.node), true);
        #[cfg(not(feature = "accessibility"))]
        let changed = self.set_keyboard_focus(candidate.key.clone(), true);
        let mut changed = changed;
        changed |= self.hit_test.set_embedded_focus_index(embedded);
        // The caret follows keyboard focus: traversal onto a field focuses
        // it for editing, and a non-text candidate ends editing exactly as
        // a pointer press on one does.
        changed |= self.set_focused_text_input(text_input);
        changed
    }

    /// The row-navigation context of a focused `List` row: the row siblings
    /// under its `List` parent in emission order, the focused row's index
    /// among them, and the list's scroll axis — so arrows know which pair of
    /// keys steps and `Home`/`End` know the edges. `None` when the focused
    /// node is not a `List` row: a node is a row when it carries the
    /// `ListRow` action target, which section chrome never has, so role
    /// hoisting and chrome nodes can neither fake nor lose row membership
    /// (water-rs/waterui#1223).
    #[cfg(feature = "accessibility")]
    fn list_row_context(
        &self,
        node: AccessibilityNodeId,
    ) -> Option<(Vec<AccessibilityNodeId>, usize, ScrollAxis)> {
        if !matches!(
            self.accessibility.actions.get(&node),
            Some(AccessibilityActionTarget::ListRow { .. })
        ) {
            return None;
        }
        let (parent_id, parent) = self.accessibility.nodes.iter().find(|(_, emitted)| {
            emitted.role() == AccessibilityNodeRole::List && emitted.children().contains(&node)
        })?;
        let axis = match self.accessibility.actions.get(parent_id) {
            Some(AccessibilityActionTarget::Scroll { axis, .. }) => *axis,
            _ => ScrollAxis::Vertical,
        };
        let siblings: Vec<AccessibilityNodeId> = parent
            .children()
            .iter()
            .filter(|child| {
                matches!(
                    self.accessibility.actions.get(*child),
                    Some(AccessibilityActionTarget::ListRow { .. })
                )
            })
            .copied()
            .collect();
        let index = siblings.iter().position(|sibling| *sibling == node)?;
        Some((siblings, index, axis))
    }

    /// Land keyboard focus on `dest` row: reveal it through the accessibility
    /// tree first, then move the visible focus. Rows activate only through
    /// Enter/Space, resolved the same way a pointer click on the row's centre
    /// would be (water-rs/waterui#1223). A row belonging to a selectable list
    /// also writes the selection on the way: plain arrows move it to the
    /// destination row and Shift extends the anchored range
    /// (water-rs/waterui#1226).
    #[cfg(feature = "accessibility")]
    fn navigate_list_row(
        &mut self,
        dest: AccessibilityNodeId,
        env: &Environment,
        modifiers: Modifiers,
    ) -> bool {
        let _ = self.handle_accessibility_action(
            AccessibilityActionRequest {
                action: AccessibilityAction::ScrollIntoView,
                target_node: dest,
                target_tree: AccessibilityTreeId::ROOT,
                data: None,
            },
            env,
        );
        if let Some(AccessibilityActionTarget::ListRow {
            index,
            id,
            selection: Some(selection),
            ..
        }) = self.accessibility.actions.get(&dest)
        {
            selection.write(*index, *id, modifiers);
        }
        self.set_keyboard_focus_node(Some(dest), true)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
    )]
    pub(crate) fn handle_keyboard_key_down(
        &mut self,
        key: &KeyCode,
        logical_key: &keyboard_types::Key,
        modifiers: Modifiers,
        env: &Environment,
    ) -> bool {
        self.registries();
        // A declared menu chord claims its key before any built-in key
        // handling — Escape, Tab traversal, arrow stepping, activation and
        // focused text input — as AppKit offers key equivalents before
        // `keyDown` (water-rs/hydrolysis#247). The runner seeds the registry
        // into every window's environment — a missing one is a bug in the
        // runner, not an absent table. Chords match the press's W3C
        // `logical_key`. Without Control, Alt or Super a press is typing
        // only while a text editor holds focus: J into a focused field
        // types J, while J with nothing editable focused fires its chord —
        // the space bar included — and a bare `Delete` chord does not
        // steal the field's own delete either (water-rs/waterui#2118).
        let registry = env
            .get::<MenuShortcutRegistry>()
            .expect(MISSING_MENU_SHORTCUT_REGISTRY);
        let typing = !(modifiers.control || modifiers.alt || modifiers.super_key)
            && self.text_editing.has_focus();
        if !typing && registry.dispatch(self.window_id, logical_key, modifiers, env) {
            return true;
        }
        if matches!(key, KeyCode::Named(value) if value == "Escape")
            && let Some(modal) = self.hit_test.modal_interaction.clone()
            && modal.close_on_escape()
        {
            modal.handle_escape(env);
            return true;
        }
        if matches!(key, KeyCode::Named(value) if value == "Escape") {
            #[cfg(feature = "accessibility")]
            if let Some(action) = self
                .keyboard_focus_node()
                .and_then(|node| self.accessibility.focus_bindings.get(&node))
                .and_then(|binding| binding.escape_action_handle().cloned())
            {
                action.call(env);
                return true;
            }
            if let Some(focused) = self.hit_test.keyboard_focus.as_ref()
                && let Some(action) = self
                    .hit_test
                    .pointer_targets
                    .iter()
                    .rev()
                    .filter_map(|target| target.press_slot.as_ref())
                    .find(|slot| &slot.key == focused)
                    .and_then(|slot| slot.escape_action.clone())
            {
                action.call(env);
                return true;
            }
        }
        // Tab traversal, including the Ctrl-modified chord: Ctrl+Tab is
        // the way keyboard focus leaves an input surface that consumes
        // plain Tab as input (GTK's text-view convention), so it traverses
        // everywhere. Alt and Super stay reserved for the window system.
        if matches!(key, KeyCode::Named(value) if value == "Tab")
            && !(modifiers.alt || modifiers.super_key)
        {
            return self.move_keyboard_focus(modifiers.shift);
        }
        let activates = matches!(key, KeyCode::Named(value) if value == "Enter" || value == "Space")
            || matches!(key, KeyCode::Character(value) if value == " ");
        let step_forward =
            matches!(key, KeyCode::Named(value) if value == "ArrowRight" || value == "ArrowUp");
        let step_backward =
            matches!(key, KeyCode::Named(value) if value == "ArrowLeft" || value == "ArrowDown");
        if step_forward || step_backward {
            #[cfg(feature = "accessibility")]
            {
                // A focused `List` row owns the arrows along the list's axis:
                // Up/Down in a vertical list, Left/Right in a horizontal one
                // step between sibling rows. The cross-axis pair keeps its
                // other meanings and falls through to the stepper handling
                // below.
                if let Some(node) = self.keyboard_focus_node()
                    && let Some((siblings, index, axis)) = self.list_row_context(node)
                {
                    let delta: isize = match (axis, key) {
                        (ScrollAxis::Vertical, KeyCode::Named(name)) if name == "ArrowUp" => -1,
                        (ScrollAxis::Vertical, KeyCode::Named(name)) if name == "ArrowDown" => 1,
                        (ScrollAxis::Horizontal, KeyCode::Named(name)) if name == "ArrowLeft" => -1,
                        (ScrollAxis::Horizontal, KeyCode::Named(name)) if name == "ArrowRight" => 1,
                        _ => 0,
                    };
                    if delta != 0 {
                        let Some(dest) = index
                            .checked_add_signed(delta)
                            .filter(|next| *next < siblings.len())
                            .map(|next| siblings[next])
                        else {
                            // First row Up / last row Down: the list consumes
                            // the key rather than dropping it or leaving the
                            // list.
                            return true;
                        };
                        return self.navigate_list_row(dest, env, modifiers);
                    }
                }
                if let Some(node) = self.keyboard_focus_node() {
                    let step_action = if step_forward {
                        AccessibilityAction::Increment
                    } else {
                        AccessibilityAction::Decrement
                    };
                    if self
                        .accessibility
                        .nodes
                        .iter()
                        .any(|(id, emitted)| *id == node && emitted.supports_action(step_action))
                    {
                        return self.handle_accessibility_action(
                            AccessibilityActionRequest {
                                action: step_action,
                                target_node: node,
                                target_tree: AccessibilityTreeId::ROOT,
                                data: None,
                            },
                            env,
                        );
                    }
                }
            }
            // A widget may bind a keyboard-step affordance without advertising
            // the matching semantic actions — the pointer-bound fallback
            // covers it.
            let modal_active = self.modal_shield_active();
            let Some(focused) = self.hit_test.keyboard_focus.as_ref() else {
                return false;
            };
            let Some(action) = self
                .hit_test
                .pointer_targets
                .iter()
                .rev()
                .find(|target| {
                    (!modal_active || target.modal)
                        && target
                            .press_slot
                            .as_ref()
                            .is_some_and(|slot| &slot.key == focused)
                })
                .and_then(|target| target.keyboard_step.clone())
            else {
                return false;
            };
            let changed = (action.borrow_mut())(step_forward);
            if changed {
                self.mark_key_owner(focused, Dirty::LAYOUT);
            }
            return true;
        }
        #[cfg(feature = "accessibility")]
        // `Home`/`End` jump the focused `List` row to the first and last row
        // — consumed, so the keys do not fall through to whoever else would
        // claim them.
        if let KeyCode::Named(name) = key
            && (name == "Home" || name == "End")
            && let Some(node) = self.keyboard_focus_node()
            && let Some((siblings, _, _)) = self.list_row_context(node)
        {
            if let Some(dest) = if name == "Home" {
                siblings.first()
            } else {
                siblings.last()
            } {
                return self.navigate_list_row(*dest, env, modifiers);
            }
            return true;
        }
        if !activates || modifiers.control || modifiers.alt || modifiers.super_key {
            return false;
        }
        #[cfg(feature = "accessibility")]
        if self.hit_test.keyboard_activation == KeyboardActivation::Semantic {
            let Some(node) = self.keyboard_focus_node() else {
                return false;
            };
            if self.focused_text_input_accessibility_node() != Some(node)
                && self.accessibility.nodes.iter().any(|(id, emitted)| {
                    *id == node && emitted.supports_action(AccessibilityAction::Click)
                })
            {
                return self.handle_accessibility_action(
                    AccessibilityActionRequest {
                        action: AccessibilityAction::Click,
                        target_node: node,
                        target_tree: AccessibilityTreeId::ROOT,
                        data: None,
                    },
                    env,
                );
            }
            // A focused node that does not advertise `Click` is not
            // activatable — the pointer-press fallback would fire an action
            // the semantics say does not exist.
            return false;
        }
        let Some(focused) = self.hit_test.keyboard_focus.as_ref() else {
            return false;
        };
        let modal_active = self.modal_shield_active();
        let Some(target) = self
            .hit_test
            .pointer_targets
            .iter()
            .rev()
            .find(|target| {
                (!modal_active || target.modal)
                    && target
                        .press_slot
                        .as_ref()
                        .is_some_and(|slot| &slot.key == focused)
            })
            .cloned()
        else {
            return false;
        };
        if target.captures_drag {
            return false;
        }
        if self.hit_test.active_keyboard_target.is_some() {
            return true;
        }
        let origin = target.bounds.center();
        if let Some(slot) = target.press_slot.as_ref() {
            self.hit_test
                .interaction
                .begin_press(slot, origin, self.frame_instant());
        }
        let bits = if target
            .interaction
            .as_ref()
            .is_some_and(|handles| handles.chrome_state_dependent())
        {
            Dirty::LAYOUT
        } else {
            Dirty::PAINT
        };
        Self::mark_owner(&target.owner, bits);
        self.hit_test.active_keyboard_target = Some(target);
        true
    }

    pub(crate) fn handle_keyboard_key_up(&mut self, key: &KeyCode, env: &Environment) -> bool {
        self.registries();
        let activates = matches!(key, KeyCode::Named(value) if value == "Enter" || value == "Space")
            || matches!(key, KeyCode::Character(value) if value == " ");
        if !activates {
            return false;
        }
        let Some(target) = self.hit_test.active_keyboard_target.take() else {
            return false;
        };
        let point = target.bounds.center();
        let action_changed = (target.action.borrow_mut())(self, point, env);
        let clear = self
            .hit_test
            .interaction
            .clear_all_presses(self.frame_instant());
        if action_changed {
            Self::mark_owner(&target.owner, Dirty::LAYOUT);
        }
        self.hit_test.interaction.mark_press_clear(&clear);
        true
    }

    /// A synthetic focus-out release — winit resends every held key as
    /// released when the window loses focus — aborts the press it belonged
    /// to rather than completing it: the armed activation target drops
    /// without firing and the pressed affordance comes down, so a real
    /// release arriving later has nothing stale left to activate.
    pub(crate) fn cancel_keyboard_press(&mut self) -> bool {
        let cancelled = !self.hit_test.bubbled_key_sinks.is_empty();
        for entry in self.hit_test.bubbled_key_sinks.drain(..) {
            entry.sink.key(&KeyDelivery {
                pressed: false,
                logical: &entry.logical,
                code: entry.code,
                repeat: false,
                modifiers: entry.modifiers,
            });
        }
        if self.hit_test.active_keyboard_target.take().is_none() {
            return cancelled;
        }
        let clear = self
            .hit_test
            .interaction
            .clear_all_presses(self.frame_instant());
        self.hit_test.interaction.mark_press_clear(&clear);
        true
    }
}

impl HydrolysisRenderer {
    /// The rendered-runtime gesture tick: advances the engine's recognizers,
    /// then fires an armed context-menu hold whose deadline elapsed. The
    /// deadline rides [`SemanticCore::next_gesture_deadline`], so the runner
    /// wakes for it — never a timer of its own. A hold that survives to the
    /// threshold opens the press point's menu through the same mount a
    /// secondary press takes, and consumes the press: pending recognizers
    /// fail as on a cancel, the captured pointer target drops, and a tap no
    /// longer fires on release.
    pub fn handle_gesture_tick(&mut self, at: Instant, env: &Environment) -> bool {
        self.registries();
        let gesture_changed = self.core.handle_gesture_tick(at, env);
        let Some(hold) = self.hit_test.pending_context_menu_hold else {
            return gesture_changed;
        };
        if at.duration_since(hold.started_at) < CONTEXT_MENU_HOLD_DURATION {
            return gesture_changed;
        }
        self.hit_test.pending_context_menu_hold = None;
        if !self.open_context_menu_at(hold.point, env) {
            return gesture_changed;
        }
        self.consume_active_press(at, env);
        true
    }

    /// Consumes the press the active pointer sequence carries: the
    /// content's press is cancelled — never completed — pending recognizers
    /// are failed, and the press's visual state releases. The shared
    /// teardown a context-menu hold commits with and a touch-scroll claim
    /// steals the sequence through.
    fn consume_active_press(&mut self, at: Instant, env: &Environment) {
        self.hit_test.pending_pointer_press = None;
        self.hit_test.active_pointer_target = None;
        self.hit_test.active_pointer_drag_target = None;
        self.hit_test.active_pointer_drag_owner = Weak::new();
        self.hit_test.active_pointer_drag_signature = None;
        self.clear_scrollbar_drag();
        self.text_editing.active_text_selection_drag = None;
        self.hit_test.active_press_bounds = None;
        self.hit_test.active_press_origin = None;
        let press_clear = self.hit_test.interaction.clear_all_presses(at);
        self.hit_test.interaction.mark_press_clear(&press_clear);
        let _ = self.gesture_engine.handle_pointer_cancel(at, env);
    }

    /// What a secondary press at `point` mounts: the menu of the
    /// `.context_menu` region claiming the spot — for an embedded surface,
    /// the topmost region enclosing its window rect; for anything else, the
    /// topmost region containing the point — shown through
    /// [`HydrolysisRenderer::show_context_menu`]. A touch or pen
    /// press-and-hold resolves here once it earns the gesture, so it takes
    /// the drawn presentation: the source lifts and the menu sits beside it.
    /// Returns whether a menu opened.
    fn open_context_menu_at(&mut self, point: kurbo::Point, env: &Environment) -> bool {
        let pointer_priority = self
            .hit_test
            .pointer_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.bounds.contains(point))
            .map(|(index, target)| {
                SemanticCore::target_hit_priority(target.depth, target.order, index)
            })
            .max();
        let text_priority = self.topmost_text_input_index_at_point(point).map(|index| {
            let target = &self.text_editing.text_input_targets[index];
            SemanticCore::target_hit_priority(target.depth, target.order, index)
        });
        let menu_target = if let Some((_, surface, _)) =
            self.embedded_target_wins_at(point, pointer_priority, text_priority)
        {
            let surface_bounds = surface.to_window_rect(kurbo::Rect::from_origin_size(
                kurbo::Point::ORIGIN,
                surface.local_bounds.size(),
            ));
            self.topmost_context_menu_target_enclosing(point, surface_bounds)
        } else {
            self.topmost_context_menu_target_at_point(point)
        };
        let mut items = menu_target
            .as_ref()
            .map(|target| {
                popup_menu_nodes(&target.items.snapshot(), &target.env, self.window_closable)
            })
            .unwrap_or_default();
        self.append_inspect_element_item(&mut items, point);
        if items.is_empty() {
            return false;
        }
        // Same inheritance the secondary-press arms apply: the declaring
        // view's environment layers over this dispatch's.
        let menu_env = menu_target
            .as_ref()
            .map_or_else(|| env.clone(), |target| target.env.layered_on(env));
        let metrics = self.theme().text_context_menu_metrics();
        self.show_context_menu(
            items,
            menu_target.as_ref(),
            LayoutPoint::new(
                crate::num_cast::f64_as_f32(point.x),
                crate::num_cast::f64_as_f32(point.y),
            ),
            metrics,
            &menu_env,
            true,
        )
    }

    /// Handles a cancelled pointer stream; returns whether anything consumed it.
    pub fn handle_pointer_cancel(&mut self, env: &Environment) -> bool {
        self.registries();
        let Some((pointer_id, pointer_kind)) = self.hit_test.active_pointer else {
            return false;
        };
        self.handle_pointer_cancel_with_source(pointer_id, pointer_kind, env)
    }

    /// Handles a cancelled pointer stream with the pointer's id and kind.
    pub fn handle_pointer_cancel_with_source(
        &mut self,
        pointer_id: u64,
        pointer_kind: PointerKind,
        env: &Environment,
    ) -> bool {
        self.registries();
        if self.hit_test.active_pointer != Some((pointer_id, pointer_kind)) {
            return false;
        }
        let at = self.frame_instant();
        let mut refresh_requested =
            self.finish_interactive_navigation_pop(NavigationInteractivePopOutcome::Cancel);
        self.text_editing.active_text_selection_drag = None;
        self.hit_test.active_pointer_drag_target = None;
        self.hit_test.active_pointer_drag_owner = Weak::new();
        self.hit_test.active_pointer_drag_signature = None;
        self.clear_scrollbar_drag();
        self.hit_test.active_pointer_target = None;
        self.hit_test.active_embedded_target = None;
        self.hit_test.active_pointer = None;
        self.hit_test.active_pointer_button = None;
        self.hit_test.pending_pointer_press = None;
        self.hit_test.pending_context_menu_hold = None;
        self.hit_test.active_press_bounds = None;
        self.hit_test.active_press_origin = None;
        self.hit_test.pointer_press_origin = None;
        self.hit_test.touch_scroll = TouchScrollGesture::Idle;
        refresh_requested |= self.cancel_active_drag(env);
        let press_clear = self.hit_test.interaction.clear_all_presses(at);
        self.hit_test.interaction.mark_press_clear(&press_clear);
        refresh_requested |= press_clear.chrome_changed;
        let frame_instant = self.core.frame_instant;
        let gesture_changed = self
            .core
            .gesture_engine
            .handle_pointer_cancel(frame_instant, env);
        refresh_requested |= gesture_changed;
        let mut hover_visual_changed = false;
        let mut hover_chrome_keys = Vec::new();
        let mut hover_paint_keys = Vec::new();
        let hit_test = &mut self.core.hit_test;
        for target in &mut hit_test.hover_targets {
            if !hit_test.interaction.key_active(&target.slot.key) {
                continue;
            }
            let hovering = hit_test.interaction.hovering(&target.slot);
            if !hovering {
                continue;
            }
            hit_test.interaction.set_hovering(&target.slot, false);
            let paint = target.handles.as_ref().is_some_and(|handles| {
                handles.set_hovering(false, at);
                hover_visual_changed = true;
                true
            });
            let chrome = target
                .on_exit
                .as_mut()
                .is_some_and(|on_exit| (on_exit.borrow_mut())(env));
            refresh_requested |= chrome;
            if chrome {
                hover_chrome_keys.push(target.slot.key.clone());
            } else if paint {
                hover_paint_keys.push(target.slot.key.clone());
            }
        }
        self.hit_test
            .interaction
            .mark_owners(&hover_chrome_keys, Dirty::LAYOUT);
        self.hit_test
            .interaction
            .mark_owners(&hover_paint_keys, Dirty::PAINT);
        refresh_requested || hover_visual_changed
    }

    /// Handles a scroll event at `(x, y)` with deltas `(dx, dy)`; `is_line_delta` selects line vs pixel units.
    pub fn handle_scroll(&mut self, x: f32, y: f32, dx: f32, dy: f32, is_line_delta: bool) -> bool {
        self.registries();
        let point = kurbo::Point::new(f64::from(x), f64::from(y));
        let unit = if is_line_delta {
            ScrollUnit::Line
        } else {
            ScrollUnit::Pixel
        };
        // A discrete wheel notch is complete on its own; a pixel-precise wheel
        // delta arriving through this path carries no phase, so neither ends a
        // gesture that the surface should settle.
        let scroll_priority = self
            .hit_test
            .scroll_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.bounds.contains(point))
            .map(|(index, target)| {
                SemanticCore::target_hit_priority(target.depth, target.order, index)
            })
            .max();
        if let Some((_, target, position)) =
            self.embedded_target_wins_at(point, scroll_priority, None)
        {
            target.sink.scroll(position, dx, dy, unit, is_line_delta);
            return true;
        }
        // Newest-registered first: every scroll container registers its target
        // before flushing its children, so a nested scroll region hit-tests
        // ahead of the one enclosing it. A region that cannot move — at its
        // edge, or on the delta's dead axis — does not consume; the delta
        // falls through to the next enclosing region.
        for target in self.hit_test.scroll_targets.iter_mut().rev() {
            if !target.bounds.contains(point) {
                continue;
            }
            let changed = (target.action.borrow_mut())(dx, dy, is_line_delta);
            if !changed {
                continue;
            }
            // A scroll offset is transform-level state outside the
            // reactive graph: the retained tree must re-encode at the
            // new offset (scene, hit-test geometry, accessibility),
            // but its placements are unchanged — no layout. The mark
            // lands on the target's owner; a dead owner retired with
            // its record, so there is nothing left to wake.
            if let Some(cell) = target.owner.upgrade() {
                cell.mark(Dirty::LAYOUT);
            }
            self.dismiss_active_text_context_menu();
            // A scroll inside the context-menu presentation — its
            // drawn menu or its accessory — belongs to it and does
            // not close the menu.
            if !self.context_menu_presentation_contains(point) {
                self.dismiss_active_popup_menu();
            }
            return true;
        }
        false
    }

    #[cfg(test)]
    pub(crate) fn scroll_metrics_at(&self, x: f32, y: f32) -> Option<crate::scroll::ScrollMetrics> {
        let point = kurbo::Point::new(f64::from(x), f64::from(y));
        self.hit_test
            .scroll_targets
            .iter()
            .rev()
            .find(|target| target.bounds.contains(point))
            .map(|target| target.handle.metrics())
    }

    /// Handles a trackpad pan at `(x, y)` with deltas and a touch phase.
    pub fn handle_trackpad_pan(
        &mut self,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        phase: TouchPhase,
    ) -> bool {
        self.registries();
        let point = kurbo::Point::new(f64::from(x), f64::from(y));
        let finished = matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled);
        let contender_priority = self
            .hit_test
            .scroll_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.bounds.contains(point))
            .map(|(index, target)| {
                SemanticCore::target_hit_priority(target.depth, target.order, index)
            })
            .max();
        if let Some((_, target, position)) =
            self.embedded_target_wins_at(point, contender_priority, None)
        {
            target
                .sink
                .scroll(position, dx, dy, ScrollUnit::Pixel, finished);
            return true;
        }
        self.handle_scroll(x, y, dx, dy, false)
    }

    /// Arms the touch-scroll claim for a `PointerKind::Touch` sequence: a
    /// running fling stops where it is, and the new press watches for a
    /// slop crossing while its press still lands on the content under it.
    fn arm_touch_scroll(&mut self, point: kurbo::Point, button: PointerButton, at: Instant) {
        self.hit_test.touch_fling = None;
        self.hit_test.touch_scroll =
            if button == PointerButton::Primary && self.hit_test.touch_scroll_config.is_some() {
                TouchScrollGesture::Pending(TouchScrollPending {
                    origin: point,
                    tracker: VelocityTracker::new(at, point),
                })
            } else {
                TouchScrollGesture::Idle
            };
    }

    /// Drives the touch-drag scroll gesture for a `PointerKind::Touch`
    /// move and reports whether the sequence is the gesture's: while a
    /// scroll view owns the drag every delta scrolls it one to one; while
    /// the sequence is still pending, the first move past the touch slop
    /// lets the innermost enclosing scroll view able to scroll along the
    /// movement's dominant axis claim it — cancelling the content's press,
    /// never completing it.
    fn handle_touch_scroll_move(
        &mut self,
        point: kurbo::Point,
        at: Instant,
        env: &Environment,
    ) -> bool {
        let gesture = core::mem::replace(&mut self.hit_test.touch_scroll, TouchScrollGesture::Idle);
        match gesture {
            TouchScrollGesture::Dragging(mut drag) => {
                drag.tracker.record(at, point);
                // The offset moves opposite the finger: a drag up pushes it
                // down into the content.
                let changed = drag.handle.apply_gesture_delta(
                    &drag.claim,
                    drag.last.x - point.x,
                    drag.last.y - point.y,
                );
                drag.last = point;
                let owner = drag.owner.clone();
                self.hit_test.touch_scroll = TouchScrollGesture::Dragging(drag);
                if changed {
                    SemanticCore::mark_owner(&owner, Dirty::LAYOUT);
                }
                true
            }
            TouchScrollGesture::Pending(mut pending) => {
                pending.tracker.record(at, point);
                let displacement_x = point.x - pending.origin.x;
                let displacement_y = point.y - pending.origin.y;
                let (dominant, dominant_x) = if displacement_x.abs() > displacement_y.abs() {
                    (displacement_x, true)
                } else {
                    (displacement_y, false)
                };
                let slop = self
                    .hit_test
                    .touch_scroll_config
                    .map_or(0.0, |config| f64::from(config.touch_slop));
                if dominant.abs() <= slop {
                    self.hit_test.touch_scroll = TouchScrollGesture::Pending(pending);
                    return false;
                }
                if self.hit_test.active_embedded_target.is_some() {
                    // The press landed on a surface that owns its input; a
                    // scroll view never steals a sequence from it.
                    return false;
                }
                let Some((handle, owner)) =
                    self.touch_scroll_claimant(pending.origin, displacement_x, displacement_y)
                else {
                    return false;
                };
                // The claim steals the sequence: the content's press is
                // cancelled — never completed — through the same teardown a
                // context-menu hold commits with.
                self.hit_test.pending_context_menu_hold = None;
                self.consume_active_press(at, env);
                // The crossing move applies its excess over the slop at
                // once — the content slides under the finger with no jump,
                // anchored where the dominant axis left the slop.
                let anchor = if dominant_x {
                    kurbo::Point::new(pending.origin.x + slop.copysign(dominant), pending.origin.y)
                } else {
                    kurbo::Point::new(pending.origin.x, pending.origin.y + slop.copysign(dominant))
                };
                let claim = handle.begin_gesture();
                let changed =
                    handle.apply_gesture_delta(&claim, anchor.x - point.x, anchor.y - point.y);
                self.hit_test.touch_scroll = TouchScrollGesture::Dragging(TouchScrollDrag {
                    handle,
                    claim,
                    owner: owner.clone(),
                    last: point,
                    tracker: pending.tracker,
                });
                if changed {
                    SemanticCore::mark_owner(&owner, Dirty::LAYOUT);
                }
                true
            }
            TouchScrollGesture::Idle => false,
        }
    }

    /// The innermost scroll view under the touch-down point able to move
    /// along the drag's dominant axis and direction — the claimant of a
    /// past-slop touch drag. Newest-registered first, the same hit order
    /// [`Self::handle_scroll`] walks.
    fn touch_scroll_claimant(
        &self,
        origin: kurbo::Point,
        displacement_x: f64,
        displacement_y: f64,
    ) -> Option<(crate::scroll::ScrollHandle, Weak<crate::renderer::NodeCell>)> {
        const EPSILON: f64 = 0.000_01;
        let dominant_x = displacement_x.abs() > displacement_y.abs();
        self.hit_test
            .scroll_targets
            .iter()
            .rev()
            .find_map(|target| {
                if !target.bounds.contains(origin) {
                    return None;
                }
                let metrics = target.handle.metrics();
                // The finger's sign flips into the offset's: a drag up
                // (dy < 0) pushes the offset toward its max, so the view
                // can claim only while that room remains.
                let able = match (target.handle.axis(), dominant_x) {
                    (ScrollAxis::Horizontal | ScrollAxis::All, true) => {
                        if displacement_x > 0.0 {
                            metrics.offset_x > EPSILON
                        } else {
                            metrics.offset_x < metrics.max_x - EPSILON
                        }
                    }
                    (ScrollAxis::Vertical | ScrollAxis::All, false) => {
                        if displacement_y > 0.0 {
                            metrics.offset_y > EPSILON
                        } else {
                            metrics.offset_y < metrics.max_y - EPSILON
                        }
                    }
                    (ScrollAxis::Horizontal, false) | (ScrollAxis::Vertical, true) => false,
                    _ => panic!("scroll axis variant is not supported by hydrolysis"),
                };
                able.then(|| (target.handle.clone(), target.owner.clone()))
            })
    }

    /// Releases the touch-drag scroll gesture: a claimed drag whose
    /// release velocity earns a fling starts it on the claimed handle;
    /// anything else ends the gesture plainly. Returns whether the gesture
    /// changed something the frame must show.
    fn finish_touch_scroll(&mut self, at: Instant) -> bool {
        let gesture = core::mem::replace(&mut self.hit_test.touch_scroll, TouchScrollGesture::Idle);
        let TouchScrollGesture::Dragging(drag) = gesture else {
            return false;
        };
        let Some(config) = self.hit_test.touch_scroll_config else {
            return false;
        };
        let fling = TouchFling::start(
            drag.handle,
            drag.claim,
            drag.tracker.velocity(at),
            &config,
            at,
        );
        self.hit_test.touch_fling = fling;
        self.hit_test.touch_fling.is_some()
    }
}

impl SemanticCore {
    /// The `OnKeyPress` chain enclosing a registration, folded into
    /// [`HitTestState::root_key_handlers`]: with nothing focused, a key
    /// bubbles only through the scopes enclosing every registration of the
    /// frame — the longest common ancestor of the snapshots.
    pub(crate) fn snapshot_key_handlers(&mut self) -> Option<Rc<KeyHandlerNode>> {
        let chain = self.key_handler_stack.clone();
        if self.hit_test.root_key_chain_seen {
            self.hit_test.root_key_handlers =
                common_key_handler_scope(self.hit_test.root_key_handlers.take(), chain.clone());
        } else {
            self.hit_test.root_key_chain_seen = true;
            self.hit_test.root_key_handlers.clone_from(&chain);
        }
        chain
    }

    pub(crate) fn register_pointer_target<F>(&mut self, bounds: kurbo::Rect, action: F)
    where
        F: 'static + FnMut(&mut Self, kurbo::Point, &Environment) -> bool,
    {
        self.register_pointer_target_action(
            bounds,
            false,
            None,
            Rc::new(RefCell::new(action)),
            self.render_depth,
        );
    }

    pub(crate) fn register_pointer_drag_target<F>(&mut self, bounds: kurbo::Rect, action: F)
    where
        F: 'static + FnMut(&mut Self, kurbo::Point, &Environment) -> bool,
    {
        self.register_pointer_target_action(
            bounds,
            true,
            None,
            Rc::new(RefCell::new(action)),
            self.render_depth,
        );
    }

    /// Computes a platform-view leaf's placement in window hit-test space:
    /// the leaf's rect clipped to the open hit clips (a scrolled-off child
    /// keeps only its remaining slice) plus the hit-test order claimed on its
    /// behalf — the same order every interactive target uses, which is what
    /// makes "placed later" mean "stacked above" for the host.
    ///
    /// Returns `(clipped bounds, order)`; an empty clipped rect means the leaf
    /// is fully clipped away.
    /// Pushes the leaf's retained placement record; the resolved bounds and
    /// paint rank are written into the sink table at materialization.
    pub(crate) fn register_platform_view_placement(
        &mut self,
        local: kurbo::Rect,
        table: Rc<RefCell<crate::platform_view::PlatformViewTable>>,
        placement: crate::platform_view::PlatformViewPlacement,
    ) {
        self.register_retained(
            mount::PlatformViewRegistration { table, placement },
            local,
            |regs| &mut regs.platform_view_placements,
        );
    }

    pub(crate) fn register_pointer_target_action(
        &mut self,
        bounds: kurbo::Rect,
        captures_drag: bool,
        press_slot: Option<PressSlot>,
        action: PointerAction,
        depth: usize,
    ) {
        let interaction = press_slot
            .as_ref()
            .and_then(|slot| self.hit_test.interaction.handles_for(slot));
        let modal = press_slot.as_ref().is_some_and(|slot| slot.modal);
        let key_handlers = self.snapshot_key_handlers();
        self.register_retained(
            PointerTarget {
                owner: Weak::new(),
                bounds,
                captures_drag,
                depth,
                order: 0,
                press_slot,
                owners: self.owner_stack.clone(),
                interaction,
                action,
                keyboard_step: None,
                keyboard_focusable: false,
                modal,
                key_handlers,
            },
            bounds,
            |regs| &mut regs.pointer_targets,
        );
    }

    /// Registers an opaque occlusion for `bounds`: an overlay's painted panel
    /// must own every press inside it, whatever hit regions the content
    /// beneath carries (water-rs/hydrolysis#260).
    ///
    /// The pointer target runs first in dispatch order for its rect —
    /// `action` answers `true`, so a press inside the panel is consumed
    /// without touching content press targets underneath. The
    /// `gesture_occluders` record is the gesture side of the same contract:
    /// when the engine picks its candidates at pointer-down, targets whose
    /// order is below the covering occluder's never reach it — no recognizer
    /// under the panel is armed, of any kind, so nothing fires on release or
    /// hold.
    ///
    /// Register the occluder BEFORE the overlay's own controls flush — they
    /// take a later order and outrank it inside the panel. `bounds` is the
    /// panel's painted rect in hit space; a press outside it is the
    /// dismiss-and-pass-through the overlay already implements, which this
    /// target must not shadow.
    /// `bounds` is already in window space (overlay frames are resolved
    /// before registration). The records still anchor at the recording
    /// placement — the presentation host's own identity-chain slot — so
    /// the occluder ranks under the host's paint order (a window-anchored
    /// occluder would rank above the host's own content and swallow its
    /// clicks), while the identity transform keeps the window-space rect
    /// verbatim.
    pub(crate) fn register_hit_test_occluder(&mut self, bounds: kurbo::Rect) {
        let key_handlers = self.snapshot_key_handlers();
        let anchor = self.current_placement();
        self.register_retained_at(
            PointerTarget {
                owner: Weak::new(),
                bounds,
                captures_drag: false,
                depth: self.render_depth,
                order: 0,
                press_slot: None,
                owners: self.owner_stack.clone(),
                interaction: None,
                action: Rc::new(RefCell::new(
                    |_: &mut Self, _: kurbo::Point, _: &Environment| true,
                )),
                keyboard_step: None,
                keyboard_focusable: false,
                modal: false,
                key_handlers,
            },
            bounds,
            &anchor,
            |regs| &mut regs.pointer_targets,
        );
        self.register_retained_at((), bounds, &anchor, |regs| &mut regs.gesture_occluders);
    }

    /// Runs `f` against the gesture engine with every target occluded at
    /// `point` filtered out of its candidate list, then restores the list.
    ///
    /// The engine arms every recognizer in the topmost group under the point
    /// — a choice it can only make among what it can see. Two kinds of
    /// occluder hide the targets painted underneath:
    ///
    /// - The explicit `gesture_occluders` an overlay registers for its painted
    ///   panel bounds: candidates offered for a press inside the panel are
    ///   exactly the registrations that outrank the highest covering
    ///   occluder, so the overlay's own controls stay armable and nothing
    ///   below the panel can arm (water-rs/hydrolysis#260).
    /// - Every pointer target covering the point is an occluder for the
    ///   gesture regions it outranks: a press belongs to the topmost
    ///   hittable target at the point, whichever engine carries it, so a
    ///   control painted over content — the layer a `when` mounts over a
    ///   list, a sibling in a `zstack` — stops the regions beneath it
    ///   without registering a panel occluder. Ancestry relaxes the rule:
    ///   a press target whose owner sits in the gesture's owner chain (a
    ///   container's press, or the view the gesture is attached to) never
    ///   hides that gesture, since nested targets settle the press among
    ///   themselves.
    ///
    /// [`GestureEngine::swap_targets`] splices the filtered list in for the
    /// duration of the call. The clone shares each target's recognizer, so
    /// the armed set and the restored list are the same state machines.
    pub(crate) fn with_unoccluded_gesture_targets<R>(
        &mut self,
        point: kurbo::Point,
        f: impl FnOnce(&mut crate::gesture::GestureEngine) -> R,
    ) -> R {
        let cutoff = self
            .hit_test
            .gesture_occluders
            .iter()
            .filter(|(bounds, _)| bounds.contains(point))
            .map(|(_, order)| *order)
            .max();
        let covering_pointers: Vec<&PointerTarget> = self
            .hit_test
            .pointer_targets
            .iter()
            .filter(|target| target.bounds.contains(point))
            .collect();
        if cutoff.is_none() && covering_pointers.is_empty() {
            return f(&mut self.gesture_engine);
        }
        let occluded_by_pointer = |target: &crate::gesture::GestureTarget| {
            covering_pointers.iter().any(|pointer| {
                pointer.order > target.order
                    && !pointer.owners.last().is_some_and(|owner| {
                        self.hit_test
                            .gesture_regions
                            .iter()
                            .find(|region| region.order == target.order)
                            .is_some_and(|region| region.owners.contains(owner))
                    })
            })
        };
        let mut all = Vec::new();
        self.gesture_engine.swap_targets(&mut all);
        let mut kept: Vec<crate::gesture::GestureTarget> = all
            .iter()
            .filter(|target| {
                cutoff.is_none_or(|cutoff| target.order >= cutoff) && !occluded_by_pointer(target)
            })
            .cloned()
            .collect();
        self.gesture_engine.swap_targets(&mut kept);
        let out = f(&mut self.gesture_engine);
        self.gesture_engine.swap_targets(&mut kept);
        self.gesture_engine.swap_targets(&mut all);
        out
    }

    /// Registers a scrollbar-gutter drag target: it captures the press like any
    /// drag target, but its changes are transform-level (a scroll offset), so
    /// they schedule a re-encode instead of a layout refresh.
    pub(crate) fn register_scrollbar_drag_target<F>(&mut self, bounds: kurbo::Rect, action: F)
    where
        F: 'static + FnMut(&mut Self, kurbo::Point, &Environment) -> bool,
    {
        let key_handlers = self.snapshot_key_handlers();
        self.register_retained(
            PointerTarget {
                owner: Weak::new(),
                bounds,
                captures_drag: true,
                depth: self.render_depth,
                order: 0,
                press_slot: None,
                owners: self.owner_stack.clone(),
                interaction: None,
                action: Rc::new(RefCell::new(action)),
                keyboard_step: None,
                keyboard_focusable: false,
                modal: false,
                key_handlers,
            },
            bounds,
            |regs| &mut regs.pointer_targets,
        );
    }

    /// The grab offset of the in-flight scrollbar drag owned by `key`, if any.
    pub(crate) fn scrollbar_drag_grab(&self, key: usize) -> Option<f64> {
        self.hit_test
            .active_scrollbar_drag
            .as_ref()
            .filter(|drag| drag.key == key)
            .map(|drag| drag.grab)
    }

    /// Records the start of a scrollbar-thumb drag for the scroll slot `key`.
    /// The drag's owner is the `ScrollTarget` holding that handle — the
    /// dispatch runs outside any record, so it looks the owner up through
    /// the registry rather than asking the reader.
    pub(crate) fn begin_scrollbar_drag(&mut self, key: usize, grab: f64) {
        let owner = self.scroll_target_owner(key);
        self.hit_test.active_scrollbar_drag = Some(ScrollbarDrag { key, grab, owner });
    }

    /// Whether the scroll slot `key` currently owns a scrollbar-thumb drag
    /// (drawn with a widened thumb).
    pub(crate) fn scrollbar_drag_active(&self, key: usize) -> bool {
        self.hit_test
            .active_scrollbar_drag
            .as_ref()
            .is_some_and(|drag| drag.key == key)
    }

    /// Register the drag source for `draggable`. The payload is read when
    /// the drag begins — the action snapshots the [`Draggable`]'s signal then,
    /// so a binding-backed payload carries its current value.
    pub(crate) fn register_draggable_target(
        &mut self,
        bounds: kurbo::Rect,
        draggable: Rc<Draggable>,
    ) {
        self.register_pointer_target_action(
            bounds,
            true,
            None,
            Rc::new(RefCell::new(
                move |renderer: &mut Self, point: kurbo::Point, env: &Environment| {
                    renderer.begin_or_update_drag(draggable.payload(), point, env)
                },
            )),
            self.render_depth,
        );
    }

    /// Register a drop target from pre-wrapped, shareable handler handles. A
    /// retained `Wrapper` node wraps the destination's handlers once at build and
    /// re-registers the same `Rc`s on every flush (it holds the destination by
    /// reference and cannot move the handlers out each frame).
    pub(crate) fn register_drop_destination_handles(
        &mut self,
        bounds: kurbo::Rect,
        handles: &DropDestinationHandles,
        env: &Environment,
    ) {
        self.register_retained(
            DropTarget {
                owner: Weak::new(),
                bounds,
                key: DropTargetKey {
                    depth: self.render_depth,
                    order: 0,
                },
                env: env.clone(),
                destination: Rc::clone(&handles.destination),
            },
            bounds,
            |regs| &mut regs.drop_targets,
        );
    }
}

impl HydrolysisRenderer {
    pub(crate) fn bind_interaction_target(
        &mut self,
        key: InteractionKey,
        bounds: kurbo::Rect,
        env: &Environment,
    ) -> (
        WidgetInteractionState,
        PressSlot,
        Rc<InteractionLayerHandles>,
    ) {
        self.bind_interaction_target_with_focus(key, bounds, env, None, false)
    }

    /// Binds an interaction target for a control that supports the disabled
    /// state: a disabled control samples an at-rest [`WidgetInteractionState`]
    /// (with its `disabled` flag set) and registers no hover target, so the
    /// pointer neither hovers nor presses it.
    pub(crate) fn bind_control_interaction_target(
        &mut self,
        key: InteractionKey,
        bounds: kurbo::Rect,
        env: &Environment,
        disabled: bool,
    ) -> (
        WidgetInteractionState,
        PressSlot,
        Rc<InteractionLayerHandles>,
    ) {
        self.bind_interaction_target_with_focus(key, bounds, env, None, disabled)
    }

    pub(crate) fn bind_focused_control_interaction_target(
        &mut self,
        key: InteractionKey,
        bounds: kurbo::Rect,
        env: &Environment,
        focused: bool,
        disabled: bool,
    ) -> (
        WidgetInteractionState,
        PressSlot,
        Rc<InteractionLayerHandles>,
    ) {
        self.bind_interaction_target_with_focus(
            key,
            bounds,
            env,
            Some(InteractionFocus::visible(focused)),
            disabled,
        )
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
    )]
    fn bind_interaction_target_with_focus(
        &mut self,
        key: InteractionKey,
        bounds: kurbo::Rect,
        env: &Environment,
        focus: Option<InteractionFocus>,
        disabled: bool,
    ) -> (
        WidgetInteractionState,
        PressSlot,
        Rc<InteractionLayerHandles>,
    ) {
        // `WidgetInteractionInput::bounds` filters inherited press waves by
        // origin — origins are window-space points, so the check needs the
        // resolved window rect, not the node-local `bounds` the register
        // calls take.
        let window_bounds = self
            .current_placement()
            .resolve_hit(bounds)
            .map_or(kurbo::Rect::ZERO, |resolved| resolved.rect);
        let keyboard_focused = self.hit_test.keyboard_focus.as_ref() == Some(&key);
        let focus = if focus.is_some_and(|focus| focus.visible) {
            focus
        } else if keyboard_focused {
            Some(InteractionFocus::visible(
                self.hit_test.keyboard_focus_visible,
            ))
        } else {
            focus
        };
        let owner = self.registration_owner();
        let (hover_slot, hovered) = self.hit_test.interaction.bind_hover(&key, &owner);
        if disabled {
            // No hover target is registered for a disabled control, so pointer
            // moves never sync its slot; reset it so a stale hover from before
            // the control was disabled does not resurface on re-enable.
            self.hit_test.interaction.set_hovering(&hover_slot, false);
        }
        let motion = self.theme().interaction_motion();
        let now = self.frame_instant();
        let (mut state, mut press_slot, handles) =
            self.core.hit_test.interaction.bind_widget_state(
                &key,
                WidgetInteractionInput {
                    bounds: window_bounds,
                    hovered,
                    focus,
                    disabled,
                },
                &motion,
                &mut self.core.animation_controller,
                now,
                &owner,
            );
        if env
            .get::<ModalInteraction>()
            .is_some_and(waterui_backend_core::widget::ModalInteraction::is_active)
        {
            press_slot.modal = true;
            self.register_modal_scope(env);
        }
        if let Some(focus_binding) = env.get::<InteractionFocusBinding>() {
            press_slot.focus_binding = Some(focus_binding.focused().clone());
            press_slot.escape_action = focus_binding.escape_action_handle().cloned();
            if keyboard_focused {
                focus_binding.focused().set(true);
                self.hit_test.keyboard_focus_binding = Some(focus_binding.focused().clone());
            }
        }
        let flags = self.interaction_state_flags(env, &key, state);
        state.state = flags;
        self.hit_test.interaction.set_reported_state(&key, flags);
        self.claim_interaction_reports(env, flags);
        // Every widget that binds an interaction target draws its hover/focus/press
        // state layers from the sampled state each flush, so its chrome is
        // state-dependent by construction: a press or hover change must schedule a
        // re-flush (not just a re-present of the stale scene).
        handles.mark_chrome_state_dependent();
        if !disabled {
            self.core.register_retained(
                HoverTarget {
                    owner: Weak::new(),
                    bounds,
                    slot: hover_slot,
                    handles: Some(Rc::clone(&handles)),
                    on_enter: None,
                    on_move: None,
                    on_exit: None,
                },
                bounds,
                |regs| &mut regs.hover_targets,
            );
        }
        (state, press_slot, handles)
    }
}

impl SemanticCore {
    /// The `InteractionKey` of the pointer target currently owning the active
    /// drag, resolved through the persisted drag signature so the answer
    /// survives the target list being rebuilt between frames.
    fn active_pointer_drag_key(&self) -> Option<InteractionKey> {
        let (depth, order) = self.hit_test.active_pointer_drag_signature?;
        self.hit_test
            .pointer_targets
            .iter()
            .find(|target| target.captures_drag && target.depth == depth && target.order == order)
            .and_then(|target| target.press_slot.as_ref().map(|slot| slot.key.clone()))
    }

    /// Resolves the full [`InteractionState`] `key` reports: the sampled
    /// hover/press/focus-visible/disabled flags the interaction state already
    /// carries, DRAGGED while this target owns the active pointer drag, and
    /// SELECTED when the outermost interactive control under a [`Selected`]
    /// scope reads `true`.
    fn interaction_state_flags(
        &mut self,
        env: &Environment,
        key: &InteractionKey,
        state: WidgetInteractionState,
    ) -> InteractionState {
        let mut flags = state.state;
        if self
            .active_pointer_drag_key()
            .is_some_and(|drag| drag == *key)
        {
            flags |= InteractionState::DRAGGED;
        }
        if let Some(selected) = env.get::<Selected>()
            && self.hit_test.interaction.claim_selected(selected, key)
            && self.read_signal(&selected.0)
        {
            flags |= InteractionState::SELECTED;
        }
        flags
    }

    /// The [`InteractionState`] a bound control resolved to — how draw sites
    /// read back the state the reporting bookkeeping already computed.
    pub(crate) fn reported_interaction_state(&self, key: &InteractionKey) -> InteractionState {
        self.hit_test.interaction.reported_state(key)
    }

    /// Whether `key`'s owner claims the nearest [`Selected`] scope and the
    /// scope reads `true` — the accessibility announcement counterpart of the
    /// SELECTED flag the interaction flags carry.
    #[cfg(feature = "accessibility")]
    pub(crate) fn control_selected(&mut self, env: &Environment, key: &InteractionKey) -> bool {
        let Some(selected) = env.get::<Selected>() else {
            return false;
        };
        self.hit_test.interaction.claim_selected(selected, key) && self.read_signal(&selected.0)
    }

    /// Writes `state` into every [`InteractionReport`] scope this control
    /// claims — the outermost interactive control inside each reporting view
    /// owns the write — and only when the value changed, so a report never
    /// signals no-op updates.
    fn claim_interaction_reports(&mut self, env: &Environment, state: InteractionState) {
        for index in 0.. {
            let Some(report) = env.get_nth::<InteractionReport>(index) else {
                break;
            };
            if !self.hit_test.interaction.claim_report(report) {
                continue;
            }
            if report.0.snapshot() != state {
                report.0.set(state);
            }
        }
    }
}

impl SemanticCore {
    /// Registers an active [`ModalInteraction`] scope an environment overlay
    /// introduces — the Escape target and keyboard-trap owner for the subtree
    /// it scopes. Called when the node carrying the metadata is emitted, on
    /// the rendered flush and the headless semantic walk alike, so the scope
    /// exists whether or not anything beneath it binds an interaction target.
    /// The modal record carries no geometry: materialization keeps the
    /// frontmost live entry on a paint-resolving chain — the same
    /// "topmost wins" the per-frame overwrite gave.
    pub(crate) fn register_modal_scope(&mut self, env: &Environment) {
        if let Some(modal) = env
            .get::<ModalInteraction>()
            .filter(|modal| modal.is_active())
        {
            self.register_retained(modal.clone(), kurbo::Rect::ZERO, |regs| {
                &mut regs.modal_scope
            });
        }
    }

    pub(crate) fn register_interactive_pointer_target<F>(
        &mut self,
        bounds: kurbo::Rect,
        press_slot: PressSlot,
        action: F,
    ) where
        F: 'static + FnMut(&mut Self, kurbo::Point, &Environment) -> bool,
    {
        self.register_interactive_pointer_target_with_keyboard(bounds, press_slot, true, action);
    }

    pub(crate) fn register_interactive_pointer_target_with_keyboard<F>(
        &mut self,
        bounds: kurbo::Rect,
        press_slot: PressSlot,
        keyboard_focusable: bool,
        action: F,
    ) where
        F: 'static + FnMut(&mut Self, kurbo::Point, &Environment) -> bool,
    {
        let interaction = self.hit_test.interaction.handles_for(&press_slot);
        let modal = press_slot.modal;
        let key_handlers = self.snapshot_key_handlers();
        self.register_retained(
            PointerTarget {
                owner: Weak::new(),
                bounds,
                captures_drag: false,
                depth: self.render_depth,
                order: 0,
                press_slot: Some(press_slot),
                owners: self.owner_stack.clone(),
                interaction,
                action: Rc::new(RefCell::new(action)),
                keyboard_step: None,
                keyboard_focusable,
                modal,
                key_handlers,
            },
            bounds,
            |regs| &mut regs.pointer_targets,
        );
    }

    pub(crate) fn register_interactive_pointer_drag_target<F, K>(
        &mut self,
        bounds: kurbo::Rect,
        press_slot: PressSlot,
        action: F,
        keyboard_step: K,
    ) where
        F: 'static + FnMut(&mut Self, kurbo::Point, &Environment) -> bool,
        K: 'static + FnMut(bool) -> bool,
    {
        let interaction = self.hit_test.interaction.handles_for(&press_slot);
        let modal = press_slot.modal;
        let key_handlers = self.snapshot_key_handlers();
        self.register_retained(
            PointerTarget {
                owner: Weak::new(),
                bounds,
                captures_drag: true,
                depth: self.render_depth,
                order: 0,
                press_slot: Some(press_slot),
                owners: self.owner_stack.clone(),
                interaction,
                action: Rc::new(RefCell::new(action)),
                keyboard_step: Some(Rc::new(RefCell::new(keyboard_step))),
                keyboard_focusable: true,
                modal,
                key_handlers,
            },
            bounds,
            |regs| &mut regs.pointer_targets,
        );
    }

    pub(crate) fn ensure_active_pointer_drag_target_is_live(&mut self) {
        let Some(active) = self.hit_test.active_pointer_drag_target.as_ref() else {
            return;
        };
        let alive = self
            .hit_test
            .pointer_targets
            .iter()
            .any(|target| Rc::ptr_eq(&target.action, active));
        if alive {
            return;
        }
        // A re-record mints a fresh action `Rc` for the same slot — the drag
        // survives while a target still claims the captured signature, exactly
        // the re-resolution `sync_active_pointer_drag_target_after_layout`
        // applies after layout. Only a slot that left the list (a retired
        // owner, a gate that took the target out) drops the drag here.
        if let Some((depth, order)) = self.hit_test.active_pointer_drag_signature
            && let Some(target) = self.hit_test.pointer_targets.iter().find(|target| {
                target.captures_drag && target.depth == depth && target.order == order
            })
        {
            self.hit_test.active_pointer_drag_target = Some(Rc::clone(&target.action));
            self.hit_test.active_pointer_drag_owner = target.owner.clone();
            return;
        }
        self.hit_test.active_pointer_drag_target = None;
        self.hit_test.active_pointer_drag_signature = None;
        self.hit_test.active_pointer_drag_owner = Weak::new();
        self.clear_scrollbar_drag();
    }

    /// Ends any in-flight scrollbar-thumb drag; the thumb draws widened while
    /// dragged, so ending one schedules a re-encode to restore it.
    fn clear_scrollbar_drag(&mut self) {
        if let Some(drag) = self.hit_test.active_scrollbar_drag.take() {
            Self::mark_owner(&drag.owner, Dirty::LAYOUT);
        }
    }

    pub(crate) fn register_cursor_target(&mut self, bounds: kurbo::Rect, style: CursorStyle) {
        self.register_cursor_target_style(bounds, style);
    }

    pub(crate) fn register_cursor_target_style(&mut self, bounds: kurbo::Rect, style: CursorStyle) {
        self.register_retained(
            CursorTarget {
                bounds,
                style,
                owner: Weak::new(),
            },
            bounds,
            |regs| &mut regs.cursor_targets,
        );
    }

    pub(crate) fn register_hover_target(
        &mut self,
        key: InteractionKey,
        bounds: kurbo::Rect,
        on_enter: Option<HoverAction>,
        on_move: Option<HoverMoveAction>,
        on_exit: Option<HoverAction>,
    ) {
        self.register_hover_target_with_handles(key, bounds, None, on_enter, on_move, on_exit);
    }

    #[expect(
        clippy::needless_pass_by_value,
        reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
    )]
    pub(crate) fn register_hover_target_with_handles(
        &mut self,
        key: InteractionKey,
        bounds: kurbo::Rect,
        handles: Option<Rc<InteractionLayerHandles>>,
        on_enter: Option<HoverAction>,
        on_move: Option<HoverMoveAction>,
        on_exit: Option<HoverAction>,
    ) {
        let owner = self.registration_owner();
        let (slot, _hovering) = self.hit_test.interaction.bind_hover(&key, &owner);
        self.register_retained(
            HoverTarget {
                owner: Weak::new(),
                bounds,
                slot,
                handles,
                on_enter,
                on_move,
                on_exit,
            },
            bounds,
            |regs| &mut regs.hover_targets,
        );
    }

    pub(crate) fn register_hover_enter_target<F>(
        &mut self,
        key: InteractionKey,
        bounds: kurbo::Rect,
        action: F,
    ) where
        F: 'static + FnMut(&Environment) -> bool,
    {
        self.register_hover_target(key, bounds, Some(Rc::new(RefCell::new(action))), None, None);
    }

    pub(crate) fn register_hover_exit_target<F>(
        &mut self,
        key: InteractionKey,
        bounds: kurbo::Rect,
        action: F,
    ) where
        F: 'static + FnMut(&Environment) -> bool,
    {
        self.register_hover_target(key, bounds, None, None, Some(Rc::new(RefCell::new(action))));
    }

    pub(crate) fn register_hover_move_target<F>(
        &mut self,
        key: InteractionKey,
        bounds: kurbo::Rect,
        action: F,
    ) where
        F: 'static + FnMut(kurbo::Point, &Environment) -> bool,
    {
        self.register_hover_target(key, bounds, None, Some(Rc::new(RefCell::new(action))), None);
    }

    pub(crate) fn register_scroll_target<F>(
        &mut self,
        bounds: kurbo::Rect,
        handle: crate::scroll::ScrollHandle,
        action: F,
    ) where
        F: 'static + FnMut(f32, f32, bool) -> bool,
    {
        // The registering node's cell — this call runs inside the scroll
        // node's record under its reader, so `reader_cell()` is the owner a
        // smoothed scroll tick marks.
        let scroll_owner = self.registration_owner();
        self.retained
            .bind_scroll_owner(handle.cache_key(), &scroll_owner);
        self.register_retained(
            ScrollTarget {
                bounds,
                action: Rc::new(RefCell::new(action)),
                handle,
                owner: Rc::downgrade(&scroll_owner),
                depth: self.render_depth,
                order: 0,
            },
            bounds,
            |regs| &mut regs.scroll_targets,
        );
    }

    /// Advances every scroll view's smoothed wheel scroll or programmatic
    /// run and reports whether this frame needs to run: a tick moved an
    /// offset, or a glide still has frames to go.
    ///
    /// The tick that lands a glide moves the offset onto its target and ends
    /// the glide in one step, so "still gliding" alone would leave the landed
    /// offset unrendered — the touch fling's tick reports the same pair.
    #[expect(
        clippy::needless_pass_by_ref_mut,
        reason = "the mutable borrow is required by the shared signature even though this implementation does not mutate it"
    )]
    pub(crate) fn tick_smooth_scrolls(&mut self, now: Instant) -> bool {
        let mut ticked = false;
        for target in &self.hit_test.scroll_targets {
            let before = target.handle.metrics();
            let gliding = target.handle.tick_smooth_scroll(now);
            let after = target.handle.metrics();
            let moved = before.offset_x.to_bits() != after.offset_x.to_bits()
                || before.offset_y.to_bits() != after.offset_y.to_bits();
            if moved && let Some(cell) = target.owner.upgrade() {
                cell.mark(crate::renderer::Dirty::PAINT);
            }
            ticked |= moved || gliding;
        }
        ticked
    }

    /// Whether any scroll view's smoothed wheel scroll is still gliding,
    /// without advancing it.
    pub(crate) fn has_gliding_smooth_scrolls(&self) -> bool {
        self.hit_test
            .scroll_targets
            .iter()
            .any(|target| target.handle.is_smooth_scrolling())
    }

    /// The host's touch-drag scroll parameters for this window, carried by
    /// value from `PlatformWindow::touch_scroll_config` each input
    /// dispatch — `None` where the platform supplies none.
    pub(crate) const fn set_touch_scroll_config(
        &mut self,
        config: Option<crate::platform::TouchScrollConfig>,
    ) {
        self.hit_test.touch_scroll_config = config;
    }

    /// Whether a touch press is still waiting out its touch delay.
    #[cfg(test)]
    pub(crate) const fn has_pending_pointer_press(&self) -> bool {
        self.hit_test.pending_pointer_press.is_some()
    }

    /// Whether a touch fling is still decelerating toward rest.
    pub(crate) const fn has_active_touch_fling(&self) -> bool {
        self.hit_test.touch_fling.is_some()
    }

    /// Advances a running touch fling to `now`; returns whether its write
    /// or its continuation needs another frame. The offset write marks
    /// the fling's `ScrollTarget` owner — looked up through the registry —
    /// not the root.
    pub(crate) fn tick_touch_scroll(&mut self, now: Instant) -> bool {
        let Some(fling) = self.hit_test.touch_fling.take() else {
            return false;
        };
        // A fling whose scroll view no registered target holds any more
        // ends with it.
        let Some(owner) = self.try_scroll_target_owner(fling.handle().cache_key()) else {
            return false;
        };
        let tick = fling.tick(now);
        if tick.changed {
            Self::mark_owner(&owner, Dirty::LAYOUT);
        }
        if tick.running {
            self.hit_test.touch_fling = Some(fling);
        }
        tick.changed || tick.running
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> kurbo::Rect {
        kurbo::Rect::new(x0, y0, x1, y1)
    }

    #[test]
    fn cursor_style_falls_back_to_arrow_outside_all_targets() {
        let state = HitTestState::default();
        assert_eq!(
            state.cursor_style_at(kurbo::Point::new(5.0, 5.0)),
            CursorStyle::Arrow
        );
    }

    #[test]
    fn cursor_style_prefers_the_last_registered_target() {
        let mut state = HitTestState::default();
        state.cursor_targets.push(CursorTarget {
            bounds: rect(0.0, 0.0, 100.0, 100.0),
            style: CursorStyle::PointingHand,
            owner: Weak::new(),
        });
        state.cursor_targets.push(CursorTarget {
            bounds: rect(25.0, 25.0, 75.0, 75.0),
            style: CursorStyle::IBeam,
            owner: Weak::new(),
        });

        // Inside both: the later (topmost-drawn) target wins.
        assert_eq!(
            state.cursor_style_at(kurbo::Point::new(50.0, 50.0)),
            CursorStyle::IBeam
        );
        // Inside only the first.
        assert_eq!(
            state.cursor_style_at(kurbo::Point::new(10.0, 10.0)),
            CursorStyle::PointingHand
        );
    }
}
