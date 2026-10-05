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
use cocoa_ui::{Rect, Retained, view};
use waterui::animation::Animation;
use waterui::id::{Id as RawId, SelfId};
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

use crate::contract::{Mounted, NativeLeaf, Renderer};
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HitTest, HostView};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{HitTest, HostView};

/// A child's identity: the collection id `AnyViews` answers for an index.
type ItemId = SelfId<RawId>;

/// `withPlatformAnimation`: the watcher metadata's `Animation` mapped to a
/// kit timing — `Default` parses to the 0.25s bezier the FFI spells it as.
fn with_platform_animation(metadata: &Metadata, body: impl FnOnce() + 'static) {
    let timing = match metadata.try_get::<Animation>() {
        None => return body(),
        Some(Animation::Default) => cocoa_ui::core_animation::Timing::Bezier {
            duration: 0.25,
            control_points: [0.42, 0.0, 0.58, 1.0],
        },
        Some(Animation::Bezier {
            duration,
            x1,
            y1,
            x2,
            y2,
        }) => cocoa_ui::core_animation::Timing::Bezier {
            duration: duration.as_secs_f64(),
            control_points: [x1, y1, x2, y2],
        },
        Some(Animation::Spring { stiffness, damping }) => {
            cocoa_ui::core_animation::Timing::Spring {
                stiffness: f64::from(stiffness),
                damping: f64::from(damping),
            }
        }
    };
    cocoa_ui::core_animation::animate_with(timing, body);
}

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
    rendered: HashMap<ItemId, Mounted>,
    /// The non-lazy child order, as ids; empty while virtualizing.
    order: Vec<ItemId>,
    /// Measured main-axis extents by id.
    measured_main: HashMap<ItemId, f64>,
    /// Measured cross-axis extents by id.
    measured_cross: HashMap<ItemId, f64>,
    /// The cross-axis constraint the lazy measurements were taken under.
    last_cross: f64,
    /// The proposal the parent layout selected when it placed this
    /// container — `selectedProposal` in the Swift port.
    selected: Cell<Option<ProposalSize>>,
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

/// `subViewCache` without the per-call memoization `with_memoized_children`
/// already performs: the ordered children's layout faces.
fn children(state: &ContainerState) -> Vec<&dyn SubView> {
    state
        .order
        .iter()
        .map(|id| state.rendered[id].layout())
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
fn materialize(state: &mut ContainerState, index: usize, id: ItemId) {
    let mounted = render_at(state, index).mount(&state.host);
    view::set_translates_autoresizing(mounted.view(), true);
    state.rendered.insert(id, mounted);
}

/// `measureLazyChild`: the child's intrinsic size for the virtualized
/// proposal, cross-clamped when the child claims the axis or measures
/// unbounded.
fn measure_lazy_child(child: &dyn SubView, cross: f64, lazy: &LazyConfig) -> cocoa_ui::Size {
    let axis = child.stretch_axis();
    let proposal = lazy_child_proposal(cross, lazy);
    let intrinsic = child.measure(proposal).size;
    match lazy.main_axis() {
        Axis::Vertical => {
            assert!(
                axis != StretchAxis::Vertical
                    && axis != StretchAxis::Both
                    && axis != StretchAxis::MainAxis,
                "lazy vertical stack does not support children stretching on the main axis"
            );
            let intrinsic_w = f64::from(intrinsic.width);
            let final_width = if axis == StretchAxis::Horizontal
                || axis == StretchAxis::CrossAxis
                || intrinsic_w.is_infinite()
            {
                if cross > 0.0 { cross } else { intrinsic_w }
            } else if cross > 0.0 {
                intrinsic_w.min(cross)
            } else {
                intrinsic_w
            };
            cocoa_ui::Size::new(final_width, f64::from(intrinsic.height))
        }
        Axis::Horizontal => {
            assert!(
                axis != StretchAxis::Horizontal
                    && axis != StretchAxis::Both
                    && axis != StretchAxis::MainAxis,
                "lazy horizontal stack does not support children stretching on the main axis"
            );
            let intrinsic_h = f64::from(intrinsic.height);
            let final_height = if axis == StretchAxis::Vertical
                || axis == StretchAxis::CrossAxis
                || intrinsic_h.is_infinite()
            {
                if cross > 0.0 { cross } else { intrinsic_h }
            } else if cross > 0.0 {
                intrinsic_h.min(cross)
            } else {
                intrinsic_h
            };
            cocoa_ui::Size::new(f64::from(intrinsic.width), final_height)
        }
        _ => unreachable!("lazy stack axis is vertical or horizontal"),
    }
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
    let (main, cross_axis) = main_cross(size, lazy);
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
    if state.item_ids.is_empty() {
        return Size::new(0.0, 0.0);
    }
    let cross = match lazy.main_axis() {
        Axis::Vertical => proposal.width.map_or(0.0, f64::from),
        Axis::Horizontal => proposal.height.map_or(0.0, f64::from),
        _ => unreachable!("lazy stack axis is vertical or horizontal"),
    };
    let estimate = estimated_main_extent(state, cross, &lazy);
    let total_main = state
        .item_ids
        .iter()
        .enumerate()
        .fold(0.0_f64, |partial, (index, id)| {
            let spacing = if index + 1 < state.item_ids.len() {
                lazy.spacing
            } else {
                0.0
            };
            partial + state.measured_main.get(id).copied().unwrap_or(estimate) + spacing
        });
    match lazy.main_axis() {
        Axis::Vertical => {
            let width = proposal
                .width
                .map(f64::from)
                .or_else(|| state.measured_cross.values().copied().reduce(f64::max))
                .unwrap_or_else(|| view::bounds(&state.host).size.width);
            Size::new(width as f32, total_main as f32)
        }
        Axis::Horizontal => {
            let height = proposal
                .height
                .map(f64::from)
                .or_else(|| state.measured_cross.values().copied().reduce(f64::max))
                .unwrap_or_else(|| view::bounds(&state.host).size.height);
            Size::new(total_main as f32, height as f32)
        }
        _ => unreachable!("lazy stack axis is vertical or horizontal"),
    }
}

/// `syncChildren`: the id-keyed reconcile — reuse the mounted view of every
/// unchanged id, materialize only the joins, and make the host's subviews
/// exactly the ordered set.
fn sync_children(state: &mut ContainerState, ids: Vec<ItemId>) {
    let mut seen = HashSet::with_capacity(ids.len());
    let mut ordered_views = Vec::with_capacity(ids.len());
    let mut order = Vec::with_capacity(ids.len());
    for (index, &id) in ids.iter().enumerate() {
        assert!(
            seen.insert(id),
            "duplicate child view id in container: {id:?}"
        );
        if !state.rendered.contains_key(&id) {
            materialize(state, index, id);
        }
        let mounted = &state.rendered[&id];
        ordered_views.push(view::retain_base(mounted.view()));
        order.push(id);
    }
    let dropped: Vec<ItemId> = state
        .rendered
        .keys()
        .copied()
        .filter(|id| !seen.contains(id))
        .collect();
    for id in dropped {
        // `Mounted`'s drop detaches the view and releases the leaf.
        state.rendered.remove(&id);
    }
    view::reconcile_subviews(&state.host, &ordered_views);
    state.order = order;
    state.item_ids = ids;
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
        with_platform_animation(&metadata, {
            let state = Rc::clone(state);
            move || {
                let mut borrowed = state.borrow_mut();
                borrowed.snapshot = snapshot;
                if borrowed.lazy.is_some() {
                    update_virtual_ids(&mut borrowed, ids);
                } else {
                    sync_children(&mut borrowed, ids);
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

/// `updateVirtualIds`: filter measurements and rendered children to the ids
/// that survived, then invalidate.
fn update_virtual_ids(state: &mut ContainerState, ids: Vec<ItemId>) {
    let mut seen = HashSet::with_capacity(ids.len());
    for &id in &ids {
        assert!(
            seen.insert(id),
            "duplicate child view id in container: {id:?}"
        );
    }
    state.item_ids = ids;
    state.measured_main.retain(|id, _| seen.contains(id));
    state.measured_cross.retain(|id, _| seen.contains(id));
    crate::measure_memo::invalidate();
    let dropped: Vec<ItemId> = state
        .rendered
        .keys()
        .copied()
        .filter(|id| !seen.contains(id))
        .collect();
    for id in dropped {
        state.rendered.remove(&id);
    }
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

/// `performLazyStackLayout`: find the viewport window plus overscan, mount
/// only it, place children along the main axis.
#[expect(
    clippy::too_many_lines,
    reason = "the pass mirrors the baseline's single layout function"
)]
fn perform_lazy_layout(state: &mut ContainerState) {
    let lazy = state.lazy.clone().expect("lazy path under a stack layout");
    if state.item_ids.is_empty() {
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
    let count = state.item_ids.len();
    let window = resolve_visible_window(
        count,
        (viewport_start - overscan).max(0.0),
        viewport_end + overscan,
        |index| {
            let id = state.item_ids[index];
            let extent = state.measured_main.get(&id).copied().unwrap_or(estimate);
            extent + if index + 1 < count { lazy.spacing } else { 0.0 }
        },
    );

    let mut active: HashSet<ItemId> = HashSet::new();
    let mut cursor = window.leading_offset;
    let mut needs_invalidation = false;
    let child_proposal = lazy_child_proposal(cross, &lazy);
    let rtl = state.direction.snapshot().is_right_to_left();

    for index in window.start..window.end {
        let id = state.item_ids[index];
        if !state.rendered.contains_key(&id) {
            materialize(state, index, id);
        }
        active.insert(id);
        let mounted = &state.rendered[&id];

        // The virtualized child is natively hosted: its selected proposal
        // is the offer it was measured with, not the frame it lands in.
        proposal::deliver(mounted.view(), child_proposal);
        let size = measure_lazy_child(mounted.layout(), cross, &lazy);
        let (main, cross_axis) = main_cross(size, &lazy);
        if state.measured_main.get(&id) != Some(&main)
            || state.measured_cross.get(&id) != Some(&cross_axis)
        {
            state.measured_main.insert(id, main);
            state.measured_cross.insert(id, cross_axis);
            crate::measure_memo::invalidate();
            needs_invalidation = true;
        }

        let frame = match &lazy.axis {
            LazyStackAxis::Vertical { alignment, .. } => {
                let x = if *alignment == HorizontalAlignment::Leading {
                    0.0
                } else if *alignment == HorizontalAlignment::Trailing {
                    bounds.size.width - size.width
                } else {
                    (bounds.size.width - size.width) * 0.5
                };
                Rect::new(x, cursor, size.width, size.height)
            }
            LazyStackAxis::Horizontal { alignment, .. } => {
                let y = if *alignment == VerticalAlignment::Top {
                    0.0
                } else if *alignment == VerticalAlignment::Bottom {
                    bounds.size.height - size.height
                } else {
                    (bounds.size.height - size.height) * 0.5
                };
                let x = if rtl {
                    bounds.size.width - cursor - size.width
                } else {
                    cursor
                };
                Rect::new(x, y, size.width, size.height)
            }
        };
        // The lazy path sets cursor-based frames directly — unpixel-snapped,
        // as the baseline's `child.frame =` assignment does.
        view::set_frame(mounted.view(), frame);
        cursor += match lazy.main_axis() {
            Axis::Vertical => size.height,
            Axis::Horizontal => size.width,
            _ => unreachable!("lazy stack axis is vertical or horizontal"),
        } + if index + 1 < count { lazy.spacing } else { 0.0 };
    }

    let dropped: Vec<ItemId> = state
        .rendered
        .keys()
        .copied()
        .filter(|id| !active.contains(id))
        .collect();
    for id in dropped {
        state.rendered.remove(&id);
    }

    if needs_invalidation {
        invalidate_virtual_layout(state);
    }
}

/// `performLayout`: the non-lazy path — measure for a size answer first,
/// then place, proposals before frames, frames pixel-snapped.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the layout contract is f32; kit geometry is f64"
)]
fn perform_fixed_layout(state: &ContainerState) {
    if state.order.is_empty() {
        return;
    }
    let bounds = view::bounds(&state.host);
    // Placed by a Rust parent: the proposal it selected. Natively hosted:
    // the boundary offer for the rect this container fills. Measure and
    // place share it, as `measure_layout` does.
    let proposal = state.selected.get().unwrap_or_else(|| {
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
        state.order.len(),
        "container layout returned {} placements for {} children",
        placements.len(),
        state.order.len()
    );
    for (index, (id, placement)) in state.order.iter().zip(placements.iter()).enumerate() {
        let child = &state.rendered[id];
        let frame = Rect::new(
            f64::from(placement.frame.x()),
            f64::from(placement.frame.y()),
            f64::from(placement.frame.width()),
            f64::from(placement.frame.height()),
        );
        assert!(
            frame.is_valid_for_layout(),
            "container received an invalid layout rect for child {index}: {frame:?}"
        );
        // The negotiated proposal lands before the frame.
        proposal::deliver(child.view(), placement.proposal);
        view::set_frame(child.view(), frame);
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
            perform_fixed_layout(&state);
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
    fn is_empty(&self) -> bool {
        let state = self.state.borrow();
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
            order: Vec::new(),
            measured_main: HashMap::new(),
            measured_cross: HashMap::new(),
            last_cross: -1.0,
            selected: Cell::new(None),
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
            move |selected| {
                let state = state.borrow();
                if state.selected.get() != Some(selected) {
                    state.selected.set(Some(selected));
                    host.set_needs_layout();
                }
            }
        });

        // `watchAnyViewsIds` — the collection's membership watch. An
        // event only records into `pending` while a transaction is
        // active; otherwise it delivers immediately, so an emission
        // inside a child's own materialization can never collide with
        // the borrow that materialization holds. The subscription itself
        // runs inside a transaction — `watch` emits synchronously at
        // subscribe while `state.borrow()` is held, and recording lets
        // that first emission deliver once the borrow releases.
        let watcher = with_children_tx(&state, &pending, || {
            state.borrow().contents.watch(.., {
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
            })
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
                update_virtual_ids(&mut state_ref, ids);
            } else {
                sync_children(&mut state_ref, ids);
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
}
