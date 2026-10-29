//! The `container` leaf: `Native<LazyContainer>` rendered through a
//! [`HostView`].
//!
//! Mirrors `WuiContainer`: an `AnyViews`-backed collection the Rust layout
//! engine places, with an id-keyed reconcile on membership changes so
//! unchanged children keep their views. When the layout is one of the two
//! stacks — `lazy_stack_axis` answers for it — the container virtualizes
//! along its axis: it measures a sample child, walks the id list to find
//! the viewport window plus overscan, mounts only that window, and re-lays
//! out as the enclosing scroll view's viewport moves.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};

use cocoa_ui::geometry::MeasureProposal;
use cocoa_ui::scroll::{
    ScrollObservation, enclosing_scroll_view, observe_scroll_viewport, scroll_viewport,
};
use cocoa_ui::{PlatformView, Rect, Retained, view};
use waterui::id::Id as RawId;
use waterui::layout::container::LazyContainer;
use waterui::layout::stack::{Axis, LazyStackAxis, lazy_stack_axis};
use waterui::reactive::Signal;
use waterui::reactive::watcher::{BoxWatcherGuard, Metadata};
use waterui::views::{AnyViewsSnapshot, ViewSnapshot, Views};
use waterui_backend_core::AnyView;
use waterui_core::Computed;
use waterui_core::layout::{
    HorizontalAlignment, Layout, LayoutDirection, ProposalSize, Size, StretchAxis, SubView,
    VerticalAlignment, ViewDimensions, measure_layout, with_memoized_children,
};
use waterui_core::views::AnyViews;

use crate::contract::{NativeLeaf, Renderer};
use crate::dispatch::Dispatcher;
use crate::proposal::{self, Placement};
use crate::transition::ghost::Ghost;
use crate::transition::live::{LiveChild, Removal};
use crate::transition::slots::{FrozenPlacement, GhostKey, Slot, SlotSequence};

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HitTest, HostView};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{HitTest, HostView};

/// A child's identity: the collection id `AnyViews` answers for an index.
type ItemId = RawId;

/// `resolveVisibleWindow`'s answer: the first index inside the range, one
/// past the last, and the offset the first starts at.
struct VisibleWindow {
    /// The first visible index.
    start: usize,
    /// One past the last visible index.
    end: usize,
    /// The main-axis offset `start` begins at.
    leading_offset: f64,
}

/// The `[start_offset, end_offset)` window as indices, by walking extents
/// from the collection's front.
fn resolve_visible_window(
    count: usize,
    start_offset: f64,
    end_offset: f64,
    extent_at: impl Fn(usize) -> f64,
) -> VisibleWindow {
    if count == 0 {
        return VisibleWindow {
            start: 0,
            end: 0,
            leading_offset: 0.0,
        };
    }
    let clamped_start = start_offset.max(0.0);
    let clamped_end = end_offset.max(clamped_start);
    let mut index = 0;
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
    VisibleWindow {
        start,
        end: index.min(count),
        leading_offset,
    }
}

/// A `MeasureProposal` (f64 axes) as a layout `ProposalSize` (f32 axes).
fn to_proposal(proposal: MeasureProposal) -> ProposalSize {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; kit geometry is f64"
    )]
    ProposalSize::new(
        proposal.width.map(|width| width as f32),
        proposal.height.map(|height| height as f32),
    )
}

/// `LazyStackConfig`: the axis the layout virtualizes along plus the
/// resolved spacing, or `None` when the layout is not a stack.
#[derive(Debug, Clone)]
struct LazyConfig {
    /// Which direction items run and how they align across it.
    axis: LazyStackAxis,
    /// The resolved item gap.
    spacing: f64,
}

impl LazyConfig {
    /// `LazyStackConfig.init` — `None` for every layout that is not a stack.
    fn new(layout: &dyn Layout) -> Option<Self> {
        lazy_stack_axis(layout).map(|axis| {
            let spacing = match &axis {
                LazyStackAxis::Vertical { spacing, .. }
                | LazyStackAxis::Horizontal { spacing, .. } => f64::from(spacing.snapshot()),
            };
            Self { axis, spacing }
        })
    }

    /// The axis items run along.
    const fn main_axis(&self) -> Axis {
        match self.axis {
            LazyStackAxis::Vertical { .. } => Axis::Vertical,
            LazyStackAxis::Horizontal { .. } => Axis::Horizontal,
        }
    }
}

/// `mainAxisExtent` / `crossAxisExtent`: a measured size read on the
/// stack's axes.
fn main_cross(size: cocoa_ui::Size, lazy: &LazyConfig) -> (f64, f64) {
    match lazy.main_axis() {
        Axis::Vertical => (size.height, size.width),
        Axis::Horizontal => (size.width, size.height),
        _ => unreachable!("lazy stack axis is vertical or horizontal"),
    }
}

/// `lazyChildProposal`: the cross constraint on the cross axis, the main
/// axis left unspecified — the same value `measureLazyChild` negotiates
/// with, delivered to the child's own layout pass at placement.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the layout contract is f32; kit geometry is f64"
)]
fn lazy_child_proposal(cross: f64, lazy: &LazyConfig) -> ProposalSize {
    match lazy.main_axis() {
        Axis::Vertical => ProposalSize::new((cross > 0.0).then_some(cross as f32), None),
        Axis::Horizontal => ProposalSize::new(None, (cross > 0.0).then_some(cross as f32)),
        _ => unreachable!("lazy stack axis is vertical or horizontal"),
    }
}

/// The leaf's live state.
struct ContainerState {
    /// The `LazyContainer`'s layout object.
    layout: Box<dyn Layout>,
    /// The lazily materialized collection — watches register here and
    /// `snapshot()` captures state; membership and row answers never
    /// read it live.
    contents: AnyViews<AnyView>,
    /// The retained child-data snapshot aligned with `item_ids` — every
    /// `get_id`/`get_view` answer comes from it, never the live
    /// collection.
    snapshot: AnyViewsSnapshot<AnyView>,
    /// The render capability `get_view` results are realized through.
    renderer: Renderer,
    /// The host view — the leaf's platform object.
    host: Retained<HostView>,
    /// `lazy_stack_axis`'s answer plus its resolved spacing.
    lazy: Option<LazyConfig>,
    /// The computed layout direction, for the lazy path's RTL mirror.
    direction: Computed<LayoutDirection>,
    /// The collection's current ids, in order.
    item_ids: Vec<ItemId>,
    /// The materialized children by id — every mounted child lives here.
    rendered: HashMap<ItemId, LiveChild>,
    /// The member order layout sees: live ids and held removal ghosts in
    /// one sequence, each ghost anchored to the live id that precedes it
    /// and rebased by every reconcile (§4.1).
    slots: SlotSequence<ItemId>,
    /// Removal transitions in flight by ghost key — each retains the
    /// outgoing view and the slot its key holds in `slots` until its
    /// clock ends (layout-spec §4.5).
    ghosts: HashMap<GhostKey, Ghost>,
    /// Each live member's last placed measurement, by id — the ghost
    /// slot's source, recorded by the placement passes and never
    /// re-measured at removal (§4.2).
    placed: HashMap<ItemId, FrozenPlacement>,
    /// The non-lazy child order, as ids; empty while virtualizing.
    order: Vec<ItemId>,
    /// Lazy-path ids waiting for their first materialization as a
    /// structural join — a virtualized remount never replays insertion.
    pending_inserts: HashSet<ItemId>,
    /// `false` until the first membership applies: the initial population
    /// mounts quiet, later joins play their insertion.
    built: bool,
    /// Measured main-axis extents by id.
    measured_main: HashMap<ItemId, f64>,
    /// Measured cross-axis extents by id.
    measured_cross: HashMap<ItemId, f64>,
    /// The cross-axis constraint the lazy measurements were taken under.
    last_cross: f64,
    /// How the parent layout placed this container: the proposal it
    /// selected and the offset its pixel alignment left.
    placement: Cell<Placement>,
    /// The live scroll-viewport observation.
    scroll: Option<ScrollObservation>,
    /// The guards `layout.watch_invalidation` returned — held, not read.
    layout_guards: Vec<BoxWatcherGuard>,
}

impl core::fmt::Debug for ContainerState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ContainerState").finish_non_exhaustive()
    }
}

/// One merged member of the container — a live child or a held ghost slot
/// (§4.5: a removal transition's slot stays a layout member, spacing
/// included, until the transition completes).
#[derive(Clone, Copy)]
enum Member {
    /// A live child: its collection id and index in the ids list — the
    /// lazy path's `getView(at:)` index.
    Id { id: ItemId, index: usize },
    /// A removal ghost: its key in `state.ghosts` and `state.slots`.
    Ghost(GhostKey),
}

/// The ordered membership layout sees — the slot sequence's live ids and
/// ghosts, each live id paired with its index in `item_ids` (the lazy
/// path's `getView(at:)` index).
fn members(state: &ContainerState) -> Vec<Member> {
    let mut live_index = 0;
    state
        .slots
        .iter()
        .map(|slot| match slot {
            Slot::Live(id) => {
                let member = Member::Id {
                    id,
                    index: live_index,
                };
                live_index += 1;
                member
            }
            Slot::Ghost(key) => Member::Ghost(key),
        })
        .collect()
}

/// `subViewCache` without the per-call memoization `with_memoized_children`
/// already performs: the merged members' layout faces — live children by
/// their mounted face, ghosts by their frozen `GhostSlot`.
fn children(state: &ContainerState) -> Vec<&dyn SubView> {
    members(state)
        .iter()
        .map(|member| match member {
            Member::Id { id, .. } => state.rendered[id].layout(),
            Member::Ghost(key) => state.ghosts[key]
                .slot()
                .expect("a container ghost holds a slot")
                as &dyn SubView,
        })
        .collect()
}

/// Renders the item at `index` — `getView(at:)` — without mounting it.
fn render_at(state: &ContainerState, index: usize) -> NativeLeaf {
    let item = state
        .snapshot
        .get_view(index)
        .expect("lazy container index is in bounds");
    state.renderer.render(item)
}

/// Renders and mounts the item at `index`, recorded under its id.
/// `insertion` is whether this mount is a structural join — a lazily
/// re-materialized child replays nothing.
fn materialize(state: &mut ContainerState, index: usize, id: ItemId, insertion: bool) {
    let leaf = render_at(state, index);
    let child = if insertion {
        LiveChild::mount_inserted(leaf, &state.host, None, state.renderer.context().env())
    } else {
        LiveChild::mount(leaf, &state.host, None)
    };
    view::set_translates_autoresizing(child.view(), true);
    state.rendered.insert(id, child);
}

/// §4.5's removal: the member leaves the semantic tree now while a
/// declared transition keeps the outgoing view as a non-interactive
/// ghost, holding the frozen `slot` the reconcile reserved under `key`
/// until its clock ends and `on_complete` releases both.
///
/// `state_rc` is the same state the mutable borrow came from — the
/// ghost's completion runs on a later display tick, where the borrow is
/// already released.
fn retire_member(
    state: &mut ContainerState,
    state_rc: &Rc<RefCell<ContainerState>>,
    key: GhostKey,
    child: LiveChild,
    slot: waterui_core::transition::GhostSlot,
) {
    let weak = Rc::downgrade(state_rc);
    let Some(ghost) = child.remove(Removal {
        env: state.renderer.context().env(),
        slot: Some(slot),
        on_complete: Box::new(move || {
            let Some(state_rc) = weak.upgrade() else {
                return;
            };
            let mut state = state_rc.borrow_mut();
            state.slots.remove_ghost(key);
            state.ghosts.remove(&key);
            if state.lazy.is_some() {
                invalidate_virtual_layout(&state);
            } else {
                state.host.set_needs_layout();
                crate::measure_memo::invalidate();
            }
        }),
    }) else {
        // The transition was already complete: the reserved slot goes
        // back, so siblings never see the gap.
        state.slots.remove_ghost(key);
        return;
    };
    // The ghost paints at its slot beneath live children (§4.5).
    view::add_subview_at_bottom(&state.host, ghost.host());
    state.ghosts.insert(key, ghost);
}

/// Applies a reconcile's departures: a member that asked for no ghost —
/// unclaimed, or never placed — detaches and releases at once (§4.2); a
/// ghosted member retires onto the reserved slot.
fn depart_members(
    state: &mut ContainerState,
    state_rc: &Rc<RefCell<ContainerState>>,
    departed: Vec<(ItemId, Option<GhostKey>)>,
) {
    for (id, key) in departed {
        state.pending_inserts.remove(&id);
        let placement = state.placed.remove(&id);
        let Some(child) = state.rendered.remove(&id) else {
            continue;
        };
        let Some(key) = key else {
            drop(child);
            continue;
        };
        // `wants_ghost` required a recorded placement for `key` to exist.
        let slot = placement.expect("a ghosted member was placed").ghost_slot();
        retire_member(state, state_rc, key, child, slot);
    }
}

/// A ghost slot's frozen size read on the lazy stack's axes.
fn ghost_main_cross(slot: &waterui_core::transition::GhostSlot, lazy: &LazyConfig) -> (f64, f64) {
    let size = slot.size();
    main_cross(
        cocoa_ui::Size::new(f64::from(size.width), f64::from(size.height)),
        lazy,
    )
}

/// A lazy item's cross extent and whether it stretches the cross axis, under
/// the per-child protocol of the layout specification (§4.1, §5): a cross
/// stretcher takes `max(measured, cross)`, an unbounded measurement is a
/// stretcher that takes `cross`, and any other item keeps its own measured
/// answer unclamped, overflowing the stack as it would a fixed one. A zero
/// `cross` is an unspecified offer: the measured answer stands.
const fn lazy_cross_extent(measured: f64, flagged: bool, cross: f64) -> (f64, bool) {
    if cross <= 0.0 {
        (measured, flagged)
    } else if measured.is_infinite() {
        (cross, true)
    } else if flagged {
        (measured.max(cross), true)
    } else {
        (measured, false)
    }
}

/// Cross-axis origin of a lazy item before direction mirroring: a stretcher
/// is pinned to the min edge, every other item sits at `fraction` of the
/// free space (`0` leading/top, `0.5` center, `1` trailing/bottom).
fn lazy_cross_origin(stretches: bool, fraction: f64, bounds: f64, extent: f64) -> f64 {
    if stretches {
        0.0
    } else {
        (bounds - extent) * fraction
    }
}

/// The x origin of a vertical lazy stack's item, mirrored in right-to-left.
fn lazy_vertical_item_x(
    alignment: HorizontalAlignment,
    stretches: bool,
    rtl: bool,
    bounds_width: f64,
    width: f64,
) -> f64 {
    let fraction = if alignment == HorizontalAlignment::Leading {
        0.0
    } else if alignment == HorizontalAlignment::Trailing {
        1.0
    } else {
        0.5
    };
    let x = lazy_cross_origin(stretches, fraction, bounds_width, width);
    if rtl { bounds_width - x - width } else { x }
}

/// The y origin of a horizontal lazy stack's item.
fn lazy_horizontal_item_y(
    alignment: VerticalAlignment,
    stretches: bool,
    bounds_height: f64,
    height: f64,
) -> f64 {
    let fraction = if alignment == VerticalAlignment::Top {
        0.0
    } else if alignment == VerticalAlignment::Bottom {
        1.0
    } else {
        0.5
    };
    lazy_cross_origin(stretches, fraction, bounds_height, height)
}

/// `measureLazyChild`: the child's size for the virtualized proposal, and
/// whether it stretches the cross axis. The main axis keeps the ideal extent
/// the child reports under an unspecified main proposal — a main-axis
/// stretcher included — and the cross extent is resolved by
/// [`lazy_cross_extent`].
fn measure_lazy_child(
    child: &dyn SubView,
    cross: f64,
    lazy: &LazyConfig,
) -> (cocoa_ui::Size, bool) {
    let (_, size, stretches) = measure_lazy_child_dimensions(child, cross, lazy);
    (size, stretches)
}

/// [`measure_lazy_child`] plus the raw measurement the placement pass
/// negotiated — the frozen placement a departed member's ghost slot is
/// built from (§4.2).
fn measure_lazy_child_dimensions(
    child: &dyn SubView,
    cross: f64,
    lazy: &LazyConfig,
) -> (ViewDimensions, cocoa_ui::Size, bool) {
    let axis = child.stretch_axis();
    let proposal = lazy_child_proposal(cross, lazy);
    let dimensions = child.measure(proposal);
    let intrinsic = dimensions.size;
    let (intrinsic_main, intrinsic_cross) = main_cross(
        cocoa_ui::Size::new(f64::from(intrinsic.width), f64::from(intrinsic.height)),
        lazy,
    );
    let claims_cross = axis == StretchAxis::CrossAxis
        || match lazy.main_axis() {
            Axis::Vertical => axis.stretches_horizontal(),
            Axis::Horizontal => axis.stretches_vertical(),
            _ => unreachable!("lazy stack axis is vertical or horizontal"),
        };
    let (final_cross, stretches) = lazy_cross_extent(intrinsic_cross, claims_cross, cross);
    let size = match lazy.main_axis() {
        Axis::Vertical => cocoa_ui::Size::new(final_cross, intrinsic_main),
        Axis::Horizontal => cocoa_ui::Size::new(intrinsic_main, final_cross),
        _ => unreachable!("lazy stack axis is vertical or horizontal"),
    };
    (dimensions, size, stretches)
}

/// `ensureSampleMeasurement`: measure the first item once to seed the
/// estimate.
fn ensure_sample(state: &mut ContainerState, cross: f64, lazy: &LazyConfig) {
    if !state.measured_main.is_empty() {
        return;
    }
    let Some(&first_id) = state.item_ids.first() else {
        return;
    };
    let size = state.rendered.get(&first_id).map_or_else(
        || {
            // A measured throwaway — the baseline's `getView(at: 0)` sample.
            let leaf = render_at(state, 0);
            measure_lazy_child(leaf.layout(), cross, lazy)
        },
        |mounted| measure_lazy_child(mounted.layout(), cross, lazy),
    );
    let (main, cross_axis) = main_cross(size.0, lazy);
    state.measured_main.insert(first_id, main);
    state.measured_cross.insert(first_id, cross_axis);
}

/// `estimatedMainAxisExtent`: the running mean of measured extents, seeded
/// by the sample.
#[expect(
    clippy::cast_precision_loss,
    reason = "item counts are view counts; they never approach 2^53"
)]
fn estimated_main_extent(state: &mut ContainerState, cross: f64, lazy: &LazyConfig) -> f64 {
    ensure_sample(state, cross, lazy);
    if state.measured_main.is_empty() {
        return 0.0;
    }
    state.measured_main.values().sum::<f64>() / state.measured_main.len() as f64
}

/// `measure(proposal)` / `sizeThatFits`: the leaf's measurement, lazy-aware.
/// The lazy path materializes a sample under the borrow, so the whole
/// call runs inside a children transaction.
fn measure(
    state: &Rc<RefCell<ContainerState>>,
    pending: &Rc<RefCell<PendingChildren>>,
    proposal: ProposalSize,
) -> ViewDimensions {
    with_children_tx(state, pending, || {
        let is_lazy = state.borrow().lazy.is_some();
        if is_lazy {
            let size = lazy_size_that_fits(&mut state.borrow_mut(), proposal);
            return ViewDimensions::new(size);
        }
        let state = state.borrow();
        measure_layout(&*state.layout, proposal, &children(&state))
    })
}

/// `lazyStackSizeThatFits`: measured extents where known, the estimate
/// where not, spacing between consecutive items.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the layout contract is f32; kit geometry is f64"
)]
fn lazy_size_that_fits(state: &mut ContainerState, proposal: ProposalSize) -> Size {
    let lazy = state.lazy.clone().expect("lazy path under a stack layout");
    let members = members(state);
    if members.is_empty() {
        return Size::new(0.0, 0.0);
    }
    let cross = match lazy.main_axis() {
        Axis::Vertical => proposal.width.map_or(0.0, f64::from),
        Axis::Horizontal => proposal.height.map_or(0.0, f64::from),
        _ => unreachable!("lazy stack axis is vertical or horizontal"),
    };
    let estimate = estimated_main_extent(state, cross, &lazy);
    let member_extent = |member: &Member| match member {
        Member::Id { id, .. } => state.measured_main.get(id).copied().unwrap_or(estimate),
        Member::Ghost(key) => {
            ghost_main_cross(
                state.ghosts[key]
                    .slot()
                    .expect("a container ghost holds a slot"),
                &lazy,
            )
            .0
        }
    };
    let total_main = members
        .iter()
        .enumerate()
        .fold(0.0_f64, |partial, (index, member)| {
            let spacing = if index + 1 < members.len() {
                lazy.spacing
            } else {
                0.0
            };
            partial + member_extent(member) + spacing
        });
    let ghost_cross = || {
        state
            .ghosts
            .values()
            .map(|ghost| {
                ghost_main_cross(ghost.slot().expect("a container ghost holds a slot"), &lazy).1
            })
            .reduce(f64::max)
    };
    match lazy.main_axis() {
        Axis::Vertical => {
            let width = proposal
                .width
                .map(f64::from)
                .or_else(|| {
                    state
                        .measured_cross
                        .values()
                        .copied()
                        .reduce(f64::max)
                        .map(|live| live.max(ghost_cross().unwrap_or(0.0)))
                        .or_else(ghost_cross)
                })
                .unwrap_or_else(|| view::bounds(&state.host).size.width);
            Size::new(width as f32, total_main as f32)
        }
        Axis::Horizontal => {
            let height = proposal
                .height
                .map(f64::from)
                .or_else(|| {
                    state
                        .measured_cross
                        .values()
                        .copied()
                        .reduce(f64::max)
                        .map(|live| live.max(ghost_cross().unwrap_or(0.0)))
                        .or_else(ghost_cross)
                })
                .unwrap_or_else(|| view::bounds(&state.host).size.height);
            Size::new(total_main as f32, height as f32)
        }
        _ => unreachable!("lazy stack axis is vertical or horizontal"),
    }
}

/// The host's subviews in paint order — every ghost host beneath the
/// live children (§4.5), the materialized children in item order. An
/// unmaterialized lazy member owns no view and takes no slot here.
fn subview_order(state: &ContainerState) -> Vec<Retained<PlatformView>> {
    state
        .slots
        .iter()
        .filter_map(|slot| match slot {
            Slot::Live(_) => None,
            Slot::Ghost(key) => Some(view::retain_base(state.ghosts[&key].host())),
        })
        .chain(state.item_ids.iter().filter_map(|id| {
            state
                .rendered
                .get(id)
                .map(|child| view::retain_base(child.view()))
        }))
        .collect()
}

/// `syncChildren`: the id-keyed reconcile — the slot sequence first
/// rebases every live id and ghost against the new order, departing
/// members retire through [`depart_members`] (a declared, placed
/// transition leaves a ghost holding its slot beneath the live
/// children, §4.5), and joins materialize through [`materialize`]. The
/// first population mounts quiet; later joins play their insertion.
fn sync_children(
    state: &mut ContainerState,
    state_rc: &Rc<RefCell<ContainerState>>,
    ids: Vec<ItemId>,
) {
    let mut seen = HashSet::with_capacity(ids.len());
    for &id in &ids {
        assert!(
            seen.insert(id),
            "duplicate child view id in container: {id:?}"
        );
    }
    let reconciled = state.slots.reconcile(&ids, |id| {
        state
            .rendered
            .get(&id)
            .is_some_and(|child| child.claim().is_some())
            && state.placed.contains_key(&id)
    });
    depart_members(state, state_rc, reconciled.departed);
    state.item_ids = ids;
    let inserted: HashSet<ItemId> = reconciled.inserted.into_iter().collect();
    let item_ids = state.item_ids.clone();
    for (index, id) in item_ids.into_iter().enumerate() {
        if !state.rendered.contains_key(&id) {
            materialize(state, index, id, state.built && inserted.contains(&id));
        }
    }
    state.built = true;
    view::reconcile_subviews(&state.host, &subview_order(state));
    state.order = state.item_ids.clone();
    crate::measure_memo::invalidate();
    state.host.set_needs_layout();
}

/// Coalesced `contents` notifications plus the children transaction
/// depth — outside `ContainerState`, so an emission landing while
/// materialization holds the state borrow never touches it. Each event
/// records its own snapshot together with the ids captured from it and
/// the metadata, keeping the pair's index space coherent; the newest
/// recorded triple applies at the outermost transaction's finish and an
/// older snapshot is never replayed after a newer one.
#[derive(Default)]
struct PendingChildren {
    /// Active transaction depth — every scope that may run view
    /// generation or mounting under the state borrow enters one, so an
    /// emission raised inside only records and the outermost finish
    /// delivers.
    depth: usize,
    /// The newest unapplied (snapshot, ids, metadata) triple.
    emission: Option<(AnyViewsSnapshot<AnyView>, Vec<ItemId>, Metadata)>,
}

impl PendingChildren {
    /// The newest emission is authoritative.
    fn record(
        &mut self,
        snapshot: AnyViewsSnapshot<AnyView>,
        ids: Vec<ItemId>,
        metadata: Metadata,
    ) {
        self.emission = Some((snapshot, ids, metadata));
    }

    /// Drains the pending emission; the apply owns delivery from here.
    const fn take(&mut self) -> Option<(AnyViewsSnapshot<AnyView>, Vec<ItemId>, Metadata)> {
        self.emission.take()
    }
}

/// Delivers recorded children changes until the newest has applied: each
/// snapshot and its ids swap in atomically under one borrow inside the
/// emission's animation. Runs as a transaction itself — a synchronous
/// emission raised by materialization only records, and this loop drains
/// it before the outermost finish. Every scope that can run view
/// generation or mounting under the state borrow is a transaction, so
/// the borrow is always free here; the `borrow_mut` asserts it.
fn deliver_children(state: &Rc<RefCell<ContainerState>>, pending: &Rc<RefCell<PendingChildren>>) {
    pending.borrow_mut().depth += 1;
    loop {
        // `take` inside the loop body, not a `while let` scrutinee — the
        // scrutinee would hold the `RefMut` across the apply, and a
        // reentrant `record` there would collide with it.
        let Some((snapshot, ids, metadata)) = pending.borrow_mut().take() else {
            break;
        };
        crate::animation::with_platform_animation(&metadata, {
            let state = Rc::clone(state);
            move || {
                let mut borrowed = state.borrow_mut();
                borrowed.snapshot = snapshot;
                if borrowed.lazy.is_some() {
                    update_virtual_ids(&mut borrowed, &state, ids);
                } else {
                    sync_children(&mut borrowed, &state, ids);
                }
            }
        });
    }
    pending.borrow_mut().depth -= 1;
}

/// Runs `body` as a children transaction: contents emissions raised
/// inside it only record into `pending`, and the outermost finish
/// delivers the newest recorded emission. Every scope that holds the
/// state borrow across `get_view`/render/mount goes through here.
fn with_children_tx<T>(
    state: &Rc<RefCell<ContainerState>>,
    pending: &Rc<RefCell<PendingChildren>>,
    body: impl FnOnce() -> T,
) -> T {
    pending.borrow_mut().depth += 1;
    let result = body();
    let outermost = {
        let mut pending = pending.borrow_mut();
        pending.depth -= 1;
        pending.depth == 0
    };
    if outermost {
        deliver_children(state, pending);
    }
    result
}

/// `updateVirtualIds`: the slot sequence rebases live ids and ghosts
/// against the new order, departing members retire through
/// [`depart_members`] — an un-materialized member never realized a leaf,
/// so nothing is claimed and its slot releases at once (§4.2) — joins
/// record their pending insertion, then measurements filter to the ids
/// that survived and the layout invalidates.
fn update_virtual_ids(
    state: &mut ContainerState,
    state_rc: &Rc<RefCell<ContainerState>>,
    ids: Vec<ItemId>,
) {
    let mut seen = HashSet::with_capacity(ids.len());
    for &id in &ids {
        assert!(
            seen.insert(id),
            "duplicate child view id in container: {id:?}"
        );
    }
    let reconciled = state.slots.reconcile(&ids, |id| {
        state
            .rendered
            .get(&id)
            .is_some_and(|child| child.claim().is_some())
            && state.placed.contains_key(&id)
    });
    depart_members(state, state_rc, reconciled.departed);
    state.item_ids = ids;
    // The initial population mounts quiet; later joins play their
    // insertion on first materialization — a virtualized remount never
    // does.
    if state.built {
        state.pending_inserts.extend(reconciled.inserted);
    }
    state.built = true;
    state.measured_main.retain(|id, _| seen.contains(id));
    state.measured_cross.retain(|id, _| seen.contains(id));
    crate::measure_memo::invalidate();
    invalidate_virtual_layout(state);
}

/// `invalidateVirtualLayout`: invalidate intrinsic size, mark layout, and
/// propagate up the hierarchy.
fn invalidate_virtual_layout(state: &ContainerState) {
    state.host.invalidateIntrinsicContentSize();
    state.host.set_needs_layout();
    view::invalidate_layout(&state.host);
    crate::measure_memo::invalidate();
}

/// `installScrollObservationIfNeeded` + `teardownScrollObservation`: while
/// the container is lazy and inside a scroll view, watch the viewport.
fn install_scroll_observation(state: &mut ContainerState) {
    if state.lazy.is_none() || state.scroll.is_some() {
        return;
    }
    let Some(scroll_view) = enclosing_scroll_view(&state.host) else {
        return;
    };
    // The observation lives inside the state; borrow the view rather
    // than retain it — the state owns the host, not the other way around,
    // and a dead view needs no layout pass.
    let host = objc2::rc::Weak::new(&*state.host);
    state.scroll = Some(observe_scroll_viewport(&scroll_view, move || {
        if let Some(host) = host.load() {
            host.set_needs_layout();
        }
    }));
}

/// Drop any live viewport observation.
fn teardown_scroll_observation(state: &mut ContainerState) {
    state.scroll = None;
}

/// `currentViewportBounds`: the scroll view's visible rect in this
/// container's coordinates, or the bounds when unscrolled.
fn current_viewport(state: &ContainerState) -> Rect {
    enclosing_scroll_view(&state.host).map_or_else(
        || view::bounds(&state.host),
        |scroll| scroll_viewport(&state.host, &scroll),
    )
}

/// The cursor-positioned frame one lazy member occupies: cross resolved
/// by the stack's alignment on the container's bounds (a cross stretcher
/// pinned to the min edge), main at `cursor`. Live children and held ghost
/// slots place identically (§4.5).
fn member_frame(
    axis: &LazyStackAxis,
    bounds: cocoa_ui::Size,
    cursor: f64,
    size: cocoa_ui::Size,
    stretches: bool,
    rtl: bool,
) -> Rect {
    match axis {
        LazyStackAxis::Vertical { alignment, .. } => {
            let x = lazy_vertical_item_x(*alignment, stretches, rtl, bounds.width, size.width);
            Rect::new(x, cursor, size.width, size.height)
        }
        LazyStackAxis::Horizontal { alignment, .. } => {
            let y = lazy_horizontal_item_y(*alignment, stretches, bounds.height, size.height);
            let x = if rtl {
                bounds.width - cursor - size.width
            } else {
                cursor
            };
            Rect::new(x, y, size.width, size.height)
        }
    }
}

/// The inputs one lazy layout pass shares across its member placements.
struct LazyPass<'a> {
    lazy: &'a LazyConfig,
    cross: f64,
    bounds: cocoa_ui::Size,
    child_proposal: ProposalSize,
    rtl: bool,
}

/// Mounts the live member `id` if needed, measures it and places it at
/// `cursor`, returning its main-axis extent and whether its recorded
/// measurement changed.
fn place_live_member(
    state: &mut ContainerState,
    pass: &LazyPass<'_>,
    id: ItemId,
    item_index: usize,
    cursor: f64,
) -> (f64, bool) {
    if !state.rendered.contains_key(&id) {
        let insertion = state.pending_inserts.remove(&id);
        materialize(state, item_index, id, insertion);
    }
    let mounted = &state.rendered[&id];

    let (mut measured, size, stretches) =
        measure_lazy_child_dimensions(mounted.layout(), pass.cross, pass.lazy);
    let (main, cross_axis) = main_cross(size, pass.lazy);
    // The lazy path places cursor-based frames directly, pixel-aligned.
    let frame = member_frame(
        &pass.lazy.axis,
        pass.bounds,
        cursor,
        size,
        stretches,
        pass.rtl,
    );
    // The virtualized child is natively hosted: its selected proposal
    // is the offer it was measured with, not the frame it lands in.
    proposal::place(
        mounted.view(),
        frame,
        Some(pass.child_proposal),
        state.placement.get().offset,
    );
    // What the member's ghost slot reports if it departs: this pass's
    // negotiated measurement, size replaced by the frame it landed in.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; kit geometry is f64"
    )]
    {
        measured.size = Size::new(frame.size.width as f32, frame.size.height as f32);
    }
    state.placed.insert(
        id,
        FrozenPlacement {
            dimensions: measured,
            priority: mounted.layout().priority(),
        },
    );

    let remeasured = state.measured_main.get(&id) != Some(&main)
        || state.measured_cross.get(&id) != Some(&cross_axis);
    if remeasured {
        state.measured_main.insert(id, main);
        state.measured_cross.insert(id, cross_axis);
        crate::measure_memo::invalidate();
    }
    (main, remeasured)
}

/// Places a lazy ghost member at `cursor`: the slot still places every
/// frame — its extent and spacing count like a member's (§4.5) — and the
/// ghost view tracks it, so the outgoing pixels follow layout. Answers
/// the main-axis extent the cursor advances by.
fn place_ghost_member(
    state: &ContainerState,
    pass: &LazyPass<'_>,
    key: GhostKey,
    cursor: f64,
) -> f64 {
    let ghost = &state.ghosts[&key];
    let (main, cross) = ghost_main_cross(
        ghost.slot().expect("a container ghost holds a slot"),
        pass.lazy,
    );
    let size = match pass.lazy.main_axis() {
        Axis::Vertical => cocoa_ui::Size::new(cross, main),
        Axis::Horizontal => cocoa_ui::Size::new(main, cross),
        _ => unreachable!("lazy stack axis is vertical or horizontal"),
    };
    let frame = member_frame(&pass.lazy.axis, pass.bounds, cursor, size, false, pass.rtl);
    ghost.place(frame, state.placement.get().offset);
    main
}

/// `performLazyStackLayout`: find the viewport window plus overscan, mount
/// only it, place children along the main axis.
fn perform_lazy_layout(state: &mut ContainerState) {
    let lazy = state.lazy.clone().expect("lazy path under a stack layout");
    if state.item_ids.is_empty() {
        // No live members — only the ghosts a removal left behind, in
        // slot order beneath them.
        view::reconcile_subviews(&state.host, &subview_order(state));
        return;
    }
    let bounds = view::bounds(&state.host);
    let cross = match lazy.main_axis() {
        Axis::Vertical => bounds.size.width,
        Axis::Horizontal => bounds.size.height,
        _ => unreachable!("lazy stack axis is vertical or horizontal"),
    };
    #[expect(
        clippy::float_cmp,
        reason = "the baseline compares the constraint for exact equality"
    )]
    if cross != state.last_cross {
        state.last_cross = cross;
        state.measured_main.clear();
        state.measured_cross.clear();
        state.rendered.clear();
        state.placed.clear();
        crate::measure_memo::invalidate();
    }

    let viewport = current_viewport(state);
    let estimate = estimated_main_extent(state, cross, &lazy);
    let overscan = estimate.max(1.0) * 2.0;
    let (viewport_start, viewport_end) = match lazy.main_axis() {
        Axis::Vertical => (viewport.origin.y, viewport.origin.y + viewport.size.height),
        Axis::Horizontal => (viewport.origin.x, viewport.origin.x + viewport.size.width),
        _ => unreachable!("lazy stack axis is vertical or horizontal"),
    };
    let members = members(state);
    let count = members.len();
    let window = resolve_visible_window(
        count,
        (viewport_start - overscan).max(0.0),
        viewport_end + overscan,
        |index| {
            let extent = match &members[index] {
                Member::Id { id, .. } => state.measured_main.get(id).copied().unwrap_or(estimate),
                Member::Ghost(key) => {
                    ghost_main_cross(
                        state.ghosts[key]
                            .slot()
                            .expect("a container ghost holds a slot"),
                        &lazy,
                    )
                    .0
                }
            };
            extent + if index + 1 < count { lazy.spacing } else { 0.0 }
        },
    );

    let mut active: HashSet<ItemId> = HashSet::new();
    let mut cursor = window.leading_offset;
    let mut needs_invalidation = false;
    let rtl = state.direction.snapshot().is_right_to_left();
    let pass = LazyPass {
        lazy: &lazy,
        cross,
        bounds: bounds.size,
        child_proposal: lazy_child_proposal(cross, &lazy),
        rtl,
    };

    for (index, member) in members
        .iter()
        .enumerate()
        .take(window.end)
        .skip(window.start)
    {
        match *member {
            Member::Ghost(key) => {
                let main = place_ghost_member(state, &pass, key, cursor);
                cursor += main + if index + 1 < count { lazy.spacing } else { 0.0 };
            }
            Member::Id {
                id,
                index: item_index,
            } => {
                active.insert(id);
                let (main, remeasured) = place_live_member(state, &pass, id, item_index, cursor);
                needs_invalidation |= remeasured;
                cursor += main + if index + 1 < count { lazy.spacing } else { 0.0 };
            }
        }
    }

    let dropped: Vec<ItemId> = state
        .rendered
        .keys()
        .copied()
        .filter(|id| !active.contains(id))
        .collect();
    for id in dropped {
        state.rendered.remove(&id);
        state.placed.remove(&id);
    }

    // `materialize` appends every child it mounts at the end of the
    // host's subviews; restore paint order to item order, as
    // `sync_children` does for the fixed path (#3509).
    view::reconcile_subviews(&state.host, &subview_order(state));

    if needs_invalidation {
        invalidate_virtual_layout(state);
    }
}

/// `performLayout`: the non-lazy path — measure for a size answer first,
/// then place, proposals before frames, frames pixel-aligned.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the layout contract is f32; kit geometry is f64"
)]
fn perform_fixed_layout(state: &mut ContainerState) {
    let members = members(state);
    if members.is_empty() {
        return;
    }
    let bounds = view::bounds(&state.host);
    // Placed by a Rust parent: the proposal it selected. Natively hosted:
    // the boundary offer for the rect this container fills. Measure and
    // place share it, as `measure_layout` does.
    let placed = state.placement.get();
    let proposal = placed.proposal.unwrap_or_else(|| {
        ProposalSize::new(
            Some(bounds.size.width as f32),
            Some(bounds.size.height as f32),
        )
    });
    let bounds_layout = waterui_core::layout::Rect::new(
        waterui_core::layout::Point::new(bounds.origin.x as f32, bounds.origin.y as f32),
        waterui_core::layout::Size::new(bounds.size.width as f32, bounds.size.height as f32),
    );
    let placements = {
        let children = children(state);
        // `containerSize` warms the same measurement pass `placements` reads
        // — `measure_layout`'s own call.
        let _ = with_memoized_children(&children, |memoized| {
            state.layout.size_that_fits(proposal, memoized)
        });
        with_memoized_children(&children, |memoized| {
            state.layout.place(bounds_layout, proposal, memoized)
        })
    };
    assert_eq!(
        placements.len(),
        members.len(),
        "container layout returned {} placements for {} members",
        placements.len(),
        members.len()
    );
    for (index, (member, placement)) in members.iter().zip(placements.iter()).enumerate() {
        let frame = Rect::new(
            f64::from(placement.frame.x()),
            f64::from(placement.frame.y()),
            f64::from(placement.frame.width()),
            f64::from(placement.frame.height()),
        );
        assert!(
            frame.is_valid_for_layout(),
            "container received an invalid layout rect for member {index}: {frame:?}"
        );
        match member {
            Member::Id { id, .. } => {
                let child = &state.rendered[id];
                proposal::place(child.view(), frame, Some(placement.proposal), placed.offset);
                // What the member's ghost slot reports if it departs: this
                // pass's negotiated measurement, size replaced by the
                // frame it landed in.
                let mut dimensions = child.layout().measure(placement.proposal);
                dimensions.size = Size::new(frame.size.width as f32, frame.size.height as f32);
                state.placed.insert(
                    *id,
                    FrozenPlacement {
                        dimensions,
                        priority: child.layout().priority(),
                    },
                );
            }
            Member::Ghost(key) => {
                // A ghost paints at its slot — the same placement, so the
                // outgoing pixels track the layout (§4.5).
                state.ghosts[key].place(frame, placed.offset);
            }
        }
    }
}

/// The layout pass: lazy when the stack virtualizes, the fixed path
/// otherwise, plus the lazy path's viewport watch. The lazy path
/// materializes window children under the borrow, so it runs inside a
/// children transaction.
fn perform_layout(state: &Rc<RefCell<ContainerState>>, pending: &Rc<RefCell<PendingChildren>>) {
    install_scroll_observation_for(state);
    with_children_tx(state, pending, || {
        let mut state = state.borrow_mut();
        if state.lazy.is_some() {
            perform_lazy_layout(&mut state);
        } else {
            perform_fixed_layout(&mut state);
        }
    });
}

/// Install or refresh the viewport observation from a handler that only
/// borrows the state — the split keeps `Rc` cycles out of the watch.
fn install_scroll_observation_for(state: &Rc<RefCell<ContainerState>>) {
    let mut borrowed = state.borrow_mut();
    install_scroll_observation(&mut borrowed);
}

/// The container's layout face.
struct ContainerSubView {
    /// The leaf's state.
    state: Rc<RefCell<ContainerState>>,
    /// The collection's pending delivery and transaction depth.
    pending: Rc<RefCell<PendingChildren>>,
}

impl core::fmt::Debug for ContainerSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ContainerSubView").finish_non_exhaustive()
    }
}

impl SubView for ContainerSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        measure(&self.state, &self.pending, proposal)
    }

    /// `LazyContainer::stretch_axis` answers `layout.stretch_axis(&[])` — it
    /// cannot enumerate children without materializing the collection.
    fn stretch_axis(&self) -> StretchAxis {
        self.state.borrow().layout.stretch_axis(&[])
    }

    fn priority(&self) -> i32 {
        0
    }

    /// `rendersNothing`: non-empty and every child empty — the lazy path's
    /// `childViews` is empty, so a virtualized container answers false.
    /// A held ghost keeps the container rendering — its slot is still a
    /// member (§4.5).
    fn is_empty(&self) -> bool {
        let state = self.state.borrow();
        if !state.ghosts.is_empty() {
            return false;
        }
        !state.order.is_empty()
            && state
                .order
                .iter()
                .all(|id| state.rendered[id].layout().is_empty())
    }
}

/// Installs the `container` handler on the dispatcher.
#[expect(
    clippy::too_many_lines,
    reason = "the handler mirrors the baseline's single render entry"
)]
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<LazyContainer>(|container, ctx| {
        let mtm = ctx.mtm();
        let direction = container.direction();
        let (layout, contents) = container.into_inner();
        let host = HostView::new(mtm, Rect::ZERO);

        let lazy = LazyConfig::new(&*layout);
        let state = Rc::new(RefCell::new(ContainerState {
            layout,
            snapshot: contents.snapshot(),
            contents,
            renderer: ctx.renderer(),
            host: host.clone(),
            lazy,
            direction,
            item_ids: Vec::new(),
            rendered: HashMap::new(),
            slots: SlotSequence::new(Vec::new()),
            ghosts: HashMap::new(),
            placed: HashMap::new(),
            order: Vec::new(),
            pending_inserts: HashSet::new(),
            built: false,
            measured_main: HashMap::new(),
            measured_cross: HashMap::new(),
            last_cross: -1.0,
            placement: Cell::new(Placement::UNPLACED),
            scroll: None,
            layout_guards: Vec::new(),
        }));

        host.set_hit_test_handler(|_host, _point| HitTest::PassIfSelf);
        host.set_intrinsic_auto_layout(true);
        // The scroll-surface search descends through this container's
        // children, in stacking order.
        host.set_scroll_surface_handler({
            let state = Rc::clone(&state);
            move |_host| {
                let state = state.borrow();
                state
                    .order
                    .iter()
                    .map(|id| view::retain_base(state.rendered[id].view()))
                    .collect()
            }
        });
        let pending = Rc::new(RefCell::new(PendingChildren::default()));
        host.set_measure_handler({
            let state = Rc::clone(&state);
            let pending = Rc::clone(&pending);
            move |_host, proposal| {
                let measured = measure(&state, &pending, to_proposal(proposal));
                cocoa_ui::Size::new(
                    f64::from(measured.size.width),
                    f64::from(measured.size.height),
                )
            }
        });
        host.set_layout_handler({
            let state = Rc::clone(&state);
            let pending = Rc::clone(&pending);
            move |_host| {
                perform_layout(&state, &pending);
            }
        });
        // Moving superviews can change the enclosing scroll view.
        host.set_superview_handler({
            let state = Rc::clone(&state);
            move |_host| {
                let mut borrowed = state.borrow_mut();
                teardown_scroll_observation(&mut borrowed);
                install_scroll_observation(&mut borrowed);
            }
        });

        let sink_guard = proposal::register_sink(&host, {
            let state = Rc::clone(&state);
            let host = host.clone();
            move |placement| {
                let state = state.borrow();
                if state.placement.get() != placement {
                    state.placement.set(placement);
                    host.set_needs_layout();
                }
            }
        });

        // `watchAnyViewsIds` — the collection's membership watch. An
        // event only records into `pending` while a transaction is
        // active; otherwise it delivers immediately, so an emission
        // inside a child's own materialization can never collide with
        // the borrow that materialization holds. Subscribing delivers
        // nothing: the initial population below reads the snapshot.
        let watcher = state.borrow().contents.watch(.., {
            let state = Rc::clone(&state);
            let pending = Rc::clone(&pending);
            move |ctx, _change| {
                let snapshot = ctx.value().clone();
                let metadata = ctx.metadata().clone();
                let ids: Vec<ItemId> = snapshot
                    .range()
                    .filter_map(|index| snapshot.get_id(index))
                    .collect();
                let in_transaction = {
                    let mut pending = pending.borrow_mut();
                    pending.record(snapshot, ids, metadata);
                    pending.depth > 0
                };
                if !in_transaction {
                    deliver_children(&state, &pending);
                }
            }
        });

        // The layout's own invalidation signal — `wuiLayout.setOwner` +
        // `WuiLayoutInvalidationTarget.invalidate`: rebuild by re-laying
        // out, since stretch axes and priorities bake into the child set.
        let layout_guards = state.borrow().layout.watch_invalidation(Rc::new({
            // The guards are stored inside the very state a strong capture
            // would keep alive — a self-cycle with no outside participant
            // (WaterUI #1575). Both captures stay weak; a dead owner no-ops.
            let state = Rc::downgrade(&state);
            let host = objc2::rc::Weak::new(&*host);
            move || {
                let (Some(_state), Some(host)) = (state.upgrade(), host.load()) else {
                    return;
                };
                crate::invalidation::invalidate_layout_hierarchy(&host);
            }
        }));
        state.borrow_mut().layout_guards = layout_guards;

        // `reloadChildrenFromRust` — the initial population, a
        // transaction since materializing children may emit back.
        with_children_tx(&state, &pending, || {
            let ids: Vec<ItemId> = {
                let state_ref = state.borrow();
                state_ref
                    .snapshot
                    .range()
                    .filter_map(|index| state_ref.snapshot.get_id(index))
                    .collect()
            };
            let mut state_ref = state.borrow_mut();
            if state_ref.lazy.is_some() {
                update_virtual_ids(&mut state_ref, &state, ids);
            } else {
                sync_children(&mut state_ref, &state, ids);
            }
        });

        let mut leaf = NativeLeaf::new(
            &*host,
            ContainerSubView {
                state: Rc::clone(&state),
                pending: Rc::clone(&pending),
            },
        );
        leaf.keep(sink_guard);
        leaf.keep(watcher);
        leaf.keep(state);
        leaf
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_window_walks_extents_like_the_swift_loop() {
        // Five items of extent 10; viewport [15, 35) covers items 1..=3.
        let window = resolve_visible_window(5, 15.0, 35.0, |_| 10.0);
        assert_eq!(window.start, 1);
        assert_eq!(window.end, 4);
        assert!((window.leading_offset - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn visible_window_clamps_and_empties() {
        // Empty collection reports an empty window.
        let window = resolve_visible_window(0, 0.0, 100.0, |_| 10.0);
        assert_eq!(
            (window.start, window.end, window.leading_offset),
            (0, 0, 0.0)
        );

        // A negative start offset clamps to the collection's front.
        let window = resolve_visible_window(3, -50.0, 15.0, |_| 10.0);
        assert_eq!((window.start, window.end), (0, 2));
    }

    #[test]
    fn visible_window_past_the_end_is_empty() {
        let window = resolve_visible_window(2, 100.0, 200.0, |_| 10.0);
        assert_eq!((window.start, window.end), (2, 2));
    }

    #[test]
    fn lazy_cross_extent_follows_the_per_child_protocol() {
        // (measured, flagged stretcher, offered cross, expected extent, stretches)
        let cases = [
            // A non-stretcher wider than the offer overflows unclamped.
            (300.0, false, 200.0, 300.0, false),
            (50.0, false, 200.0, 50.0, false),
            // A cross stretcher narrower than the offer fills it.
            (50.0, true, 200.0, 200.0, true),
            // A cross stretcher wider than the offer keeps its measured extent.
            (300.0, true, 200.0, 300.0, true),
            // An unbounded measurement is a stretcher taking the offer.
            (f64::INFINITY, false, 200.0, 200.0, true),
            // No offer: the measured answer stands.
            (50.0, true, 0.0, 50.0, true),
        ];
        for (measured, flagged, cross, extent, stretches) in cases {
            let (got, got_stretches) = lazy_cross_extent(measured, flagged, cross);
            assert!(
                (got - extent).abs() < f64::EPSILON && got_stretches == stretches,
                "measured={measured} flagged={flagged} cross={cross}"
            );
        }
    }

    #[test]
    fn lazy_item_cross_origin_follows_alignment_direction_and_stretch() {
        let h = |a| move |s, rtl, w| lazy_vertical_item_x(a, s, rtl, 100.0, w);
        let lead = h(HorizontalAlignment::Leading);
        let center = h(HorizontalAlignment::Center);
        let trail = h(HorizontalAlignment::Trailing);
        // (origin, expected) in a 100-wide stack; stretches, rtl, width.
        let cases = [
            // Narrow non-stretchers, left-to-right.
            (lead(false, false, 20.0), 0.0),
            (center(false, false, 20.0), 40.0),
            (trail(false, false, 20.0), 80.0),
            // Narrow non-stretchers, right-to-left mirror.
            (lead(false, true, 20.0), 80.0),
            (center(false, true, 20.0), 40.0),
            (trail(false, true, 20.0), 0.0),
            // Overflowing non-stretchers are alignment-placed.
            (lead(false, false, 300.0), 0.0),
            (center(false, false, 300.0), -100.0),
            (trail(false, false, 300.0), -200.0),
            (lead(false, true, 300.0), -200.0),
            // Narrow stretchers fill the stack.
            (lead(true, false, 100.0), 0.0),
            (trail(true, false, 100.0), 0.0),
            (trail(true, true, 100.0), 0.0),
            // Overflowing stretchers pin to the min edge for every
            // alignment, mirrored in right-to-left.
            (lead(true, false, 300.0), 0.0),
            (center(true, false, 300.0), 0.0),
            (trail(true, false, 300.0), 0.0),
            (lead(true, true, 300.0), -200.0),
            (trail(true, true, 300.0), -200.0),
        ];
        for (index, (got, expected)) in cases.into_iter().enumerate() {
            assert!((got - expected).abs() < f64::EPSILON, "case {index}: {got}");
        }

        let v = |a, s, h| lazy_horizontal_item_y(a, s, 100.0, h);
        let cases = [
            (v(VerticalAlignment::Top, false, 20.0), 0.0),
            (v(VerticalAlignment::Center, false, 20.0), 40.0),
            (v(VerticalAlignment::Bottom, false, 20.0), 80.0),
            (v(VerticalAlignment::Bottom, false, 300.0), -200.0),
            (v(VerticalAlignment::Bottom, true, 300.0), 0.0),
            (v(VerticalAlignment::Center, true, 100.0), 0.0),
        ];
        for (index, (got, expected)) in cases.into_iter().enumerate() {
            assert!((got - expected).abs() < f64::EPSILON, "case {index}: {got}");
        }
    }

    /// A leaf answering a fixed size and stretch axis, recording the
    /// proposal `measure_lazy_child` negotiated with it.
    struct FixedChild {
        size: Size,
        axis: StretchAxis,
        last_proposal: Cell<Option<ProposalSize>>,
    }

    impl FixedChild {
        fn new(size: Size, axis: StretchAxis) -> Self {
            Self {
                size,
                axis,
                last_proposal: Cell::new(None),
            }
        }
    }

    impl SubView for FixedChild {
        fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
            self.last_proposal.set(Some(proposal));
            ViewDimensions::new(self.size)
        }

        fn stretch_axis(&self) -> StretchAxis {
            self.axis
        }

        fn priority(&self) -> i32 {
            0
        }
    }

    /// `measure_lazy_child` resolves every stretch axis against the
    /// stack's axes: a child claiming the cross — directly or through
    /// `CrossAxis`/`Both` — fills `cross`, a non-stretching child keeps its
    /// measured cross extent, and the main axis always keeps the
    /// ideal extent the child reports under an unspecified main proposal
    /// (water-rs/waterui#3448). Expressed in main/cross terms, the
    /// expectations hold for both stack orientations.
    #[test]
    fn lazy_child_measurement_resolves_stretch_axes() {
        for (lazy, proposal) in [
            (
                LazyConfig {
                    axis: LazyStackAxis::Vertical {
                        spacing: Computed::constant(0.0),
                        alignment: HorizontalAlignment::Center,
                    },
                    spacing: 0.0,
                },
                ProposalSize::new(Some(200.0), None),
            ),
            (
                LazyConfig {
                    axis: LazyStackAxis::Horizontal {
                        spacing: Computed::constant(0.0),
                        alignment: VerticalAlignment::Top,
                    },
                    spacing: 0.0,
                },
                ProposalSize::new(None, Some(200.0)),
            ),
        ] {
            // The absolute axis a plain cross stretcher declares, and the
            // one a main-axis-only stretcher declares, this orientation.
            let (cross_stretcher, main_stretcher) = match lazy.main_axis() {
                Axis::Vertical => (StretchAxis::Horizontal, StretchAxis::Vertical),
                Axis::Horizontal => (StretchAxis::Vertical, StretchAxis::Horizontal),
                _ => unreachable!("lazy stack axis is vertical or horizontal"),
            };
            // (axis, intrinsic (main, cross), expected (main, cross)).
            let cases = [
                // A non-stretching child keeps its measured cross, unclamped
                // when it overflows the offer (§4.1, water-rs/waterui#3474).
                (StretchAxis::None, (40.0, 30.0), (40.0, 30.0)),
                (StretchAxis::None, (40.0, 300.0), (40.0, 300.0)),
                // An unbounded cross answer fills the offered cross.
                (StretchAxis::None, (40.0, f32::INFINITY), (40.0, 200.0)),
                // Main-axis stretchers keep their ideal main extent and
                // claim no cross.
                (StretchAxis::MainAxis, (40.0, 30.0), (40.0, 30.0)),
                (main_stretcher, (40.0, 30.0), (40.0, 30.0)),
                // Cross claimants fill the cross and keep the ideal main.
                (cross_stretcher, (40.0, 30.0), (40.0, 200.0)),
                (StretchAxis::CrossAxis, (40.0, 30.0), (40.0, 200.0)),
                (StretchAxis::Both, (40.0, 30.0), (40.0, 200.0)),
            ];
            for (axis, (intrinsic_main, intrinsic_cross), (expected_main, expected_cross)) in cases
            {
                let (intrinsic, expected) = match lazy.main_axis() {
                    Axis::Vertical => (
                        Size::new(intrinsic_cross, intrinsic_main),
                        cocoa_ui::Size::new(expected_cross, expected_main),
                    ),
                    Axis::Horizontal => (
                        Size::new(intrinsic_main, intrinsic_cross),
                        cocoa_ui::Size::new(expected_main, expected_cross),
                    ),
                    _ => unreachable!("lazy stack axis is vertical or horizontal"),
                };
                let child = FixedChild::new(intrinsic, axis);
                assert_eq!(
                    measure_lazy_child(&child, 200.0, &lazy).0,
                    expected,
                    "{axis:?} under {lazy:?}"
                );
                assert_eq!(
                    child.last_proposal.get(),
                    Some(proposal),
                    "the probe leaves the main axis unspecified"
                );
            }
        }
    }
}
