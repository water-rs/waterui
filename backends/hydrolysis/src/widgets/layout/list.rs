use crate::renderer::HeldChild;
use crate::renderer::bounded_proposal;
use core::ops::Range;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::list_motion::RowMotion;
use crate::gesture::GestureTarget;
use crate::num_cast::f64_as_f32;
use crate::renderer::chrome::{ChromeContent, LiveChromeParts};
use crate::renderer::live_toggle::LiveToggleParts;
#[cfg(feature = "accessibility")]
use crate::renderer::{
    AccessibilityActionTarget, ScopedAccessibilitySemantics,
    accessibility_container_child_environment,
};
use crate::renderer::{
    HydroState, HydrolysisRenderer, ListSeparatorMode, ListStyleGeometry, ListSurfaceKind,
    RenderContext, ScopeOperands, VisibleSubviewCache, WidgetRenderContext,
    list_intrinsic_for_first_row, list_row_height_for_content, list_row_horizontal_insets,
    list_spec, list_style_geometry, materialize_list_item,
};
use crate::scroll::{ScrollHandle, ScrollRun, ScrollRunOutcome};
use crate::theme::{
    Chrome, DisclosureLook, DisclosureSpec, ListGestures, ListLook, ListMetrics, ListSpec,
    SpecTint, WidgetTheme,
};
#[cfg(feature = "accessibility")]
use accesskit::{
    Action as AccessibilityAction, Node as AccessibilityNode, NodeId as AccessibilityNodeId,
    Role as AccessibilityNodeRole,
};
#[cfg(feature = "accessibility")]
use waterui_accessibility::{AccessibilityHidden, AccessibilityStateSignal};
use waterui_backend_core::scroll::{ScrollPositionReport, animated_row_scroll_approach};
use waterui_controls::list::{
    ListConfig, ListRowDeletable, ListRowInsets, ListRowTags, ListScrollController, ListSection,
    ListSectionSpan, ListSelection, ListTagId,
};
use waterui_core::Environment;
use waterui_core::animation::Animation;
use waterui_core::contract_violation;
use waterui_core::extract::State;
use waterui_core::handler::{BoxedAction, boxed_action};
use waterui_core::id::Id as RawId;
use waterui_core::layout::{
    ProposalSize, Size as LayoutSize, UnitPoint, ViewDimensions, layout_direction,
};
use waterui_core::views::AnyViewsSnapshot;
use waterui_interaction::gesture::{DragEvent, DragGesture, Gesture, GesturePhase};
use waterui_interaction::interaction::{InteractionState, Selected};
use waterui_layout::padding::EdgeInsets;
use waterui_layout::scroll::Axis as ScrollAxis;
use waterui_text::Text;

use crate::platform::Modifiers;
use crate::renderer::lazy::VirtualExtentIndex;
use crate::widgets::draw_scroll_indicators;
use cherenkov_record::Draw as _;
use nami::watcher::BoxWatcherGuard;
use nami::{Binding, Computed, Signal, SignalExt as _};
use waterui_controls::theme::color;
use waterui_core::resolve::Resolvable as _;

/// The stable per-row id used to key the retained content sub-view cache, matching
/// the id `ListConfig::contents` yields per index.
pub type ListItemId = RawId;

/// A row as this backend holds it: its content a child the renderer builds.
pub type Row<E> = HeldChild<E>;

#[derive(Clone, Copy)]
struct ListViewportAnchor {
    id: ListItemId,
    index: usize,
    offset_within_row: f64,
    /// The offset the moment this anchor was recorded — or, once a
    /// membership change resolved it, its offset under the new membership.
    /// The next frame's glide/fling tick moves the offset before the
    /// membership shift applies, so the shift must translate from this
    /// recording — not the live offset — to keep the tick's motion.
    recorded_offset: f64,
}

/// A pending programmatic scroll to a row: the request's generation, row
/// index and animation, and how far applying it has progressed. The first
/// apply arms the animation; later applies only refine its target, so a
/// re-issue refines the destination instead of restarting the clock.
#[derive(Clone)]
struct PendingScroll {
    generation: i32,
    target: ListTagId,
    cached_index: Option<usize>,
    animation: Option<Animation>,
    state: PendingScrollState,
}

/// How far a [`PendingScroll`] has progressed against the scroll handle.
#[derive(Clone)]
enum PendingScrollState {
    /// No run has been armed for this request yet.
    Unarmed,
    /// The request armed `run`; [`ScrollHandle::scroll_run_outcome`] answers
    /// whether it still drives the offset, landed, or was interrupted —
    /// asking the handle instead of guessing from the offset is what keeps
    /// a membership re-anchor or a clamped landing from looking like the
    /// user scrolled.
    Armed { run: ScrollRun },
}

/// This frame's identity and geometry for one row, shared with that row's
/// retained gesture recognizers.
#[derive(Clone, Copy)]
struct RowBinding {
    /// The row's current position in the collection.
    index: usize,
    /// Row width, against which the swipe-dismiss threshold is measured.
    width: f64,
    /// Row height, the slot size a reorder drag steps by.
    height: f64,
    /// Inclusive start of the movable part span containing this row.
    movable_start: usize,
    /// Exclusive end of the movable part span containing this row.
    movable_end: usize,
}

impl RowBinding {
    /// Stand-in stored when a row's binding cell is created; every registration
    /// path overwrites it with real geometry in the same frame.
    const PLACEHOLDER: Self = Self {
        index: 0,
        width: 0.0,
        height: 0.0,
        movable_start: 0,
        movable_end: 0,
    };
}

/// The list's row-selection state, shared by every input path — pointer,
/// keyboard and accessibility — so all of them write the same erased
/// `ListSelection` binding under the same rules: a plain click selects, the
/// toggle modifier toggles the clicked row in multi mode, and Shift extends a
/// range from the anchor the last non-Shift write set (water-rs/waterui#1226).
///
/// The selection holds row *tags*, not row ids: a row is selected when its
/// tag is in the binding, a write stores the row's tag, and a row without a
/// tag is not selectable at all.
pub struct ListRowSelection<E: crate::text::TextEngine> {
    /// The erased selection `ListConfig` carries — keyed by the tags `tags`
    /// answers for the rows.
    selection: ListSelection<ListTagId>,
    /// The tag each row is selected by, keyed by the row ids the snapshot's
    /// `get_id` reports.
    tags: ListRowTags,
    /// The list's retained row snapshot — row ids by index, resolved for
    /// Shift-range writes against the membership the list actually rendered.
    snapshot: Rc<RefCell<AnyViewsSnapshot<Row<E>>>>,
    /// The row a Shift range extends from — the last row written without
    /// Shift, or the list's first row when nothing has been written yet.
    anchor: Cell<Option<ListItemId>>,
}

impl<E: crate::text::TextEngine> ListRowSelection<E> {
    /// Shares the config's selection when the list is selectable; `None` on
    /// `ListSelection::None`, so a non-selectable list keeps its old input
    /// behaviour — no row press target, no selected state.
    fn new(
        config: &ListConfig<HeldChild<E>>,
        snapshot: Rc<RefCell<AnyViewsSnapshot<Row<E>>>>,
    ) -> Option<Rc<Self>> {
        match config.selection() {
            ListSelection::None => None,
            selection => Some(Rc::new(Self {
                selection: selection.clone(),
                tags: config.row_tags().clone(),
                snapshot,
                anchor: Cell::new(None),
            })),
        }
    }

    /// Whether the row carries a tag, so input may select it.
    pub(crate) fn selects(&self, id: ListItemId) -> bool {
        self.tags.tag(id).is_some()
    }

    /// Whether the row is selected, as a signal the flush reads like any other
    /// row state — a binding write repaints the row's chrome. A row without a
    /// tag is never selected.
    pub(crate) fn is_selected(&self, id: ListItemId) -> Computed<bool> {
        let Some(tag) = self.tags.tag(id) else {
            return nami::constant(false).computed();
        };
        match &self.selection {
            ListSelection::None => nami::constant(false).computed(),
            ListSelection::Single(selection) => selection
                .clone()
                .map(move |current| current == Some(tag))
                .computed(),
            ListSelection::Multiple(selection) => selection
                .clone()
                .map(move |current| current.contains(&tag))
                .computed(),
        }
    }

    /// The index `id` currently occupies — Shift ranges are measured in
    /// indices, so a range write resolves the anchor's position the same way
    /// the row loop does.
    fn index_of(&self, id: ListItemId) -> Option<usize> {
        let snapshot = self.snapshot.borrow().clone();
        snapshot
            .range()
            .find(|index| snapshot.get_id(*index) == Some(id))
    }

    /// Writes a row interaction into the selection binding: plain selects the
    /// row (and anchors the next range), the toggle modifier toggles the row
    /// in multi mode (also anchoring), and Shift writes the whole range
    /// between the anchor and this row without moving the anchor; the range
    /// selects only the rows in it that carry a tag.
    ///
    /// Callers offer selection input only on rows
    /// [`ListRowSelection::selects`]; input reaching a row that carries no tag
    /// is a contract violation, reported, and writes nothing.
    pub(crate) fn write(&self, index: usize, id: ListItemId, modifiers: Modifiers) {
        let Some(tag) = self.tags.tag(id) else {
            contract_violation!(
                "selection input reached a list row that carries no tag; the selection is unchanged"
            );
            return;
        };
        match &self.selection {
            ListSelection::None => {}
            ListSelection::Single(selection) => {
                selection.set(Some(tag));
            }
            ListSelection::Multiple(selection) if modifiers.shift => {
                let anchor = self
                    .anchor
                    .get()
                    .and_then(|anchor| self.index_of(anchor))
                    .unwrap_or(0);
                let (start, end) = if anchor <= index {
                    (anchor, index)
                } else {
                    (index, anchor)
                };
                let snapshot = self.snapshot.borrow().clone();
                selection.set(
                    (start..=end)
                        .filter_map(|row| snapshot.get_id(row))
                        .filter_map(|row| self.tags.tag(row))
                        .collect(),
                );
            }
            ListSelection::Multiple(selection) => {
                if modifiers.control || modifiers.super_key {
                    selection.with_mut(|selected| {
                        if !selected.insert(tag) {
                            selected.remove(&tag);
                        }
                    });
                } else {
                    selection.set(std::collections::BTreeSet::from([tag]));
                }
                self.anchor.set(Some(id));
            }
        }
    }
}

/// Retained state for a list `Widget` node: the consumed config plus a per-widget
/// cache of the visible rows' content sub-views, keyed by stable row id. Only the
/// rows in the current visible window are built and retained (evicted once they
/// scroll out), so the list stays virtualized — cost is bounded by visible rows.
pub struct ListRenderState<E: crate::text::TextEngine> {
    pub(crate) config: ListConfig<HeldChild<E>>,
    /// Estimated/measured row extents belong to this list, not to its render
    /// position in a backend-global slot array. Shared with each row's
    /// `ListRow` accessibility target so a `ScrollIntoView` request resolves
    /// the row's span against the same measured extents the draw pass uses.
    extent_index: Rc<RefCell<VirtualExtentIndex>>,
    /// The scroll offset belongs to this semantic list node.
    scroll: RefCell<Option<ScrollHandle>>,
    /// Content sub-views for the rows currently in view, keyed by stable row id so a
    /// steady scroll reuses each visible row's node (keeping its reactive content
    /// live) and only builds rows entering the window.
    item_cache: RefCell<VisibleSubviewCache<ListItemId, E>>,
    /// What each row in `item_cache` was realized with, keyed by the same id
    /// and retired with it: the row modifiers (`ListRowDeletable`,
    /// `ListRowInsets`) and the environments the content was built under.
    realized_rows: RefCell<std::collections::HashMap<ListItemId, RealizedRow>>,
    /// One activation claim scope per row, keyed by the same stable id. The
    /// row's content sub-view flushes under the environment it was *built*
    /// under, so the scope each frame inserts into `subtree_env` must be the
    /// object that retained environment already carries — a fresh scope per
    /// frame would see a silenced tap's donation land in a cell nothing
    /// drains (water-rs/hydrolysis#27). Entries retire with the rows leaving
    /// the visible window.
    #[cfg(feature = "accessibility")]
    semantics_scopes:
        RefCell<std::collections::HashMap<ListItemId, ScopedAccessibilitySemantics<E>>>,
    /// A membership change invalidates index-based extents, including reorder
    /// operations whose collection length stays unchanged.
    rows_dirty: Rc<Cell<bool>>,
    /// Ids the collection watcher reported as replaced since the last
    /// `prepare_rows` consume — same-id items whose content may differ, so
    /// exactly those rows are dropped from `item_cache` and re-materialized
    /// while every other row keeps its retained node (focus, gestures,
    /// in-flight scroll anchoring all live inside it).
    replaced_row_ids: Rc<RefCell<std::collections::HashSet<ListItemId>>>,
    /// Last programmatic scroll generation applied to this semantic list.
    applied_scroll_generation: Cell<i32>,
    /// A requested row stays pending until its measured extent intersects the
    /// concrete viewport. This is required when estimated and measured row
    /// heights differ, especially for a jump to the final row.
    pending_scroll: RefCell<Option<PendingScroll>>,
    /// Stable top-row identity and intra-row offset from the previous frame.
    /// Membership changes use this anchor so delete/move operations do not
    /// visibly jump the viewport.
    viewport_anchor: Cell<Option<ListViewportAnchor>>,
    /// The anchor's target offset under the new membership plus the offset
    /// it was recorded at: `(rebased, recorded)` — the pure translation
    /// `bind_scroll` hands `rebind` is `rebased - recorded`, regardless of
    /// how far this frame's tick already moved the offset.
    pending_membership_offset: Cell<Option<(f64, f64)>>,
    /// A backend move action keeps the viewport's index fixed for the next
    /// membership reconcile so the moved row visibly changes position.
    preserve_anchor_index_once: Cell<bool>,
    /// Gesture recognizers for the visible rows, keyed by stable row id.
    ///
    /// The engine clears its target list every frame, so a recognizer that was
    /// only ever built by `register_target` would forget an in-flight drag on
    /// the very next frame. Retaining the target here and re-registering it at
    /// the row's fresh bounds keeps the state machine alive across frames,
    /// which is what makes a swipe or a reorder drag survive scrolling.
    row_gestures: RefCell<std::collections::HashMap<ListItemId, RowGestures>>,
    /// The look the theme gave the list and the spec it was requested for,
    /// held while the spec stays the same.
    look: RefCell<Option<(ListSpec, ListLook)>>,
    /// The disclosure look the theme provides for collapsible section chevrons.
    disclosure_look: RefCell<Option<(DisclosureSpec, DisclosureLook)>>,
    /// Retained animated disclosure indicators for collapsible section headers,
    /// keyed by the section's first row id so the indicator animates in place
    /// across expand and collapse.
    section_toggles: RefCell<std::collections::HashMap<ListItemId, Option<LiveToggleParts>>>,
    /// The sections holding rows in `rows_snapshot`, resolved once per
    /// membership change from the list's `ListSections` — row ids only, no
    /// row is materialized for it.
    sections: RefCell<Vec<ListSectionSpan>>,
    /// Display-position mapping after folding collapsed sections.
    visible_layout: RefCell<VisibleLayout>,
    /// Set whenever the watcher replaces `rows_snapshot`: the sections are
    /// resolved again from the new membership. Two sections can change in
    /// opposite directions between frames, so the row count alone cannot
    /// tell.
    sections_stale: Rc<Cell<bool>>,
    /// The list's row-selection state shared by pointer, keyboard and
    /// accessibility input; `None` when the list is not selectable.
    row_selection: Option<Rc<ListRowSelection<E>>>,
    /// The immutable row set the membership watcher last applied. Section
    /// resolution, anchor mapping, the virtual window, and row emission all
    /// read this one membership — never the live collection while an older
    /// event is still being reconciled.
    rows_snapshot: Rc<RefCell<AnyViewsSnapshot<Row<E>>>>,
    /// The rows' structural transitions (layout-spec.md §4.5): a row's
    /// declaration is claimed when its content is built, an inserted row
    /// animates in, and a removed row on screen stays as a ghost holding its
    /// slot. The extent index runs over display positions — rows and ghosts.
    transitions: crate::renderer::LazyTransitions<E, Row<E>>,
    /// The list widget's cell: the owner a transition's blur host marks.
    owner: std::rc::Weak<crate::renderer::NodeCell<E>>,
    /// §7.1's scroll-surface bookkeeping: the extension and clearance
    /// bounds layout computed once (the scroll handle is rebound with
    /// them inside [`Self::bind_scroll`]), plus the focused-field state
    /// the flush drives.
    pub(crate) surface: std::rc::Rc<crate::renderer::ScrollSurfaceArea>,
    /// Collection membership watcher.
    _guard: BoxWatcherGuard,
}

/// Retained gesture targets for one row, plus the live binding they read.
struct RowGestures {
    /// This frame's index and geometry for the row.
    binding: Rc<Cell<RowBinding>>,
    /// Horizontal swipe-to-dismiss over the whole row.
    swipe: Option<GestureTarget>,
    /// Vertical reorder drag on the row's move handle.
    reorder: Option<GestureTarget>,
    /// What the row moves live by: its displacement and its dismiss
    /// background.
    motion: Rc<RowMotion>,
    /// The realized chrome of the row's edit controls.
    controls: RowControlChrome,
}

/// One control's realized chrome and the interaction state it reports.
struct ControlChrome {
    parts: Option<LiveChromeParts>,
    report: nami::Binding<InteractionState>,
}

impl ControlChrome {
    fn new() -> Self {
        Self {
            parts: None,
            report: nami::Binding::new(InteractionState::empty()),
        }
    }
}

/// The realized chrome of a row's three edit controls.
struct RowControlChrome {
    move_up: ControlChrome,
    move_down: ControlChrome,
    delete: ControlChrome,
}

/// One contiguous run in the list's displayed order after folding collapsed sections.
#[derive(Clone, Debug, PartialEq, Eq)]
enum VisibleRun {
    /// Expanded semantic row indices `rows`, displayed starting at `display_start`.
    Rows {
        display_start: usize,
        rows: Range<usize>,
    },
    /// One collapsed section's header, occupying a single display slot.
    CollapsedHeader {
        display_start: usize,
        section_index: usize,
        first_row_index: usize,
    },
}

impl VisibleRun {
    const fn display_start(&self) -> usize {
        match self {
            Self::Rows { display_start, .. } | Self::CollapsedHeader { display_start, .. } => {
                *display_start
            }
        }
    }
}

/// Display-position mapping after folding collapsed sections.
///
/// When every section is expanded (`identity == true`), display positions map
/// 1:1 to semantic rows with zero per-row allocation and removal ghosts stay
/// active. When any section is collapsed, `runs` holds an `O(sections)` table
/// folding collapsed sections to their headers while preserving the full
/// underlying snapshot for tags and selection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct VisibleLayout {
    identity: bool,
    len: usize,
    runs: Vec<VisibleRun>,
}

/// What occupies one display position in the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VisibleEntry {
    Row(usize),
    CollapsedHeader {
        section_index: usize,
        first_row_index: usize,
    },
    Ghost(usize),
}

impl VisibleLayout {
    fn push_rows(runs: &mut Vec<VisibleRun>, display_cursor: &mut usize, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        let count = range.len();
        if let Some(VisibleRun::Rows { rows, .. }) = runs.last_mut()
            && rows.end == range.start
        {
            rows.end = range.end;
        } else {
            runs.push(VisibleRun::Rows {
                display_start: *display_cursor,
                rows: range,
            });
        }
        *display_cursor += count;
    }

    fn build(
        row_count: usize,
        sections: &[ListSectionSpan],
        mut read_expanded: impl FnMut(&Binding<bool>) -> bool,
    ) -> Self {
        let mut any_collapsed = false;
        let mut expanded_flags = Vec::with_capacity(sections.len());
        for span in sections {
            let expanded = span
                .section
                .as_ref()
                .and_then(ListSection::expanded)
                .is_none_or(&mut read_expanded);
            if !expanded {
                any_collapsed = true;
            }
            expanded_flags.push(expanded);
        }
        if !any_collapsed {
            return Self {
                identity: true,
                len: row_count,
                runs: Vec::new(),
            };
        }
        let mut runs: Vec<VisibleRun> = Vec::with_capacity(sections.len());
        let mut cursor = 0usize;
        let mut display_cursor = 0usize;
        for (section_index, (span, &expanded)) in
            sections.iter().zip(expanded_flags.iter()).enumerate()
        {
            if cursor < span.rows.start {
                Self::push_rows(&mut runs, &mut display_cursor, cursor..span.rows.start);
            }
            if expanded {
                Self::push_rows(&mut runs, &mut display_cursor, span.rows.clone());
            } else if !span.rows.is_empty()
                && span.section.as_ref().and_then(ListSection::label).is_some()
            {
                runs.push(VisibleRun::CollapsedHeader {
                    display_start: display_cursor,
                    section_index,
                    first_row_index: span.rows.start,
                });
                display_cursor += 1;
            }
            cursor = span.rows.end;
        }
        if cursor < row_count {
            Self::push_rows(&mut runs, &mut display_cursor, cursor..row_count);
        }
        Self {
            identity: false,
            len: display_cursor,
            runs,
        }
    }

    fn entry_at(&self, position: usize) -> Option<VisibleEntry> {
        let idx = self
            .runs
            .partition_point(|run| run.display_start() <= position)
            .checked_sub(1)?;
        match self.runs.get(idx)? {
            VisibleRun::Rows {
                display_start,
                rows,
            } => {
                let offset = position - *display_start;
                (offset < rows.len()).then_some(VisibleEntry::Row(rows.start + offset))
            }
            VisibleRun::CollapsedHeader {
                display_start,
                section_index,
                first_row_index,
            } => (position == *display_start).then_some(VisibleEntry::CollapsedHeader {
                section_index: *section_index,
                first_row_index: *first_row_index,
            }),
        }
    }

    fn display_of_row(&self, index: usize) -> Option<usize> {
        for run in &self.runs {
            if let VisibleRun::Rows {
                display_start,
                rows,
            } = run
                && rows.contains(&index)
            {
                return Some(*display_start + (index - rows.start));
            }
        }
        None
    }

    fn nearest_row_at(&self, position: usize) -> Option<usize> {
        match self.entry_at(position)? {
            VisibleEntry::Row(index) => Some(index),
            VisibleEntry::CollapsedHeader {
                first_row_index, ..
            } => Some(first_row_index),
            VisibleEntry::Ghost(_) => None,
        }
    }
}

/// Interaction slots allocated per row id:
/// 0 = move up, 1 = move down, 2 = delete, 3 = row selection press,
/// 4 = row accessibility anchor, 5 = collapsible section header toggle.
const INTERACTION_SLOTS_PER_ROW: usize = 6;
#[cfg(feature = "accessibility")]
const INTERACTION_SLOT_ROW_PRESS: usize = 3;
#[cfg(feature = "accessibility")]
const INTERACTION_SLOT_ROW_ANCHOR: usize = 4;
const INTERACTION_SLOT_SECTION_HEADER: usize = 5;

fn row_interaction_base(row_id: ListItemId) -> usize {
    (crate::num_cast::i32_as_u32(i32::from(row_id)) as usize)
        .checked_mul(INTERACTION_SLOTS_PER_ROW)
        .expect("hydrolysis List interaction identity overflow")
}

/// The section chrome one row is responsible for drawing.
///
/// The row that currently opens a section owns its header, and the row that
/// currently closes it owns its footer — whichever rows those are after the
/// last membership change. Resolving both onto rows up front keeps every
/// row's height a local question, even while only part of the list is
/// realized.
///
/// The header and footer are semantic text, not strings resolved once: an app
/// can drive a section title from a signal, and the retained chrome is only
/// rebuilt when membership changes. Holding the signal here is what lets a
/// title change repaint without the list being rebuilt.
#[derive(Clone, Default)]
struct RowSectionChrome {
    header: Option<Text>,
    footer: Option<Text>,
    header_expanded: Option<Binding<bool>>,
    is_section_first: bool,
    is_section_last: bool,
    starts_later_section: bool,
}

impl RowSectionChrome {
    const fn top_gap(&self, style_geom: ListStyleGeometry) -> f64 {
        if self.starts_later_section {
            style_geom.inter_section_gap
        } else {
            0.0
        }
    }

    const fn header_height(&self, metrics: &crate::theme::ListMetrics) -> f64 {
        if self.header.is_some() {
            metrics.section_header_height as f64
        } else {
            0.0
        }
    }

    const fn footer_height(&self, metrics: &crate::theme::ListMetrics) -> f64 {
        if self.footer.is_some() {
            metrics.section_footer_height as f64
        } else {
            0.0
        }
    }

    fn total_height(
        &self,
        metrics: &crate::theme::ListMetrics,
        style_geom: ListStyleGeometry,
    ) -> f64 {
        self.top_gap(style_geom) + self.header_height(metrics) + self.footer_height(metrics)
    }

    const fn card_radii(&self, radius: f64) -> kurbo::RoundedRectRadii {
        let top = if self.is_section_first { radius } else { 0.0 };
        let bottom = if self.is_section_last { radius } else { 0.0 };
        kurbo::RoundedRectRadii::new(top, top, bottom, bottom)
    }
}

/// What a list row was realized with: its row modifiers (`ListRowDeletable`,
/// `ListRowInsets`) and the environments its content was built under.
struct RealizedRow {
    deletable: waterui_core::Computed<bool>,
    insets: Option<waterui_layout::padding::EdgeInsets>,
    /// The row's environment with its content's accessibility naming hoisted
    /// onto it: the row node, its chrome and its controls resolve under it.
    env: Environment,
    /// Whether the row's content sits under `.disabled` — read from the
    /// environment the content's own presses resolve against, the same
    /// [`waterui_interaction::interaction::Disabled`] scope every control's input
    /// gate consults. A disabled row is inert the way a disabled `Button`
    /// cannot be pressed: the list registers no selection press for it and
    /// its `ListItem` node advertises no `Click` (water-rs/waterui#2244).
    disabled: waterui_core::Computed<bool>,
    /// The environment the content was built and lays out under.
    subtree_env: Environment,
}

/// Moves the accessibility scope a row's content opens with onto the row:
/// every naming, state and identity value `scope_env` holds that the
/// content installed itself — it is not the value `subtree_env`, the
/// environment the content was built under, already carried.
#[cfg(feature = "accessibility")]
fn adopt_content_naming<E: crate::text::TextEngine>(
    row_env: &mut Environment,
    scope_env: &Environment,
    subtree_env: &Environment,
) {
    use crate::renderer::accessibility::ScopedAccessibilityIdentifier;
    use waterui_accessibility::{
        AccessibilityAdjustable, AccessibilityChildren, AccessibilityCustomActions,
        AccessibilityGeometry, AccessibilityHint, AccessibilityLabel, AccessibilityRole,
        AccessibilityValue,
    };

    macro_rules! adopt {
        ($($ty:ty),+ $(,)?) => {
            $(
                if let Some(value) = scope_env.get::<$ty>()
                    && !subtree_env.get::<$ty>().is_some_and(|carried| std::ptr::eq(carried, value))
                {
                    row_env.insert(value.clone());
                }
            )+
        };
    }
    adopt!(
        AccessibilityLabel,
        AccessibilityValue,
        AccessibilityHint,
        AccessibilityCustomActions,
        AccessibilityAdjustable,
        AccessibilityRole,
        ScopedAccessibilityIdentifier,
        AccessibilityHidden,
        AccessibilityChildren,
        AccessibilityGeometry,
        AccessibilityStateSignal,
        ScopedAccessibilitySemantics<E>,
    );
}

/// A list row is its own chrome: a button inside one is a row, not a filled
/// container floating on a screen. Buttons that picked a style explicitly
/// keep it.
fn list_row_environment(env: &Environment) -> Environment {
    let mut row_env = env.clone();
    row_env.remove::<ListRowDeletable>();
    row_env.remove::<ListRowInsets>();
    row_env.insert(waterui_controls::button::ButtonStyle::Plain);
    row_env.insert(crate::widgets::controls::button::ListRowChrome);
    row_env
}

impl<E: crate::text::TextEngine> ListRenderState<E> {
    pub(crate) fn from_config(
        config: ListConfig<HeldChild<E>>,
        env: &Environment,
        renderer: &crate::renderer::SemanticCore<E>,
    ) -> Self {
        let rows_dirty = Rc::new(Cell::new(true));
        let rows_dirty_for_watch = Rc::clone(&rows_dirty);
        let sections_stale = Rc::new(Cell::new(true));
        let sections_stale_for_watch = Rc::clone(&sections_stale);
        let replaced_row_ids = Rc::new(RefCell::new(std::collections::HashSet::new()));
        let replaced_for_watch = Rc::clone(&replaced_row_ids);
        let rows_snapshot = Rc::new(RefCell::new(config.contents().snapshot()));
        let snapshot_for_watch = Rc::clone(&rows_snapshot);
        // Membership changes mark the owning widget's cell — the node whose
        // read built this state, or the window root when built unowned.
        let cell = Rc::downgrade(
            &renderer
                .reader_cell()
                .unwrap_or_else(|| Rc::clone(renderer.root_cell())),
        );
        let owner = std::rc::Weak::clone(&cell);
        // The membership flag the transitions consume is the rows' own dirty
        // flag: `prepare_rows` applies each change exactly once.
        let transitions = crate::renderer::LazyTransitions::new(
            env,
            rows_snapshot.borrow().clone(),
            Rc::clone(&rows_dirty),
        );

        let guard = config.contents().watch(.., move |ctx, change| {
            rows_dirty_for_watch.set(true);
            sections_stale_for_watch.set(true);
            // Each event hands over an immutable snapshot of the exact row set
            // it reported — retained so every reader below works the same
            // membership instead of the live collection.
            let event_snapshot = ctx.into_value();
            crate::renderer::collect_replaced_ids(
                &event_snapshot,
                change,
                &mut replaced_for_watch.borrow_mut(),
            );
            *snapshot_for_watch.borrow_mut() = event_snapshot;
            if let Some(cell) = cell.upgrade() {
                cell.mark(crate::renderer::Dirty::STRUCTURE);
            }
        });
        let row_selection = ListRowSelection::new(&config, Rc::clone(&rows_snapshot));
        Self {
            config,
            row_selection,
            extent_index: Rc::new(RefCell::new(VirtualExtentIndex::default())),
            scroll: RefCell::new(None),
            item_cache: RefCell::new(VisibleSubviewCache::new()),
            realized_rows: RefCell::new(std::collections::HashMap::new()),
            #[cfg(feature = "accessibility")]
            semantics_scopes: RefCell::new(std::collections::HashMap::new()),
            rows_dirty,
            replaced_row_ids,
            applied_scroll_generation: Cell::new(0),
            pending_scroll: RefCell::new(None),
            viewport_anchor: Cell::new(None),
            pending_membership_offset: Cell::new(None),
            preserve_anchor_index_once: Cell::new(false),
            row_gestures: RefCell::new(std::collections::HashMap::new()),
            look: RefCell::new(None),
            disclosure_look: RefCell::new(None),
            section_toggles: RefCell::new(std::collections::HashMap::new()),
            sections: RefCell::new(Vec::new()),
            visible_layout: RefCell::new(VisibleLayout::default()),
            sections_stale,
            rows_snapshot,
            transitions,
            owner,
            surface: std::rc::Rc::default(),
            _guard: guard,
        }
    }

    /// The list's look for its spec in `env`: the one already held when the
    /// spec is unchanged, otherwise requested again.
    fn look(&self, theme: &Rc<dyn crate::theme::WidgetTheme>, env: &Environment) -> ListLook {
        let spec = list_spec(env, self.config.style());
        let mut held = self.look.borrow_mut();
        match &*held {
            Some((held_spec, look)) if *held_spec == spec => look.clone(),
            _ => {
                let look = theme.list(&spec, env);
                *held = Some((spec, look.clone()));
                look
            }
        }
    }

    /// The theme's disclosure look for collapsible section header indicators.
    fn disclosure_look(
        &self,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
        env: &Environment,
    ) -> DisclosureLook {
        let spec = DisclosureSpec::new(layout_direction(env).snapshot(), SpecTint::resolve(env));
        let mut held = self.disclosure_look.borrow_mut();
        match &*held {
            Some((held_spec, look)) if *held_spec == spec => look.clone(),
            _ => {
                let look = theme.disclosure(&spec, env);
                *held = Some((spec, look.clone()));
                look
            }
        }
    }

    /// The resolved style geometry for this list.
    const fn style_geometry(&self) -> ListStyleGeometry {
        list_style_geometry(self.config.style())
    }

    /// Effective minimum row height combining explicit `ListMinRowHeight` with
    /// the list style's default floor.
    fn effective_min_row_height(&self) -> Option<f32> {
        self.config
            .min_row_height()
            .or_else(|| self.style_geometry().default_min_row_height)
    }

    /// How a row's drags are recognized and settle, as the held look says.
    /// The look is held by the flush that registered the drag.
    fn gestures(&self) -> ListGestures {
        self.look
            .borrow()
            .as_ref()
            .expect(
                "a row's drag runs after the list flush that registered it, which holds the look",
            )
            .1
            .gestures()
            .clone()
    }

    /// A row's drag changed what the list records — a swipe began, listing
    /// the row's dismiss background, or a row was lifted or lowered. That
    /// state is the drag's own, read by no signal, so the drag marks the
    /// list itself; every other drag frame moves live operands only.
    fn mark_rows_relisted(&self) {
        if let Some(cell) = self.owner.upgrade() {
            cell.mark(crate::renderer::Dirty::PAINT);
        }
    }

    /// Slides every other row of the visible window to make room for the
    /// row `lifted` at `from`, dragged to `to`: each row between its origin
    /// and its destination moves one slot, the rest rest. `animation` is the
    /// look's settle; `None` snaps.
    fn shift_rows_for_reorder(
        &self,
        lifted: ListItemId,
        from: usize,
        to: usize,
        animation: Option<&Animation>,
    ) {
        for (id, row) in self.row_gestures.borrow().iter() {
            if *id == lifted {
                continue;
            }
            let RowBinding { index, height, .. } = row.binding.get();
            let shift = if to > from && index > from && index <= to {
                -height
            } else if to < from && index >= to && index < from {
                height
            } else {
                0.0
            };
            row.motion.displace(kurbo::Vec2::new(0.0, shift), animation);
        }
    }

    /// Resolves the sections the current rows group into, from the row ids
    /// alone: no row is materialized, so a virtualized collection inside a
    /// section stays virtualized.
    ///
    /// Returns whether the rows owning chrome changed, which invalidates row
    /// extents: a row's height includes the chrome it owns.
    fn resolve_sections(&self) -> bool {
        if !self.sections_stale.replace(false) {
            return false;
        }
        let spans = self.config.sections().spans(&*self.rows_snapshot.borrow());
        let chrome_rows = |spans: &[ListSectionSpan]| {
            spans
                .iter()
                .filter_map(|span| {
                    let section = span.section.as_ref()?;
                    Some((
                        span.rows.clone(),
                        section.label().is_some(),
                        section.footer().is_some(),
                        section.expanded().is_some(),
                    ))
                })
                .collect::<Vec<_>>()
        };
        let changed = chrome_rows(&spans) != chrome_rows(&self.sections.borrow());
        *self.sections.borrow_mut() = spans;
        changed
    }

    /// Reads every collapsible section's `expanded` binding through `renderer`
    /// (subscribing this list node to external writes) and updates the folded
    /// display layout. Returns whether the visible layout changed.
    fn sync_visible_layout(
        &self,
        row_count: usize,
        renderer: &mut crate::renderer::SemanticCore<E>,
    ) -> bool {
        let next = {
            let sections = self.sections.borrow();
            VisibleLayout::build(row_count, &sections, |binding| {
                renderer.read_signal(binding)
            })
        };
        let mut current = self.visible_layout.borrow_mut();
        if *current == next {
            false
        } else {
            *current = next;
            true
        }
    }

    fn display_len(&self, row_count: usize) -> usize {
        let layout = self.visible_layout.borrow();
        if layout.identity {
            self.transitions.display_len(row_count)
        } else {
            layout.len
        }
    }

    fn display_entry_at(&self, position: usize) -> Option<VisibleEntry> {
        let layout = self.visible_layout.borrow();
        if layout.identity {
            Some(match self.transitions.slot_at(position) {
                crate::renderer::LazySlot::Row(index) => VisibleEntry::Row(index),
                crate::renderer::LazySlot::Ghost(ghost) => VisibleEntry::Ghost(ghost),
            })
        } else {
            layout.entry_at(position)
        }
    }

    fn display_position_of_row(&self, index: usize) -> Option<usize> {
        let layout = self.visible_layout.borrow();
        if layout.identity {
            Some(self.transitions.row_display(index))
        } else {
            layout.display_of_row(index)
        }
    }

    fn nearest_row_at(&self, position: usize) -> Option<usize> {
        let layout = self.visible_layout.borrow();
        if layout.identity {
            Some(self.transitions.row_at(position))
        } else {
            layout.nearest_row_at(position)
        }
    }

    /// Header text, expanded binding, and `starts_later_section` flag for a
    /// collapsed section.
    fn collapsed_section_header(
        &self,
        section_index: usize,
    ) -> Option<(Text, Option<Binding<bool>>, bool)> {
        let sections = self.sections.borrow();
        let section = sections.get(section_index)?.section.as_ref()?;
        let header = section.label().cloned()?;
        Some((header, section.expanded().cloned(), section_index > 0))
    }

    /// The chrome row `index` owns: its section's header when it opens the
    /// section, its footer when it closes it, and section boundary flags used
    /// by grouped / inset-grouped / sidebar styles.
    fn section_chrome(&self, index: usize) -> RowSectionChrome {
        let sections = self.sections.borrow();
        let position = sections
            .partition_point(|span| span.rows.start <= index)
            .checked_sub(1);
        let Some(position) = position else {
            return RowSectionChrome::default();
        };
        let Some(span) = sections
            .get(position)
            .filter(|span| span.rows.contains(&index))
        else {
            return RowSectionChrome::default();
        };
        let is_section_first = index == span.rows.start;
        let is_section_last = index + 1 == span.rows.end;
        let starts_later_section = is_section_first && position > 0;
        let (header, footer, header_expanded) = span.section.as_ref().map_or_else(
            || (None, None, None),
            |section| {
                (
                    is_section_first.then(|| section.label().cloned()).flatten(),
                    is_section_last.then(|| section.footer().cloned()).flatten(),
                    is_section_first
                        .then(|| section.expanded().cloned())
                        .flatten(),
                )
            },
        );
        RowSectionChrome {
            header,
            footer,
            header_expanded,
            is_section_first,
            is_section_last,
            starts_later_section,
        }
    }

    /// Drop the retained sub-views of the rows the collection watcher
    /// reported as replaced since the last consume — a same-id content change
    /// re-materializes exactly those rows on their next `entry`, while every
    /// untouched row keeps its node (focus, gestures, in-flight scroll
    /// anchoring all live inside it).
    fn consume_replaced_rows(&self) {
        let replaced = core::mem::take(&mut *self.replaced_row_ids.borrow_mut());
        if !replaced.is_empty() {
            self.item_cache.borrow_mut().invalidate_ids(&replaced);
            self.realized_rows
                .borrow_mut()
                .retain(|id, _| !replaced.contains(id));
            self.transitions.retain_rows(|id| !replaced.contains(id));
        }
    }

    /// Realizes row `index` of `contents`, whose id is `row_id`, once: its
    /// content's accessibility naming is hoisted onto the row, the content is
    /// built into a retained node under the row's subtree environment, and
    /// what the row needs every frame is kept in [`Self::realized_rows`].
    /// Marks the row visible this frame either way.
    fn realize_row(
        &self,
        contents: &AnyViewsSnapshot<Row<E>>,
        index: usize,
        row_id: ListItemId,
        env: &Environment,
        renderer: &mut crate::renderer::SemanticCore<E>,
    ) {
        let mut cache = self.item_cache.borrow_mut();
        if self.realized_rows.borrow().contains_key(&row_id) && cache.touch(&row_id) {
            return;
        }
        let row_env = list_row_environment(env);
        let content = materialize_list_item(contents, index);
        let subtree_env = self.row_subtree_environment(row_id, &row_env);
        let subview = cache.entry(row_id, move || {
            crate::renderer::RetainedSubview::held(content)
        });
        // The row owns the transition its content root declares; it is
        // claimed before anything walks the row.
        subview.ensure_built(renderer, &subtree_env);
        let scope_env = subview
            .node()
            .and_then(crate::renderer::RenderNode::leading_scope_env);
        let resolved_env = subview.node().map_or(&row_env, |node| {
            crate::renderer::RenderNode::resolved_handler_env(node, &row_env)
        });
        let deletable = resolved_env
            .get::<ListRowDeletable>()
            .map_or_else(|| Computed::new(true), |entry| entry.0.clone());
        let insets = resolved_env
            .get::<ListRowInsets>()
            .map(|entry| entry.0.clone());
        let disabled = crate::widgets::util::widget_disabled(scope_env.unwrap_or(&row_env));
        // The row's `ListItem` node claims the naming scope its content
        // opens with, or the first leaf inside the subtree would claim the
        // row's explicit label for itself.
        #[cfg(feature = "accessibility")]
        let row_env = {
            let mut row_env = row_env;
            if let Some(scope_env) = scope_env {
                adopt_content_naming::<E>(&mut row_env, scope_env, &subtree_env);
            }
            row_env
        };
        self.transitions.track(row_id, subview, renderer);
        self.realized_rows.borrow_mut().insert(
            row_id,
            RealizedRow {
                deletable,
                insets,
                env: row_env,
                disabled,
                subtree_env,
            },
        );
    }

    /// The environment a row's content builds and lays out under: the row's
    /// hoisted environment with the row's selection as `Selected` — so the
    /// row's press target reports it, owner-scoped so nested controls do not
    /// pick it up — and, with accessibility, the container-child shielding
    /// plus the row's persistent activation scope. Silenced taps in the
    /// content delegate into that scope and the row node drains them, so a
    /// semantic `Click` on the row dispatches the retained action.
    fn row_subtree_environment(&self, row_id: ListItemId, row_env: &Environment) -> Environment {
        let mut row_env = row_env.clone();
        if let Some(selection) = self
            .row_selection
            .as_ref()
            .filter(|selection| selection.selects(row_id))
        {
            row_env.insert(Selected(selection.is_selected(row_id)));
        }
        #[cfg(feature = "accessibility")]
        {
            let mut subtree_env =
                accessibility_container_child_environment(&row_env).unwrap_or(row_env);
            let scope = self
                .semantics_scopes
                .borrow_mut()
                .entry(row_id)
                .or_insert_with(ScopedAccessibilitySemantics::new)
                .clone();
            subtree_env.insert(scope);
            subtree_env
        }
        #[cfg(not(feature = "accessibility"))]
        row_env
    }

    fn row_content_proposal(
        row_width: Option<f64>,
        insets: Option<&EdgeInsets>,
        metrics: ListMetrics,
        style_geom: ListStyleGeometry,
        row_env: &Environment,
    ) -> ProposalSize {
        row_width.map_or(ProposalSize::UNSPECIFIED, |row_width| {
            let (left, right) = list_row_horizontal_insets(insets, metrics, row_env);
            let outer = style_geom.outer_horizontal_inset * 2.0;
            ProposalSize::new(
                Some(f64_as_f32((row_width - outer - left - right).max(0.0))),
                None,
            )
        })
    }

    /// Measures realized row `row_id`'s content at the width the row lays it
    /// out at: `row_width` less the list style's outer inset and the row's
    /// horizontal insets, the height open. A row's height comes from this
    /// answer, so content that wraps is held at the lines it wraps to there.
    fn measure_row_content(
        &self,
        row_id: ListItemId,
        row_width: Option<f64>,
        state: &mut HydroState<E>,
        theme: &Rc<dyn WidgetTheme>,
        metrics: ListMetrics,
    ) -> LayoutSize {
        let rows = self.realized_rows.borrow();
        let cache = self.item_cache.borrow();
        let (Some(row), Some(subview)) = (rows.get(&row_id), cache.get(&row_id)) else {
            contract_violation!(
                "hydrolysis List measured row {row_id:?} before realizing it; the row content measures as empty"
            );
            return LayoutSize::zero();
        };
        let proposal = Self::row_content_proposal(
            row_width,
            row.insets.as_ref(),
            metrics,
            self.style_geometry(),
            &row.env,
        );
        subview.measure_built_with_proposal(state, &row.subtree_env, theme, proposal)
    }

    /// Patches and measures realized row `row_id`'s content through
    /// [`RetainedSubview::patch_and_measure`], reusing its cached proposal
    /// measurement when the row took no structural patch, no layout mark, and
    /// no dirty layout dependency.
    fn patch_and_measure_row_content(
        &self,
        row_id: ListItemId,
        row_width: Option<f64>,
        renderer: &mut HydrolysisRenderer<E>,
        metrics: ListMetrics,
    ) -> LayoutSize {
        let style_geom = self.style_geometry();
        let (proposal, subtree_env) = {
            let rows = self.realized_rows.borrow();
            let Some(row) = rows.get(&row_id) else {
                contract_violation!(
                    "hydrolysis List measured row {row_id:?} before realizing it; the row content measures as empty"
                );
                return LayoutSize::zero();
            };
            let proposal = Self::row_content_proposal(
                row_width,
                row.insets.as_ref(),
                metrics,
                style_geom,
                &row.env,
            );
            (proposal, row.subtree_env.clone())
        };
        let mut cache = self.item_cache.borrow_mut();
        let Some(subview) = cache.get_mut(&row_id) else {
            contract_violation!(
                "hydrolysis List measured row {row_id:?} before realizing it; the row content measures as empty"
            );
            return LayoutSize::zero();
        };
        subview
            .patch_and_measure(renderer, &subtree_env, proposal)
            .0
    }

    /// Evicts the realized rows the frame did not touch, with their
    /// transition state; the pinned first row stays.
    fn end_row_frame(&self) {
        let mut cache = self.item_cache.borrow_mut();
        cache.end_frame();
        self.realized_rows
            .borrow_mut()
            .retain(|id, _| cache.get(id).is_some());
        self.transitions.retain_rows(|id| cache.get(id).is_some());
    }

    /// Keeps the first row realized and pinned for the list's own measure,
    /// which samples it and runs with no renderer to build on. Runs at build,
    /// before each measure the list is prepared for, and at the start of each
    /// render and semantic emit, so a membership change is caught before the
    /// list next lays out. Returns whether the pinned row changed.
    pub(crate) fn refresh_first_row(
        &self,
        env: &Environment,
        renderer: &mut crate::renderer::SemanticCore<E>,
    ) -> bool {
        self.consume_replaced_rows();
        let contents = self.rows_snapshot.borrow().clone();
        let first = if contents.is_empty() {
            None
        } else {
            contents.get_id(0)
        };
        let changed = self.item_cache.borrow().pinned().copied() != first;
        self.item_cache.borrow_mut().pin(first);
        let Some(row_id) = first else {
            return changed;
        };
        let realized = self.realized_rows.borrow().contains_key(&row_id)
            && self.item_cache.borrow().get(&row_id).is_some();
        if !realized {
            self.realize_row(&contents, 0, row_id, env, renderer);
        }
        changed || !realized
    }

    /// [`Self::refresh_first_row`] as the widget's layout-time prepare.
    pub(crate) fn prepare_first_row(
        &self,
        renderer: &mut crate::renderer::HydrolysisRenderer<E>,
        env: &Environment,
    ) {
        self.refresh_first_row(env, renderer);
    }

    /// Resets the extent index over the display order — rows and removal
    /// ghosts when all sections are expanded, or the folded visible layout
    /// when any section is collapsed.
    fn reset_extents(&self, len: usize, estimate: f64) {
        let layout = self.visible_layout.borrow();
        if layout.identity {
            drop(layout);
            self.transitions.reset_extents(
                &mut self.extent_index.borrow_mut(),
                len,
                estimate,
                0.0,
                |size| f64::from(size.height),
            );
        } else {
            let visible_len = layout.len;
            drop(layout);
            self.extent_index
                .borrow_mut()
                .reset(visible_len, estimate, 0.0);
        }
    }

    /// Advances every row insertion and removal ghost to the frame instant;
    /// a ghost that finished releases its slot. Requests frames while
    /// anything is in flight. Both the draw and the rendered accessibility
    /// emit call it before they prepare rows, whichever runs first in a
    /// frame: a second call at the same instant advances nothing.
    fn advance_transitions(&self, renderer: &crate::renderer::SemanticCore<E>) {
        let (_, animating) = self.transitions.advance(renderer.frame_instant());
        if animating {
            renderer.context_mark_layout();
        }
    }

    /// Applies the membership change the watcher reported, when there is
    /// one — realized rows that left with a removal declaration become
    /// ghosts holding their slots — and re-bases the extent index on it.
    fn prepare_rows(&self, len: usize, estimate: f64, now: crate::time::Instant) {
        let dirty = self.rows_dirty.replace(false);
        self.consume_replaced_rows();
        if dirty {
            let snapshot = self.rows_snapshot.borrow().clone();
            let _ = self.transitions.apply_membership(
                &snapshot,
                &mut self.item_cache.borrow_mut(),
                now,
            );
        }
        let display = self.display_len(len);
        if dirty || !self.extent_index.borrow().matches(display, estimate, 0.0) {
            self.reset_extents(len, estimate);
            let preserve_anchor_index = self.preserve_anchor_index_once.replace(false);
            let membership_offset = self.viewport_anchor.get().and_then(|anchor| {
                if len == 0 {
                    return None;
                }
                // Anchor re-resolution reads the snapshot the last applied
                // event carried — the same membership `prepare_rows` and the
                // visible-window loop reconcile against.
                let snapshot = self.rows_snapshot.borrow();
                let index = if preserve_anchor_index {
                    anchor.index.min(len - 1)
                } else if snapshot.get_id(anchor.index) == Some(anchor.id) {
                    anchor.index
                } else {
                    (0..len)
                        .find(|index| snapshot.get_id(*index) == Some(anchor.id))
                        .unwrap_or_else(|| anchor.index.min(len - 1))
                };
                let display_pos = self.display_position_of_row(index)?;
                let rebased = self.extent_index.borrow().offset_of(display_pos)
                    + anchor.offset_within_row.min(estimate);
                // Written back in the new coordinates: `prepare_rows` runs
                // twice a frame — the semantic pass, then the render pass —
                // and a collection update between them must translate by its
                // own change only, not this one again.
                self.viewport_anchor.set(Some(ListViewportAnchor {
                    index,
                    recorded_offset: rebased,
                    ..anchor
                }));
                Some((rebased, anchor.recorded_offset))
            });
            self.pending_membership_offset.set(membership_offset);
        }
    }

    /// §7.1's scroll-surface rule: [`ScrollSurfaceArea::extended`] grows the
    /// viewport and content by the extension layout computed, so the
    /// scrollable range keeps the resting edges on the avoided boundary
    /// while scrolling paints through the band.
    fn bind_scroll(
        &self,
        viewport_width: f64,
        viewport_height: f64,
        content_height: f64,
        position_report: Option<ScrollPositionReport>,
    ) -> ScrollHandle {
        let (viewport, content) = self.surface.extended(
            kurbo::Size::new(viewport_width, viewport_height),
            kurbo::Size::new(viewport_width, content_height),
        );
        // The membership anchor always applies — `prepare_rows`' extent
        // reset discarded measured heights, so skipping it leaves the offset
        // in the old coordinates and the viewport jumps. It is a pure
        // translation inside the rebind, ahead of the new extents' clamp, and
        // claims nothing, so a live run or fling keeps driving through it.
        let shift = self
            .pending_membership_offset
            .take()
            .map_or(0.0, |(rebased, recorded)| rebased - recorded);
        // Clone the slot's handle under a short borrow and rebind it with
        // the borrow released: the rebind's report writes run watchers that
        // may re-enter this list, and a `self.scroll` borrow held across
        // them would panic.
        let existing = self.scroll.borrow().clone();
        if let Some(mut handle) = existing {
            let rebound = handle.rebind(
                ScrollAxis::Vertical,
                viewport.width,
                viewport.height,
                content.width,
                content.height,
                (0.0, shift),
                position_report,
            );
            *self.scroll.borrow_mut() = Some(rebound.clone());
            rebound
        } else {
            let handle = ScrollHandle::new(
                ScrollAxis::Vertical,
                viewport.width,
                viewport.height,
                content.width,
                content.height,
                None,
            );
            handle.set_position_report(position_report);
            *self.scroll.borrow_mut() = Some(handle.clone());
            handle
        }
    }

    /// The position report this list's scroll controller reports through,
    /// described by the logical — unextended — viewport and content sizes.
    fn position_report(
        &self,
        viewport_width: f64,
        viewport_height: f64,
        content_height: f64,
    ) -> Option<ScrollPositionReport> {
        use crate::num_cast::f64_as_f32;
        self.config
            .scroll_controller()
            .as_ref()
            .map(|controller| ScrollPositionReport {
                binding: controller.position_report(),
                viewport_size: LayoutSize::new(
                    f64_as_f32(viewport_width),
                    f64_as_f32(viewport_height),
                ),
                content_size: LayoutSize::new(
                    f64_as_f32(viewport_width),
                    f64_as_f32(content_height),
                ),
            })
    }

    /// Applies the pending scroll request, if there is one. `animate` selects
    /// between the rendered animation and the semantic jump: nothing ticks the
    /// list's smooth scroll on the semantic runtime, so a request there lands
    /// in place instead.
    ///
    /// Returns whether this application moved the offset — a jump, an
    /// approach jump, or a landing correction. Arming or refining a run moves
    /// nothing: the run's own ticks carry the offset from here, as
    /// continuation work rather than a change still to be applied.
    fn resolve_scroll_target_index(
        &self,
        target: ListTagId,
        cached_index: Option<usize>,
        tags: &ListRowTags,
    ) -> Option<usize> {
        let snapshot = self.rows_snapshot.borrow();
        if let Some(index) = cached_index
            && snapshot.get_id(index).and_then(|id| tags.tag(id)) == Some(target)
        {
            return Some(index);
        }
        snapshot
            .range()
            .find(|&index| snapshot.get_id(index).and_then(|id| tags.tag(id)) == Some(target))
    }

    fn poll_pending_scroll(
        &self,
        renderer: &mut crate::renderer::SemanticCore<E>,
        controller: &ListScrollController,
    ) -> Option<PendingScroll> {
        let generation = renderer.read_signal(&controller.generation());
        if generation != self.applied_scroll_generation.get()
            && self
                .pending_scroll
                .borrow()
                .as_ref()
                .is_none_or(|pending| pending.generation != generation)
        {
            let request = renderer.read_signal(&controller.request());
            *self.pending_scroll.borrow_mut() = Some(PendingScroll {
                generation,
                target: *request.target(),
                cached_index: None,
                animation: request.animation().cloned(),
                state: PendingScrollState::Unarmed,
            });
        }
        self.pending_scroll.borrow().clone()
    }

    #[must_use]
    fn apply_scroll_request(
        &self,
        renderer: &mut crate::renderer::SemanticCore<E>,
        handle: &ScrollHandle,
        row_count: usize,
        animate: bool,
    ) -> bool {
        let Some(controller) = self.config.scroll_controller() else {
            return false;
        };
        let Some(pending) = self.poll_pending_scroll(renderer, controller) else {
            return false;
        };
        let Some(target_index) = self.resolve_scroll_target_index(
            pending.target,
            pending.cached_index,
            controller.row_tags(),
        ) else {
            return false;
        };
        if target_index >= row_count {
            return false;
        }
        if pending.cached_index != Some(target_index)
            && let Some(stored) = self.pending_scroll.borrow_mut().as_mut()
        {
            stored.cached_index = Some(target_index);
        }
        // Re-issuing the target every frame is what keeps a virtualized
        // request accurate: rows measured while the list moves past them move
        // `offset_of(index)`, so the destination is refined until it actually
        // lands. A row inside a collapsed section has no display slot yet.
        let Some(pending_position) = self.display_position_of_row(target_index) else {
            return false;
        };
        let offset = self.extent_index.borrow().offset_of(pending_position);
        let moved = match pending.animation.clone() {
            Some(animation) if animate => match pending.state {
                PendingScrollState::Armed { run } => match handle.scroll_run_outcome(&run) {
                    ScrollRunOutcome::Running => {
                        // Already gliding: refine the destination without
                        // restarting the run's clock. Only the run this
                        // request armed answers — a different owner's
                        // animation on the same surface is never steered.
                        let _ = handle.retarget_animated_scroll(&run, 0.0, offset);
                        false
                    }
                    ScrollRunOutcome::Landed => {
                        // The motion is over; settling the final position
                        // as rows re-measure is a correction, so it jumps
                        // rather than arming a second full-duration run.
                        handle.scroll_to(0.0, offset)
                    }
                    ScrollRunOutcome::Interrupted => {
                        // Something else claimed the offset — user input,
                        // a jump, or another owner's run (keyboard
                        // clearance). The request is spent; consuming its
                        // generation keeps the supersede rule uniform.
                        self.applied_scroll_generation.set(pending.generation);
                        self.pending_scroll.take();
                        return false;
                    }
                },
                PendingScrollState::Unarmed => {
                    let metrics = handle.metrics();
                    let start_pos = self
                        .extent_index
                        .borrow()
                        .visible_window(
                            metrics.offset_y,
                            metrics.offset_y + metrics.viewport_height,
                        )
                        .start;
                    let current = self.nearest_row_at(start_pos).unwrap_or(0);
                    // Animating the whole way across a large dataset would
                    // drag the list through every viewport between here and
                    // there, and read as a blur regardless: jump to within
                    // the approach bound of the target and animate only that
                    // final stretch.
                    let approached = animated_row_scroll_approach(current, target_index)
                        .and_then(|approach_index| self.display_position_of_row(approach_index))
                        .is_some_and(|approach_pos| {
                            let approach = self.extent_index.borrow().offset_of(approach_pos);
                            handle.scroll_to(0.0, approach)
                        });
                    // Arming starts the run's clock at the frame instant —
                    // the very next tick already shows motion, and keeps
                    // requesting frames until the run lands. The handle is
                    // bound live this frame, and the pending request being
                    // applied is the one still stored — both hold.
                    let run = handle
                        .scroll_to_animated(0.0, offset, animation, renderer.frame_instant())
                        .expect(
                            "the list binds a live scroll handle before arming a request on it",
                        );
                    self.pending_scroll
                        .borrow_mut()
                        .as_mut()
                        .expect("the pending request being applied is still stored")
                        .state = PendingScrollState::Armed { run };
                    approached
                }
            },
            _ => {
                // `None` (a jump) or the semantic runtime, whose pump never
                // registers the list's handle in its scroll targets — either
                // way the request lands in place. Re-issuing while pending
                // keeps it self-correcting as rows measure.
                handle.scroll_to(0.0, offset)
            }
        };
        let extent_index = self.extent_index.borrow();
        let Some(extent) = extent_index.measured(pending_position) else {
            return moved;
        };
        let metrics = handle.metrics();
        let row_start = extent_index.offset_of(pending_position);
        let row_end = row_start + extent;
        // The surface's safe-area extension covers rows a `scroll_to` was
        // asked to reveal, so the visibility check ends before the bands it
        // reaches under rather than at the grown viewport's bottom.
        let viewport_end =
            metrics.offset_y + metrics.viewport_height - self.surface.extension().vertical();
        let row_visible = row_end > metrics.offset_y && row_start < viewport_end;
        if row_visible && !handle.is_smooth_scrolling() {
            self.applied_scroll_generation.set(pending.generation);
            self.pending_scroll.take();
        }
        moved
    }

    fn record_viewport_anchor(&self, metrics: crate::scroll::ScrollMetrics, row_count: usize) {
        let window = self
            .extent_index
            .borrow()
            .visible_window(metrics.offset_y, metrics.offset_y + metrics.viewport_height);
        // A removal ghost at the top anchors on the row it sits before; a
        // collapsed section header anchors on that section's first row.
        let Some(index) = self.nearest_row_at(window.start) else {
            self.viewport_anchor.set(None);
            return;
        };
        if index >= row_count {
            self.viewport_anchor.set(None);
            return;
        }
        let Some(id) = self.rows_snapshot.borrow().get_id(index) else {
            contract_violation!(
                "hydrolysis List item {index} has no stable id although it is in the snapshot's range; the viewport anchor is cleared"
            );
            self.viewport_anchor.set(None);
            return;
        };
        let row_start = self
            .extent_index
            .borrow()
            .offset_of(self.display_position_of_row(index).unwrap_or(window.start));
        self.viewport_anchor.set(Some(ListViewportAnchor {
            id,
            index,
            offset_within_row: (metrics.offset_y - row_start).max(0.0),
            recorded_offset: metrics.offset_y,
        }));
    }
}

/// A row contributes up to six accessibility nodes — the section header it
/// opens, the row itself, the delete control and the two reorder halves edit
/// mode draws on it, and the section footer it closes — so each row's id owns
/// a slot of six keys rather than one.
#[cfg(feature = "accessibility")]
const A11Y_KEYS_PER_ROW: i64 = 6;
#[cfg(feature = "accessibility")]
const A11Y_KEY_ROW: i64 = 0;
#[cfg(feature = "accessibility")]
const A11Y_KEY_HEADER: i64 = 1;
#[cfg(feature = "accessibility")]
const A11Y_KEY_FOOTER: i64 = 2;
#[cfg(feature = "accessibility")]
const A11Y_KEY_DELETE: i64 = 3;
#[cfg(feature = "accessibility")]
const A11Y_KEY_MOVE_UP: i64 = 4;
#[cfg(feature = "accessibility")]
const A11Y_KEY_MOVE_DOWN: i64 = 5;

#[cfg(feature = "accessibility")]
fn row_a11y_key_base(row_id: ListItemId) -> i64 {
    i64::from(i32::from(row_id)) * A11Y_KEYS_PER_ROW
}

/// Registers one section header or footer as its own accessibility node.
///
/// Section chrome is not part of any row: a screen reader announces "Activity"
/// as a heading (or a disclosure button when collapsible) and then the rows
/// under it, rather than folding the title into the first row's label. The
/// label is read through the renderer so a title driven by a signal re-flushes
/// the tree when it changes.
#[cfg(feature = "accessibility")]
#[expect(
    clippy::needless_pass_by_value,
    reason = "the parameter is a small Copy value taken by value for a uniform call-site signature"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "section chrome registration carries renderer, context, key, text, role/expansion state, bounds, and env"
)]
fn register_section_chrome_node<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    ctx: Option<RenderContext>,
    semantic_key: i64,
    label: Text,
    is_header: bool,
    expanded: Option<Binding<bool>>,
    bounds: kurbo::Rect,
    env: &Environment,
) -> Option<AccessibilityNodeId> {
    let role = if is_header {
        if expanded.is_some() {
            AccessibilityNodeRole::Button
        } else {
            AccessibilityNodeRole::Header
        }
    } else {
        AccessibilityNodeRole::Footer
    };
    let styled = renderer.read_resolved_text_styled(&label, env);
    let mut node = AccessibilityNode::new(
        crate::renderer::SemanticCore::<E>::resolve_accessibility_role(env, role),
    );
    node.set_label(styled.to_string());
    let target = expanded.map(|binding| {
        let is_expanded = renderer.read_signal(&binding);
        node.set_expanded(is_expanded);
        node.add_action(AccessibilityAction::Focus);
        node.add_action(AccessibilityAction::Click);
        node.add_action(AccessibilityAction::Expand);
        node.add_action(AccessibilityAction::Collapse);
        AccessibilityActionTarget::Toggle {
            binding,
            mixed: None,
        }
    });
    match ctx {
        Some(_ctx) => renderer.register_accessibility_child_node_with_key(
            semantic_key,
            node,
            bounds,
            env,
            target,
        ),
        None => renderer.register_accessibility_child_node_with_key_semantic(
            semantic_key,
            node,
            env,
            target,
        ),
    }
}

/// The logical size of a frame-space rect.
const fn kurbo_logical_size(rect: kurbo::Rect) -> LayoutSize {
    LayoutSize::new(
        crate::num_cast::f64_as_f32(rect.width()),
        crate::num_cast::f64_as_f32(rect.height()),
    )
}

/// Paints removal ghost `index` in its frozen slot, whose top rests at
/// `resting_y`: the list's row background across the slot, then the row's
/// content under its removal transition. Paint only — a ghost left the
/// semantic tree, takes no input and draws no row controls
/// (layout-spec.md §4.5).
fn draw_list_ghost<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ListRenderState<E>>>,
    env: &Environment,
    index: usize,
    resting_y: f64,
) {
    let state_ref = state.borrow();
    let mut ghost = state_ref.transitions.ghost(index);
    let placement = ghost.placement;
    let viewport = ctx.bounds;
    let slot = kurbo::Rect::new(
        viewport.x0,
        resting_y,
        viewport.x1,
        resting_y + placement.slot.height(),
    );
    let look = state_ref.look(&ctx.theme(), env);
    ctx.draw_context(|draw| {
        look.record_row(&mut *draw, slot, 0);
    });
    let content = placement.content + slot.origin().to_vec2();
    let size = kurbo_logical_size(content);
    let owner = state_ref
        .owner
        .upgrade()
        .expect("hydrolysis List: a flushing list's cell is alive");
    let direction = state_ref.transitions.direction;
    let crate::renderer::LazyGhost { ghost, runtime, .. } = &mut *ghost;
    let paint = crate::renderer::TransitionPaint::sample(
        &ghost.transition,
        size,
        direction,
        false,
        runtime,
        &owner,
    );
    let delta = kurbo::Affine::translate((content.x0, content.y0));
    let child_ctx = ctx.render_context().child(
        delta,
        kurbo::Rect::from_origin_size(kurbo::Point::ZERO, content.size()),
    );
    let realization = &mut ghost.realization;
    ctx.renderer_mut()
        .with_suppressed_accessibility(|renderer| {
            crate::renderer::flush_transitioned(
                renderer,
                child_ctx,
                delta,
                &paint,
                |renderer, inner| {
                    realization.place_in_ctx(
                        renderer,
                        inner,
                        env,
                        placement.proposal,
                        size,
                        None,
                        kurbo::Affine::IDENTITY,
                    );
                },
            );
        });
}

/// Emits a list's accessibility tree from its node-owned retained state.
///
/// The rendered flush passes its [`RenderContext`] and theme: row extents are
/// real, and the emitted window is the scrolled viewport's. The semantic walk
/// passes `None` for both and emits every row — the semantic tree has no
/// viewport, and a row not emitted does not exist to assistive technology.
/// Scroll offsets then read as row indices: the scroll domain is measured in
/// rows, since the semantic path has no pixels to measure in.
#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn list_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    ctx: Option<RenderContext>,
    theme: Option<&Rc<dyn crate::theme::WidgetTheme>>,
    state: &Rc<RefCell<ListRenderState<E>>>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    let owner = state;
    let state = state.borrow();
    state.refresh_first_row(env, renderer);
    let list = &state.config;
    // Row count and every row read below come from the retained snapshot —
    // the membership the last applied event delivered, not a live re-read.
    let rows_snapshot = state.rows_snapshot.borrow().clone();
    let row_count = rows_snapshot.len();
    let list_metrics = theme.map(|theme| state.look(theme, env).metrics());
    let style_geom = state.style_geometry();
    let effective_min_height = state.effective_min_row_height();
    if let Some(list_metrics) = list_metrics {
        // `min_row_height` is the floor every row is estimated and measured
        // against; unset, the style's or theme's one-line height applies.
        let row_floor = f64::from(effective_min_height.unwrap_or(list_metrics.one_line_row_height))
            + style_geom.row_gap;
        // A 0 floor still needs a positive seed estimate — measured extents
        // replace it row by row anyway.
        let row_estimate = row_floor.max(1.0);
        // The transition clocks advance before the extents re-base on what
        // remains.
        state.advance_transitions(renderer);
        // Resolve sections and collapsible section states ahead of extent
        // preparation so collapsed rows leave the display index in one step.
        let sections_changed = state.resolve_sections();
        let layout_changed = state.sync_visible_layout(row_count, renderer);
        state.prepare_rows(row_count, row_estimate, renderer.frame_instant());
        if sections_changed || layout_changed {
            state.reset_extents(row_count, row_estimate);
            if layout_changed {
                renderer.context_mark_layout();
            }
        }
    } else {
        // The semantic path keeps section chrome current but measures rows in
        // units — one row is one extent unit, so scroll offsets read as row
        // indices. It still consumes replaced-row invalidations: the semantic
        // emit reads the same retained rows.
        state.consume_replaced_rows();
        let _ = state.resolve_sections();
        let _ = state.sync_visible_layout(row_count, renderer);
        let display = state.display_len(row_count);
        let mut extent_index = state.extent_index.borrow_mut();
        if !extent_index.matches(display, 1.0, 0.0) {
            extent_index.reset(display, 1.0, 0.0);
        }
    }
    let is_rendered = ctx.is_some();
    let viewport = ctx.map_or(kurbo::Rect::ZERO, |ctx| ctx.bounds());
    // The rendered scroll domain is the measured extent; the semantic one is
    // the row count — with a zero viewport every row is scrollable to.
    let content_height = state
        .extent_index
        .borrow()
        .total_extent()
        .max(viewport.height());
    // The semantic walk without a `ctx` never painted a frame: it must not
    // report its zeroed stand-in rect over the painted geometry, so the
    // position report is attached only when the bounds are real.
    let position_report = is_rendered
        .then(|| state.position_report(viewport.width(), viewport.height(), content_height))
        .flatten();
    let handle = state.bind_scroll(
        viewport.width(),
        viewport.height(),
        content_height,
        position_report,
    );
    // Applied ahead of the emit below, so a move it makes is in this frame's
    // tree already.
    let _ = state.apply_scroll_request(renderer, &handle, row_count, is_rendered);
    #[cfg(feature = "accessibility")]
    {
        let metrics = handle.metrics();
        let (emit_range, leading_offset) = if is_rendered {
            let span = state.surface.visible_span(&metrics, ScrollAxis::Vertical);
            let window = state
                .extent_index
                .borrow()
                .visible_window(span.start, span.end);
            (window.start..window.end, window.leading_offset)
        } else {
            (0..state.display_len(row_count), 0.0)
        };
        let mut list_node = AccessibilityNode::new(
            crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
                env,
                AccessibilityNodeRole::List,
            ),
        );
        let list_label = renderer.resolve_accessibility_label(env, None);
        if let Some(label) = list_label {
            list_node.set_label(label);
        }
        if let Some(value) = renderer.resolve_accessibility_value(env, None) {
            list_node.set_value(value);
        }
        list_node.set_scroll_y(metrics.offset_y);
        list_node.set_scroll_y_min(0.0);
        list_node.set_scroll_y_max(metrics.max_y);
        list_node.add_action(AccessibilityAction::ScrollUp);
        list_node.add_action(AccessibilityAction::ScrollDown);
        let editing = renderer.read_signal(list.editing());
        if ctx.is_none() {
            // The semantic walk emits every row's content through the shared
            // sub-view cache — the same frame bookkeeping the rendered flush
            // runs keeps a row's retained node alive across emissions and
            // evicts the ones no row touched this pass.
            state.item_cache.borrow_mut().begin_frame();
        }
        // The surface's §7.1 extension — the bounds rows may paint into —
        // while `viewport` stays the laid-out frame the row math is anchored
        // on.
        let surface_viewport = crate::renderer::grow_rect(viewport, state.surface.extension());
        let mut y = viewport.y0 - metrics.offset_y + leading_offset;
        for position in emit_range {
            // The window runs over display positions: removal ghosts hold their
            // slots without semantic nodes, and collapsed sections emit only
            // their interactive header node while hiding their rows and footer.
            let Some(entry) = state.display_entry_at(position) else {
                continue;
            };
            let index = match entry {
                VisibleEntry::Row(index) => index,
                VisibleEntry::Ghost(ghost) => {
                    if is_rendered {
                        y += f64::from(state.transitions.ghost(ghost).size().height);
                    }
                    continue;
                }
                VisibleEntry::CollapsedHeader {
                    section_index,
                    first_row_index,
                } => {
                    let Some(first_row_id) = rows_snapshot.get_id(first_row_index) else {
                        continue;
                    };
                    let Some((header, expanded_binding, starts_later)) =
                        state.collapsed_section_header(section_index)
                    else {
                        continue;
                    };
                    let top_gap = if starts_later {
                        style_geom.inter_section_gap
                    } else {
                        0.0
                    };
                    let header_height =
                        list_metrics.map_or(0.0, |m| f64::from(m.section_header_height));
                    let slot_height = if is_rendered {
                        let cached = state.extent_index.borrow().measured(position);
                        if let Some(extent) = cached {
                            extent
                        } else {
                            let extent = top_gap + header_height;
                            state
                                .extent_index
                                .borrow_mut()
                                .set_measured(position, extent);
                            extent
                        }
                    } else {
                        0.0
                    };
                    let slot_rect = kurbo::Rect::new(viewport.x0, y, viewport.x1, y + slot_height);
                    y += slot_height;
                    if is_rendered
                        && (slot_rect.y1 <= surface_viewport.y0
                            || slot_rect.y0 >= surface_viewport.y1)
                    {
                        continue;
                    }
                    let outer_inset = style_geom.outer_horizontal_inset;
                    let header_rect = kurbo::Rect::new(
                        slot_rect.x0 + outer_inset,
                        slot_rect.y0 + top_gap,
                        (slot_rect.x1 - outer_inset).max(slot_rect.x0 + outer_inset),
                        slot_rect.y0 + top_gap + header_height,
                    );
                    let key_base = row_a11y_key_base(first_row_id);
                    if let Some(node_id) = register_section_chrome_node(
                        renderer,
                        ctx,
                        key_base + A11Y_KEY_HEADER,
                        header,
                        true,
                        expanded_binding,
                        header_rect,
                        env,
                    ) {
                        list_node.push_child(node_id);
                        renderer.register_accessibility_focus_link(
                            &crate::renderer::InteractionKey::for_rc(
                                owner,
                                row_interaction_base(first_row_id)
                                    + INTERACTION_SLOT_SECTION_HEADER,
                            ),
                            node_id,
                        );
                    }
                    continue;
                }
            };
            let position = state.display_position_of_row(index).unwrap_or(position);
            let row_env = env.clone();
            let Some(row_id) = rows_snapshot.get_id(index) else {
                contract_violation!(
                    "hydrolysis list row {index} has no stable identity although it is in the snapshot's range; the row exposes no accessibility node"
                );
                continue;
            };
            state.realize_row(&rows_snapshot, index, row_id, env, renderer);
            let (row_a11y_env, row_deletable, row_insets, row_disabled_signal, subtree_env) = {
                let rows = state.realized_rows.borrow();
                let row = &rows[&row_id];
                (
                    row.env.clone(),
                    row.deletable.clone(),
                    row.insets.clone(),
                    row.disabled.clone(),
                    row.subtree_env.clone(),
                )
            };
            let chrome = state.section_chrome(index);
            // Semantic rows have no layout extent — the slot is only measured
            // when the rendered path needs it to place the row.
            let slot_height = if is_rendered {
                let cached_extent = state.extent_index.borrow().measured(position);
                if let Some(extent) = cached_extent {
                    extent
                } else if let (Some(theme), Some(list_metrics)) = (theme, list_metrics) {
                    let content_size = state.measure_row_content(
                        row_id,
                        Some(viewport.width()),
                        renderer.state_mut(),
                        theme,
                        list_metrics,
                    );
                    let extent = list_row_height_for_content(
                        f64::from(content_size.height),
                        row_insets.as_ref(),
                        effective_min_height,
                        list_metrics,
                    ) + style_geom.row_gap
                        + chrome.total_height(&list_metrics, style_geom);
                    state
                        .extent_index
                        .borrow_mut()
                        .set_measured(position, extent);
                    extent
                } else {
                    contract_violation!(
                        "hydrolysis rendered list measurement requires a theme and list metrics; the row slot measures as zero height"
                    );
                    0.0
                }
            } else {
                0.0
            };
            let slot_rect = kurbo::Rect::new(viewport.x0, y, viewport.x1, y + slot_height);
            y += slot_height;
            if is_rendered
                && (slot_rect.y1 <= surface_viewport.y0 || slot_rect.y0 >= surface_viewport.y1)
            {
                continue;
            }
            let top_gap = chrome.top_gap(style_geom);
            let header_height = list_metrics.map_or(0.0, |m| chrome.header_height(&m));
            let footer_height = list_metrics.map_or(0.0, |m| chrome.footer_height(&m));
            // The chrome a row owns is not part of the row: a section title is
            // its own node, and the row's bounds are the band left between the
            // header and the footer — the same split the draw pass makes.
            let outer_inset = style_geom.outer_horizontal_inset;
            let row_rect = kurbo::Rect::new(
                slot_rect.x0 + outer_inset,
                slot_rect.y0 + top_gap + header_height,
                (slot_rect.x1 - outer_inset).max(slot_rect.x0 + outer_inset),
                (slot_rect.y1 - footer_height - style_geom.row_gap)
                    .max(slot_rect.y0 + top_gap + header_height),
            );
            let key_base = row_a11y_key_base(row_id);
            let row_interaction_base = row_interaction_base(row_id);
            if let Some(header) = chrome.header.clone() {
                let header_rect = if chrome.header_expanded.is_some() {
                    kurbo::Rect::new(
                        slot_rect.x0 + outer_inset,
                        slot_rect.y0 + top_gap,
                        (slot_rect.x1 - outer_inset).max(slot_rect.x0 + outer_inset),
                        slot_rect.y0 + top_gap + header_height,
                    )
                } else {
                    let inset =
                        list_metrics.map_or(0.0, |m| f64::from(m.horizontal_inset)) + outer_inset;
                    kurbo::Rect::new(
                        slot_rect.x0 + inset,
                        slot_rect.y0 + top_gap,
                        (slot_rect.x1 - inset).max(slot_rect.x0 + inset),
                        slot_rect.y0 + top_gap + header_height,
                    )
                };
                if let Some(node_id) = register_section_chrome_node(
                    renderer,
                    ctx,
                    key_base + A11Y_KEY_HEADER,
                    header,
                    true,
                    chrome.header_expanded.clone(),
                    header_rect,
                    &row_env,
                ) {
                    list_node.push_child(node_id);
                    if chrome.header_expanded.is_some() {
                        renderer.register_accessibility_focus_link(
                            &crate::renderer::InteractionKey::for_rc(
                                owner,
                                row_interaction_base + INTERACTION_SLOT_SECTION_HEADER,
                            ),
                            node_id,
                        );
                    }
                }
            }
            // The row's `ListItem` node claims the naming its content carries
            // — hoisted onto the row when the row was realized — and the
            // subtree emits under the container-child env that strips that
            // naming, so the row's name is announced once and children keep
            // their own. The row's gate is the `Disabled` scope its content
            // resolves under — the same env the content's presses read — so
            // a `.disabled` row keeps its label and reports disabled but is
            // inert to selection.
            let row_disabled = renderer.read_signal(&row_disabled_signal);
            // A hidden row vanishes whole — node and content — matching the
            // naming container's treatment of `accessibilityHidden`.
            let hidden_prop = renderer.is_accessibility_hidden(&row_a11y_env);
            let hidden_state = row_a11y_env
                .get::<AccessibilityStateSignal>()
                .is_some_and(|signal| renderer.read_signal(signal.state()).is_hidden());
            let row_hidden = hidden_prop || hidden_state;
            let mut row_node = AccessibilityNode::new(
                crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
                    &row_a11y_env,
                    AccessibilityNodeRole::ListItem,
                ),
            );
            let default_label = state
                .item_cache
                .borrow()
                .get(&row_id)
                .and_then(|row| row.accessibility_label(renderer));
            let label = renderer.resolve_accessibility_label(&row_a11y_env, default_label);
            if let Some(label) = label {
                row_node.set_label(label);
            }
            row_node.add_action(AccessibilityAction::Focus);
            // The selected state belongs to the list's selection, not the
            // item: a row exposes it only when the list can select it.
            if let Some(selection) = state
                .row_selection
                .as_ref()
                .filter(|selection| selection.selects(row_id))
            {
                row_node.set_selected(renderer.read_signal(&selection.is_selected(row_id)));
            }
            let row_node_id = if row_hidden {
                None
            } else {
                // Arrow-key navigation moves through the row target:
                // `ScrollIntoView` reveals the row's span in the list's scroll
                // domain, and `Click` resolves the activation a pointer click
                // on the row's centre would run — Enter/Space fire it.
                row_node.add_action(AccessibilityAction::ScrollIntoView);
                // A disabled row reports disabled, advertises no `Click`, and
                // its `ListRow` target carries no selection to write — a
                // dispatched `Click` resolves only the row content's own
                // activations.
                if row_disabled {
                    row_node.set_disabled();
                } else {
                    row_node.add_action(AccessibilityAction::Click);
                }
                let row_target = Some(AccessibilityActionTarget::ListRow {
                    index,
                    position,
                    handle: handle.clone(),
                    extents: Rc::clone(&state.extent_index),
                    extension: state.surface.extension(),
                    id: row_id,
                    selection: if row_disabled {
                        None
                    } else {
                        state
                            .row_selection
                            .clone()
                            .filter(|selection| selection.selects(row_id))
                    },
                });
                match ctx {
                    Some(_ctx) => renderer.register_accessibility_child_node_with_key(
                        key_base + A11Y_KEY_ROW,
                        row_node,
                        row_rect,
                        &row_a11y_env,
                        row_target,
                    ),
                    None => renderer.register_accessibility_child_node_with_key_semantic(
                        key_base + A11Y_KEY_ROW,
                        row_node,
                        &row_a11y_env,
                        row_target,
                    ),
                }
            };
            if let Some(row_node_id) = row_node_id {
                list_node.push_child(row_node_id);
                // Link each interaction slot on this row to its accessibility
                // node so pointer presses land keyboard focus on it.
                renderer.register_accessibility_focus_link(
                    &crate::renderer::InteractionKey::for_rc(
                        owner,
                        row_interaction_base + INTERACTION_SLOT_ROW_ANCHOR,
                    ),
                    row_node_id,
                );
                // The row's own press slot — the selection target the draw
                // pass registers — resolves focus to the same node.
                renderer.register_accessibility_focus_link(
                    &crate::renderer::InteractionKey::for_rc(
                        owner,
                        row_interaction_base + INTERACTION_SLOT_ROW_PRESS,
                    ),
                    row_node_id,
                );
                let row_can_delete = list.edits().can_delete(row_id);
                let row_movable_span = list.edits().movable_span(&rows_snapshot, index);
                let deletable = editing && row_can_delete && renderer.read_signal(&row_deletable);
                #[cfg(hydrolysis_cherenkov)]
                if ctx.is_none() {
                    // Emit the row content's own semantics under the row's node:
                    // every text, control and image in the row becomes a child
                    // of its `ListItem`, so row content reaches the semantic
                    // tree. The rendered path parents the same subtree in
                    // `render_list_parts`.
                    renderer.push_accessibility_parent(row_node_id);
                    state
                        .item_cache
                        .borrow_mut()
                        .get_mut(&row_id)
                        .expect("hydrolysis List: an emitted row is realized")
                        .emit_accessibility(renderer, &subtree_env);
                    renderer.pop_accessibility_parent();
                    renderer.drain_claim_scope(row_node_id, &subtree_env);
                }
                // Edit mode's delete and reorder controls are pointer-only hit
                // regions in the draw pass — emit their nodes too, or the tree
                // is identical to a non-editing list and nothing can delete or
                // reorder a row through assistive technology
                // (water-rs/hydrolysis#52).
                //
                // The nodes take the same rects the draw pass paints and
                // hit-tests from `row_edit_controls`; the semantic walk has no
                // metrics, so its nodes carry no bounds.
                let controls = list_metrics.map(|metrics| {
                    row_edit_controls(
                        &metrics,
                        row_rect,
                        slot_rect.height(),
                        index,
                        editing.then(|| row_movable_span.clone()).flatten(),
                        deletable,
                    )
                });
                if deletable {
                    let state = Rc::clone(owner);
                    let action_env = row_env.clone();
                    let node_id = register_edit_control_node(
                        renderer,
                        ctx,
                        key_base + A11Y_KEY_DELETE,
                        crate::localization::text(&subtree_env, "delete"),
                        controls.as_ref().and_then(|controls| controls.delete),
                        &subtree_env,
                        Rc::new(RefCell::new(
                            move |_renderer: &mut crate::renderer::SemanticCore<E>,
                                  _env: &Environment| {
                                run_row_delete(&state, &action_env, row_id)
                            },
                        )),
                    );
                    if let Some(node_id) = node_id {
                        list_node.push_child(node_id);
                        renderer.register_accessibility_focus_link(
                            &crate::renderer::InteractionKey::for_rc(
                                owner,
                                row_interaction_base + 2,
                            ),
                            node_id,
                        );
                    }
                }
                if editing && let Some(span) = row_movable_span {
                    // The handle's halves are the two directions the draw pass
                    // presses on: a row at a part boundary advertises only the
                    // direction it can move within its part.
                    for (label_key, up, key, slot) in [
                        ("move_up", true, A11Y_KEY_MOVE_UP, 0usize),
                        ("move_down", false, A11Y_KEY_MOVE_DOWN, 1usize),
                    ] {
                        let enabled = if up {
                            index > span.start
                        } else {
                            index + 1 < span.end
                        };
                        if !enabled {
                            continue;
                        }
                        let to = if up { index - 1 } else { index + 1 };
                        let Some(target_id) = rows_snapshot.get_id(to) else {
                            continue;
                        };
                        let from_in_part = index - span.start;
                        let to_in_part = to - span.start;
                        let state = Rc::clone(owner);
                        let action_env = row_env.clone();
                        let node_id = register_edit_control_node(
                            renderer,
                            ctx,
                            key_base + key,
                            crate::localization::text(&subtree_env, label_key),
                            controls.as_ref().and_then(|controls| {
                                if up {
                                    controls.reorder_up
                                } else {
                                    controls.reorder_down
                                }
                            }),
                            &subtree_env,
                            Rc::new(RefCell::new(
                                move |_renderer: &mut crate::renderer::SemanticCore<E>,
                                      _env: &Environment| {
                                    run_row_move(
                                        &state,
                                        &action_env,
                                        row_id,
                                        target_id,
                                        from_in_part,
                                        to_in_part,
                                    )
                                },
                            )),
                        );
                        if let Some(node_id) = node_id {
                            list_node.push_child(node_id);
                            renderer.register_accessibility_focus_link(
                                &crate::renderer::InteractionKey::for_rc(
                                    owner,
                                    row_interaction_base + slot,
                                ),
                                node_id,
                            );
                        }
                    }
                }
            }
            if let Some(footer) = chrome.footer {
                let inset = list_metrics.map_or(0.0, |m| f64::from(m.horizontal_inset));
                let footer_rect = kurbo::Rect::new(
                    slot_rect.x0 + inset,
                    slot_rect.y1 - footer_height,
                    slot_rect.x1 - inset,
                    slot_rect.y1,
                );
                if let Some(node_id) = register_section_chrome_node(
                    renderer,
                    ctx,
                    key_base + A11Y_KEY_FOOTER,
                    footer,
                    false,
                    None,
                    footer_rect,
                    &row_env,
                ) {
                    list_node.push_child(node_id);
                }
            }
        }
        if ctx.is_none() {
            state.end_row_frame();
        }
        let _ = renderer.register_accessibility_leaf(
            ctx,
            list_node,
            env,
            Some(AccessibilityActionTarget::Scroll {
                handle: handle.clone(),
                axis: ScrollAxis::Vertical,
            }),
        );
    }
    #[cfg(not(feature = "accessibility"))]
    {
        let _ = handle;
    }
}

/// Emits one list edit-control node — a row's delete button or one half of its
/// reorder handle — as a sibling of the row under the list node, at the bounds
/// the draw pass paints the control into (water-rs/hydrolysis#52).
///
/// The controls sit beside their row rather than inside it: under the row they
/// would be its innermost `Click` descendants, and the semantic runtime's row
/// activation resolves to exactly that child — turning "activate row" into
/// "delete row". The draw pass registers its pointer targets on the same
/// rects, so the node's `Click` and a physical tap reach the same handler.
///
/// `bounds` is `Some` only on the rendered walk — the semantic walk carries
/// no geometry — and the rendered walk must have it: emitting a control node
/// without the rect the draw pass painted would silently desynchronize the
/// two trees.
#[cfg(feature = "accessibility")]
fn register_edit_control_node<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    ctx: Option<RenderContext>,
    semantic_key: i64,
    label: String,
    bounds: Option<kurbo::Rect>,
    env: &Environment,
    action: crate::renderer::AccessibilityActivation<E>,
) -> Option<AccessibilityNodeId> {
    let mut node = AccessibilityNode::new(
        crate::renderer::SemanticCore::<E>::resolve_accessibility_role(
            env,
            AccessibilityNodeRole::Button,
        ),
    );
    node.set_label(label);
    node.add_action(AccessibilityAction::Focus);
    node.add_action(AccessibilityAction::Click);
    let target = Some(AccessibilityActionTarget::Activate { action });
    match ctx {
        Some(_ctx) => {
            let Some(bounds) = bounds else {
                contract_violation!(
                    "hydrolysis list edit-control node: the rendered walk always has list metrics; the control exposes no accessibility node"
                );
                return None;
            };
            renderer.register_accessibility_child_node_with_key(
                semantic_key,
                node,
                bounds,
                env,
                target,
            )
        }
        None => renderer.register_accessibility_child_node_with_key_semantic(
            semantic_key,
            node,
            env,
            target,
        ),
    }
}

/// Measures the scrollable viewport, using content size only for ideal queries.
pub fn measure_list_node<E: crate::text::TextEngine>(
    list: &ListRenderState<E>,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn crate::theme::WidgetTheme>,
) -> ViewDimensions {
    if let (Some(width), Some(height)) = (proposal.width, proposal.height) {
        return ViewDimensions::new(LayoutSize::new(width, height));
    }
    let contents = list.rows_snapshot.borrow().clone();
    // The first row is pinned at build and refreshed before every render;
    // rows that arrived since the last render are not realized yet, so the
    // list measures as empty this pass and the render that realizes them
    // marks it for layout again.
    let Some(first) = list.item_cache.borrow().pinned().copied() else {
        return ViewDimensions::new(LayoutSize::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ));
    };
    let metrics = list.look(theme, env).metrics();
    let content_size =
        list.measure_row_content(first, proposal.width.map(f64::from), state, theme, metrics);
    let insets = list
        .realized_rows
        .borrow()
        .get(&first)
        .and_then(|row| row.insets.clone());
    let intrinsic = list_intrinsic_for_first_row(
        &list.config,
        &contents,
        content_size,
        insets.as_ref(),
        state,
        env,
        theme,
    );
    ViewDimensions::new(LayoutSize::new(
        proposal.width.unwrap_or(intrinsic.width),
        proposal.height.unwrap_or(intrinsic.height),
    ))
}

/// Renders a retained list leaf every flush.
pub fn render_list_node<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ListRenderState<E>>>,
    env: &Environment,
) {
    #[cfg(feature = "accessibility")]
    let hidden = ctx.renderer_mut().is_accessibility_hidden(env);
    #[cfg(feature = "accessibility")]
    if hidden {
        ctx.renderer_mut().push_accessibility_suppression();
    }
    {
        let render_ctx = ctx.render_context();
        let theme = ctx.theme();
        list_accessibility(
            ctx.renderer_mut(),
            Some(render_ctx),
            Some(&theme),
            state,
            env,
        );
    }
    #[cfg(feature = "accessibility")]
    if hidden {
        ctx.renderer_mut().pop_accessibility_suppression();
    }
    render_list_parts(ctx, state, env);
}

#[expect(
    clippy::too_many_lines,
    reason = "the function drives one continuous scenario through the renderer; splitting it would obscure the sequence"
)]
pub fn render_list_parts<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ListRenderState<E>>>,
    env: &Environment,
) {
    // The retained snapshot supplies the row count and every materialization
    // below — one immutable membership for the whole render pass.
    let (editing, contents) = {
        let state_ref = state.borrow();
        (
            state_ref.config.editing().clone(),
            state_ref.rows_snapshot.borrow().clone(),
        )
    };
    let editing = ctx.renderer_mut().read_signal(&editing);
    let row_count = contents.len();
    let look = state.borrow().look(&ctx.theme(), env);
    let list_metrics = look.metrics();
    let style_geom = state.borrow().style_geometry();
    let min_row_height = state.borrow().effective_min_row_height();
    // `min_row_height` is the floor every row is estimated and measured
    // against; unset, the style's or theme's one-line height applies.
    let row_floor =
        f64::from(min_row_height.unwrap_or(list_metrics.one_line_row_height)) + style_geom.row_gap;
    // A 0 floor still needs a positive seed estimate — measured extents
    // replace it row by row anyway.
    let row_estimate = row_floor.max(1.0);
    if state.borrow().refresh_first_row(env, ctx.renderer_mut()) {
        ctx.renderer_mut().context_mark_layout();
    }
    let frame_instant = ctx.renderer_mut().frame_instant();
    state.borrow().advance_transitions(ctx.renderer_mut());
    let sections_changed = state.borrow().resolve_sections();
    let layout_changed = state
        .borrow()
        .sync_visible_layout(row_count, ctx.renderer_mut());
    state
        .borrow()
        .prepare_rows(row_count, row_estimate, frame_instant);
    if sections_changed || layout_changed {
        // Row extents measured before section chrome or collapsed state changed
        // are stale, so drop them rather than drawing into stale slots.
        state.borrow().reset_extents(row_count, row_estimate);
        if layout_changed {
            ctx.renderer_mut().context_mark_layout();
        }
    }

    let viewport = ctx.bounds;
    // §7.1's scroll surface: the clip, the wheel target and the visible
    // window all run on the frame grown by the extension layout computed —
    // the surface paints through the bands it touched. The row math stays
    // anchored on the laid-out frame.
    let surface_viewport = crate::renderer::grow_rect(viewport, state.borrow().surface.extension());
    let content_height = state
        .borrow()
        .extent_index
        .borrow()
        .total_extent()
        .max(viewport.height());
    let position_report =
        state
            .borrow()
            .position_report(viewport.width(), viewport.height(), content_height);
    let handle = state.borrow().bind_scroll(
        viewport.width(),
        viewport.height(),
        content_height,
        position_report,
    );
    // Applied ahead of the rows' recording, so a move it makes is in this
    // frame's recording already.
    let _ = state
        .borrow()
        .apply_scroll_request(ctx.renderer_mut(), &handle, row_count, true);
    // The keyboard-moving clearance runs before the rows paint: while the
    // host's keyboard animation is in flight the offset follows it frame by
    // frame, so this flush paints the field already clear.
    #[cfg(any(feature = "text-field", feature = "secure-field"))]
    state
        .borrow()
        .surface
        .begin_flush(ctx.renderer_mut(), &handle);
    let mut metrics = handle.metrics();
    // Register before the rows flush: scroll-target dispatch walks the frame's
    // targets newest-first, so a scroll region inside a row wins the delta
    // until it hits its own edge, where it falls through to the list.
    crate::widgets::scroll::register_scroll_wheel_target(
        ctx.renderer_mut(),
        surface_viewport,
        &handle,
    );

    let span = state
        .borrow()
        .surface
        .visible_span(&metrics, ScrollAxis::Vertical);
    let span_horizontal = state
        .borrow()
        .surface
        .visible_span(&metrics, ScrollAxis::Horizontal);
    // The rows scroll as the widget's inner layer `ScrollOffset`, exactly as a
    // `scroll` node's content does (decision 1): they paint and place at their
    // resting positions in content space and the inner layer shifts them, so a
    // scroll frame writes the one scroll-offset property instead of re-placing
    // every visible row. The viewport placement carries the same offset so the
    // rows' hit regions and emitted bounds follow it without a re-record.
    let offset = kurbo::Vec2::new(metrics.offset_x, metrics.offset_y);
    {
        let renderer = ctx.renderer_mut();
        renderer.push_placement_scope(
            crate::renderer::mount::ScopeDelta::RECORD_SPACE,
            Some(surface_viewport),
        );
        let viewport_placement = renderer.current_placement();
        viewport_placement.set_content_offset(-offset);
        let stretch = renderer.scroll_stretch(&handle);
        renderer.program().begin_inner(
            surface_viewport,
            offset,
            stretch,
            std::rc::Rc::clone(&viewport_placement),
        );
        // The visible window resolves in content space while the rows record
        // at `viewport.origin + content`: the content's local transform is the
        // frame's origin — the relation `ScrollNode` publishes through
        // `content_ctx.local` — so the lazy window maps content coordinates
        // through it before the scroll shift.
        let world = renderer.record_world(kurbo::Affine::translate(kurbo::Vec2::new(
            viewport.x0,
            viewport.y0,
        )));
        renderer.push_lazy_viewport(crate::renderer::LazyViewport {
            bounds: kurbo::Rect::new(
                span_horizontal.start,
                span.start,
                span_horizontal.end,
                span.end,
            ),
            transform: world * kurbo::Affine::translate(-offset),
        });
    }

    let window = state
        .borrow()
        .extent_index
        .borrow()
        .visible_window(span.start, span.end);
    // One hit-test group for every row gesture in this list, so a swipe on one
    // row and a reorder drag on another can never both claim the same pointer.
    let gesture_group = ctx.renderer_mut().allocate_gesture_group_id();
    // Begin a fresh frame for the per-row content sub-view cache: only rows touched
    // in the visible loop below survive `end_frame`, preserving virtualization.
    state.borrow().item_cache.borrow_mut().begin_frame();
    // Resting geometry for the whole visible window is resolved before anything
    // is painted. A lifted row has to be drawn last so it floats over the rows
    // it travels past, so draw order can no longer be the order the vertical
    // cursor advances in.
    let mut rows = Vec::with_capacity(window.end.saturating_sub(window.start));
    // Rows live in content space: `leading_offset` is the window's resting
    // position under the viewport's top edge, and the inner layer's scroll
    // offset shifts the whole band on screen.
    let mut y = viewport.y0 + window.leading_offset;
    // A row's extent is derived from its content's measured size every frame —
    // the same transient measure `list_content_rect` consumes below — so a row
    // whose content re-measures differently is re-measured here and only here:
    // `set_measured` writes the identical extent back for unchanged rows and
    // rows outside the window are never touched (water-rs/hydrolysis#199).
    let mut extents_changed = false;
    // Removal ghosts in the window, with the content-space top of each slot.
    let mut ghosts = Vec::new();
    // Collapsed section headers in the window: `(section_index, first_row_id, y, height)`.
    let mut collapsed_headers = Vec::new();
    let mut position = window.start;
    let mut end = window.end;
    loop {
        while position < end {
            // The window runs over display positions: removal ghosts hold
            // frozen slots and collapsed sections appear as header-only slots.
            let Some(entry) = state.borrow().display_entry_at(position) else {
                position += 1;
                continue;
            };
            let index = match entry {
                VisibleEntry::Row(index) => index,
                VisibleEntry::Ghost(ghost) => {
                    let height = f64::from(state.borrow().transitions.ghost(ghost).size().height);
                    ghosts.push((ghost, y));
                    y += height;
                    position += 1;
                    continue;
                }
                VisibleEntry::CollapsedHeader {
                    section_index,
                    first_row_index,
                } => {
                    let Some(first_row_id) = contents.get_id(first_row_index) else {
                        position += 1;
                        continue;
                    };
                    let starts_later = section_index > 0;
                    let slot_height = if starts_later {
                        style_geom.inter_section_gap
                    } else {
                        0.0
                    } + f64::from(list_metrics.section_header_height);
                    {
                        let state_ref = state.borrow();
                        let mut extent_index = state_ref.extent_index.borrow_mut();
                        extents_changed |= extent_index
                            .measured(position)
                            .is_none_or(|old| old.to_bits() != slot_height.to_bits());
                        extent_index.set_measured(position, slot_height);
                    }
                    collapsed_headers.push((section_index, first_row_id, y, slot_height));
                    y += slot_height;
                    position += 1;
                    continue;
                }
            };
            let Some(row_id) = contents.get_id(index) else {
                contract_violation!(
                    "hydrolysis List item {index} has no stable id although it is in the snapshot's range; the row is not drawn"
                );
                position += 1;
                continue;
            };
            state
                .borrow()
                .realize_row(&contents, index, row_id, env, ctx.renderer_mut());
            let chrome = state.borrow().section_chrome(index);
            // A row's extent covers the section chrome it owns, so scroll offsets,
            // hit testing, and the visible window all account for it.
            let content_size = state.borrow().patch_and_measure_row_content(
                row_id,
                Some(viewport.width()),
                ctx.renderer_mut(),
                list_metrics,
            );
            let row_insets = state.borrow().realized_rows.borrow()[&row_id]
                .insets
                .clone();
            let row_height = list_row_height_for_content(
                f64::from(content_size.height),
                row_insets.as_ref(),
                min_row_height,
                list_metrics,
            ) + style_geom.row_gap
                + chrome.total_height(&list_metrics, style_geom);
            {
                let state_ref = state.borrow();
                let mut extent_index = state_ref.extent_index.borrow_mut();
                extents_changed |= extent_index
                    .measured(position)
                    .is_none_or(|old| old.to_bits() != row_height.to_bits());
                extent_index.set_measured(position, row_height);
            }
            rows.push((index, row_id, y, row_height, content_size));
            y += row_height;
            position += 1;
        }
        // A re-measured row can pull the window's end either way: shrinkage
        // reveals rows the stale extents hid, and those rows must be resolved
        // into this frame's stack rather than leaving the viewport's tail
        // unpainted until the next refresh.
        let span = state
            .borrow()
            .surface
            .visible_span(&metrics, ScrollAxis::Vertical);
        let refreshed_end = state
            .borrow()
            .extent_index
            .borrow()
            .visible_window(span.start, span.end)
            .end;
        if refreshed_end <= end {
            break;
        }
        end = refreshed_end;
    }
    if extents_changed {
        // Everything downstream of this pass that reads extents — the
        // accessibility emit that ran before it, the scroll domain, the
        // indicators — was resolved against the stale values, so pull one
        // more frame rather than leaving them stale until an unrelated
        // refresh happens to arrive.
        ctx.renderer_mut().context_mark_layout();
    }
    // The lifted row paints last, over the rows it travels past.
    let lifted_id = {
        let state_ref = state.borrow();
        let gestures = state_ref.row_gestures.borrow();
        rows.iter()
            .map(|(_, id, ..)| *id)
            .find(|id| gestures.get(id).is_some_and(|row| row.motion.lifted()))
    };
    if let Some(lifted_id) = lifted_id
        && let Some(position) = rows.iter().position(|(_, id, ..)| *id == lifted_id)
    {
        let lifted_row = rows.remove(position);
        rows.push(lifted_row);
    }
    let visible_ids: Vec<ListItemId> = rows.iter().map(|(_, id, ..)| *id).collect();
    // A scope's row is gone once the visible window moves past it, the same
    // lifetime `item_cache`'s `end_frame` gives the sub-view that carried it.
    #[cfg(feature = "accessibility")]
    state
        .borrow()
        .semantics_scopes
        .borrow_mut()
        .retain(|id, _| visible_ids.contains(id));

    if matches!(
        style_geom.surface,
        ListSurfaceKind::Grouped | ListSurfaceKind::InsetGrouped
    ) {
        let backdrop = ctx
            .renderer_mut()
            .read_signal(&color::Background.resolve(env).computed());
        ctx.draw_context(|draw| {
            draw.fill(surface_viewport, backdrop);
        });
    }

    // Ghosts paint first, beneath the live rows.
    for (ghost, resting_y) in ghosts {
        draw_list_ghost(ctx, state, env, ghost, resting_y);
    }

    let mut visible_header_ids: Vec<ListItemId> = Vec::new();
    let outer_inset = style_geom.outer_horizontal_inset;
    let header_horizontal_inset = f64::from(list_metrics.horizontal_inset) + outer_inset;
    for (section_index, first_row_id, resting_y, slot_height) in collapsed_headers {
        let slot_rect =
            kurbo::Rect::new(viewport.x0, resting_y, viewport.x1, resting_y + slot_height);
        if slot_rect.y1 <= span.start + viewport.y0 || slot_rect.y0 >= span.end + viewport.y0 {
            continue;
        }
        let Some((header, expanded, starts_later)) =
            state.borrow().collapsed_section_header(section_index)
        else {
            continue;
        };
        let top_gap = if starts_later {
            style_geom.inter_section_gap
        } else {
            0.0
        };
        let header_height = f64::from(list_metrics.section_header_height);
        let hit_rect = kurbo::Rect::new(
            slot_rect.x0 + outer_inset,
            slot_rect.y0 + top_gap,
            (slot_rect.x1 - outer_inset).max(slot_rect.x0 + outer_inset),
            slot_rect.y0 + top_gap + header_height,
        );
        let content_rect = kurbo::Rect::new(
            slot_rect.x0 + header_horizontal_inset,
            slot_rect.y0 + top_gap,
            (slot_rect.x1 - header_horizontal_inset).max(slot_rect.x0 + header_horizontal_inset),
            slot_rect.y0 + top_gap + header_height,
        );
        if expanded.is_some() {
            visible_header_ids.push(first_row_id);
        }
        draw_section_header(
            ctx,
            state,
            first_row_id,
            header,
            expanded,
            hit_rect,
            content_rect,
            env,
        );
    }

    for (index, row_id, resting_y, row_height, content_size) in rows {
        // The row's environment carries the naming its content declared,
        // hoisted when the row was realized; a selectable row also carries its
        // selection as `Selected`, so the row's press target reports SELECTED
        // — claims are owner-scoped, so nested controls inside the row do not
        // pick it up.
        let (row_env, row_deletable, row_insets, row_disabled_signal, subtree_env) = {
            let state_ref = state.borrow();
            let rows = state_ref.realized_rows.borrow();
            let row = &rows[&row_id];
            let mut row_env = row.env.clone();
            if let Some(selection) = state_ref
                .row_selection
                .as_ref()
                .filter(|selection| selection.selects(row_id))
            {
                row_env.insert(Selected(selection.is_selected(row_id)));
            }
            (
                row_env,
                row.deletable.clone(),
                row.insets.clone(),
                row.disabled.clone(),
                row.subtree_env.clone(),
            )
        };
        let row_interaction_base = row_interaction_base(row_id);
        let chrome = state.borrow().section_chrome(index);
        // Where the row's slot is (the gap it occupies in the list): a swipe
        // or a reorder drag displaces the row's layer live and never moves
        // the slot, so everything below records at rest.
        let slot_rect =
            kurbo::Rect::new(viewport.x0, resting_y, viewport.x1, resting_y + row_height);
        // `slot_rect` rides in the frame's space (`viewport.origin + content`);
        // shift the content-space span into the same space for the cull.
        if slot_rect.y1 <= span.start + viewport.y0 || slot_rect.y0 >= span.end + viewport.y0 {
            continue;
        }
        let top_gap = chrome.top_gap(style_geom);
        let header_height = chrome.header_height(&list_metrics);
        let footer_height = chrome.footer_height(&list_metrics);
        // The slot covers the section chrome this row owns; the row itself is
        // the band left between that header and footer, and only that band
        // swipes — a section title is not part of the row that carries it.
        let row_slot = kurbo::Rect::new(
            slot_rect.x0,
            slot_rect.y0 + top_gap + header_height,
            slot_rect.x1,
            slot_rect.y1 - footer_height,
        );
        let outer_inset = style_geom.outer_horizontal_inset;
        let row_rect = kurbo::Rect::new(
            row_slot.x0 + outer_inset,
            row_slot.y0,
            (row_slot.x1 - outer_inset).max(row_slot.x0 + outer_inset),
            (row_slot.y1 - style_geom.row_gap).max(row_slot.y0),
        );
        let (row_can_delete, row_movable_span) = {
            let state_ref = state.borrow();
            let edits = state_ref.config.edits();
            (
                edits.can_delete(row_id),
                edits.movable_span(&contents, index),
            )
        };
        let (movable_start, movable_end) = row_movable_span
            .as_ref()
            .map_or((index, index), |span| (span.start, span.end));
        // Refreshed before either recognizer runs this frame, so an in-flight
        // drag always sees the row's current index, size, and part bounds.
        let (row_binding, motion) = refresh_row_binding(
            state,
            row_id,
            RowBinding {
                index,
                width: row_rect.width(),
                height: row_height,
                movable_start,
                movable_end,
            },
        );
        // The background a swipe uncovers stays anchored to the slot, beneath
        // the row, from the swipe's first frame: its one value is the swipe's
        // progress, a live operand the swipe moves with no re-record.
        if motion.swiping()
            && let Some(part) = look.dismiss_part()
        {
            let source = motion.dismiss_source(part, row_rect);
            let render_ctx = ctx.render_context();
            ctx.renderer_mut().push_live(render_ctx, &source);
        }
        let horizontal_inset = f64::from(list_metrics.horizontal_inset) + outer_inset;
        if let Some(header) = chrome.header.clone() {
            let header_hit_rect = kurbo::Rect::new(
                slot_rect.x0 + outer_inset,
                slot_rect.y0 + top_gap,
                (slot_rect.x1 - outer_inset).max(slot_rect.x0 + outer_inset),
                slot_rect.y0 + top_gap + header_height,
            );
            let header_rect = kurbo::Rect::new(
                slot_rect.x0 + horizontal_inset,
                slot_rect.y0 + top_gap,
                (slot_rect.x1 - horizontal_inset).max(slot_rect.x0 + horizontal_inset),
                slot_rect.y0 + top_gap + header_height,
            );
            if chrome.header_expanded.is_some() {
                visible_header_ids.push(row_id);
            }
            draw_section_header(
                ctx,
                state,
                row_id,
                header,
                chrome.header_expanded.clone(),
                header_hit_rect,
                header_rect,
                &row_env,
            );
        }
        if let Some(footer) = chrome.footer.clone() {
            let footer_rect = kurbo::Rect::new(
                slot_rect.x0 + horizontal_inset,
                slot_rect.y1 - footer_height,
                (slot_rect.x1 - horizontal_inset).max(slot_rect.x0 + horizontal_inset),
                slot_rect.y1,
            );
            draw_section_label(ctx, footer, footer_rect, false, &row_env);
        }
        // Everything the row draws — its background, controls and content —
        // is one layer that follows the row's displacement operand; the
        // section chrome above and the dismiss background stay in the slot.
        // The layer's clip reaches as far as the list's own height beyond the
        // slot, so a lifted row's shadow is not cut at the row's edge.
        ctx.open_live_scope(
            crate::renderer::mount::ScopeKey {
                role: "list-row",
                item: u64::from(crate::num_cast::i32_as_u32(i32::from(row_id))),
            },
            ScopeOperands {
                opacity: None,
                transform: Some(motion.layer()),
            },
            kurbo::Rect::new(
                surface_viewport.x0,
                slot_rect.y0 - viewport.height(),
                surface_viewport.x1,
                slot_rect.y1 + viewport.height(),
            ),
        );
        let selected = state
            .borrow()
            .row_selection
            .as_ref()
            .is_some_and(|selection| {
                ctx.renderer_mut()
                    .read_signal(&selection.is_selected(row_id))
            });
        // Selection fill: sidebar lists paint `UnemphasizedSelection` so row
        // text keeps its own foreground colours (`water-rs/waterui#1495`);
        // other styles paint `SelectionContainer` paired with
        // `SelectionForeground` from `selection_themed`.
        let selection_fill = selected.then(|| {
            if style_geom.surface == ListSurfaceKind::Sidebar {
                ctx.renderer_mut()
                    .read_signal(&color::UnemphasizedSelection.resolve(&row_env).computed())
            } else {
                ctx.renderer_mut()
                    .read_signal(&color::SelectionContainer.resolve(&row_env).computed())
            }
        });
        let card_surface = (style_geom.surface == ListSurfaceKind::InsetGrouped).then(|| {
            ctx.renderer_mut()
                .read_signal(&color::Surface.resolve(&row_env).computed())
        });
        let grouped_border = (style_geom.surface == ListSurfaceKind::Grouped).then(|| {
            ctx.renderer_mut()
                .read_signal(&color::Border.resolve(&row_env).computed())
        });
        ctx.draw_context(|draw| {
            match style_geom.surface {
                ListSurfaceKind::InsetGrouped => {
                    let radii = chrome.card_radii(style_geom.corner_radius);
                    let shape = kurbo::RoundedRect::from_rect(row_rect, radii);
                    if let Some(surface) = card_surface {
                        draw.fill(shape, surface);
                    }
                    if let Some(fill) = selection_fill {
                        draw.fill(shape, fill);
                    }
                }
                ListSurfaceKind::Sidebar => {
                    if let Some(fill) = selection_fill {
                        let pill =
                            kurbo::RoundedRect::from_rect(row_rect, style_geom.corner_radius);
                        draw.fill(pill, fill);
                    }
                }
                ListSurfaceKind::Plain | ListSurfaceKind::Grouped => {
                    look.record_row(&mut *draw, row_rect, index);
                    if let Some(fill) = selection_fill {
                        if style_geom.corner_radius > 0.0 {
                            let rounded =
                                kurbo::RoundedRect::from_rect(row_rect, style_geom.corner_radius);
                            draw.fill(rounded, fill);
                        } else {
                            draw.fill(row_rect, fill);
                        }
                    }
                    if style_geom.surface == ListSurfaceKind::Grouped
                        && let Some(border) = grouped_border
                    {
                        if chrome.is_section_first {
                            draw.fill(
                                kurbo::Rect::new(
                                    row_rect.x0,
                                    row_rect.y0,
                                    row_rect.x1,
                                    row_rect.y0 + 1.0,
                                ),
                                border,
                            );
                        }
                        if chrome.is_section_last {
                            draw.fill(
                                kurbo::Rect::new(
                                    row_rect.x0,
                                    row_rect.y1 - 1.0,
                                    row_rect.x1,
                                    row_rect.y1,
                                ),
                                border,
                            );
                        }
                    }
                }
            }
            if motion.lifted() {
                look.record_lifted_row(&mut *draw, row_rect);
            }
        });

        let deletable = row_can_delete && ctx.renderer_mut().read_signal(&row_deletable);
        // The content's measured size was resolved in the resting-geometry pass
        // above — the same measure that wrote the row's extent this frame.
        let mut content_rect = list_content_rect(
            row_rect,
            list_metrics,
            row_insets.as_ref(),
            content_size,
            &row_env,
        );
        // Edit mode's trailing controls — the same computation the
        // accessibility emit uses to place their nodes.
        let controls = row_edit_controls(
            &list_metrics,
            row_rect,
            row_height,
            index,
            editing.then(|| row_movable_span.clone()).flatten(),
            editing && deletable,
        );

        // A selectable row owns the whole row rect as a press target: a
        // plain click selects it, the toggle modifier toggles it and Shift
        // extends the anchored range — while the controls and the content
        // flushed after it still take their own presses first. The press
        // belongs to the row's own view — the content sub-view's root — so the
        // ancestry check tells a gesture or tap registered inside the row (a
        // nested `on_tap`) from one attached to that same root: a descendant's
        // claims the press, the row's own handler coexists with it
        // (water-rs/hydrolysis#175).
        // The selection press honours the `Disabled` scope the row's content
        // resolves under — the same gate the content's own presses read — so
        // a `.disabled` row stays inert to pointer input.
        let row_disabled = ctx.renderer_mut().read_signal(&row_disabled_signal);
        if let Some(selection) = state.borrow().row_selection.clone()
            && !row_disabled
            && selection.selects(row_id)
        {
            let hit_bounds = row_rect;
            let key = crate::renderer::InteractionKey::for_rc(state, row_interaction_base + 3);
            let (_, press_slot, _) = ctx
                .renderer_mut()
                .bind_interaction_target(key, hit_bounds, &row_env);
            let row_owner = {
                let state_ref = state.borrow();
                let cache = state_ref.item_cache.borrow();
                cache
                    .get(&row_id)
                    .expect("hydrolysis List: a drawn row is realized")
                    .root_accessibility_identity()
            };
            if let Some(owner) = row_owner.as_ref() {
                ctx.renderer_mut().push_input_owner(owner);
            }
            ctx.renderer_mut().register_interactive_pointer_target(
                hit_bounds,
                press_slot,
                move |renderer: &mut crate::renderer::SemanticCore<E>, _point, _env| {
                    selection.write(index, row_id, renderer.modifiers());
                    true
                },
            );
            if row_owner.is_some() {
                ctx.renderer_mut().pop_input_owner();
            }
        }

        // Swipe-to-dismiss covers the whole row and is available whenever this
        // row can be deleted, matching Material's `SwipeToDismissBox` rather
        // than being gated behind edit mode.
        if deletable {
            register_row_swipe_gesture(
                ctx,
                state,
                gesture_group,
                row_id,
                RowDrag {
                    binding: Rc::clone(&row_binding),
                    motion: Rc::clone(&motion),
                    slop: look.gestures().slop,
                },
                slot_rect,
                &row_env,
            );
        }

        if let Some(control_rect) = controls.reorder
            && let Some(span) = row_movable_span
        {
            // The handle is also the reorder grip: dragging it lifts the row.
            // The tap targets below stay, so the same control still offers
            // discrete one-step moves for pointer and keyboard users.
            register_row_reorder_gesture(
                ctx,
                state,
                gesture_group,
                row_id,
                RowDrag {
                    binding: Rc::clone(&row_binding),
                    motion: Rc::clone(&motion),
                    slop: look.gestures().slop,
                },
                control_rect,
                &row_env,
            );
            if let Some(rect) = controls.reorder_up
                && let Some(target_id) = contents.get_id(index - 1)
            {
                let key = crate::renderer::InteractionKey::for_rc(state, row_interaction_base);
                let (hit_bounds, press_slot) = realize_control(
                    ctx,
                    state,
                    &ControlSlot {
                        row_id,
                        key: &key,
                        kind: EditControl::MoveUp,
                    },
                    look.move_up(),
                    rect,
                    &row_env,
                );
                let state = Rc::clone(state);
                let action_env = row_env.clone();
                let from_in_part = index - span.start;
                let to_in_part = from_in_part - 1;
                ctx.renderer_mut().register_interactive_pointer_target(
                    hit_bounds,
                    press_slot,
                    move |_renderer, _point, _env| {
                        run_row_move(
                            &state,
                            &action_env,
                            row_id,
                            target_id,
                            from_in_part,
                            to_in_part,
                        )
                    },
                );
            }
            if let Some(rect) = controls.reorder_down
                && let Some(target_id) = contents.get_id(index + 1)
            {
                let key = crate::renderer::InteractionKey::for_rc(state, row_interaction_base + 1);
                let (hit_bounds, press_slot) = realize_control(
                    ctx,
                    state,
                    &ControlSlot {
                        row_id,
                        key: &key,
                        kind: EditControl::MoveDown,
                    },
                    look.move_down(),
                    rect,
                    &row_env,
                );
                let state = Rc::clone(state);
                let action_env = row_env.clone();
                let from_in_part = index - span.start;
                let to_in_part = from_in_part + 1;
                ctx.renderer_mut().register_interactive_pointer_target(
                    hit_bounds,
                    press_slot,
                    move |_renderer, _point, _env| {
                        run_row_move(
                            &state,
                            &action_env,
                            row_id,
                            target_id,
                            from_in_part,
                            to_in_part,
                        )
                    },
                );
            }
        }

        if let Some(delete_rect) = controls.delete {
            let delete_key =
                crate::renderer::InteractionKey::for_rc(state, row_interaction_base + 2);
            let (delete_hit_bounds, delete_press_slot) = realize_control(
                ctx,
                state,
                &ControlSlot {
                    row_id,
                    key: &delete_key,
                    kind: EditControl::Delete,
                },
                look.delete(),
                delete_rect,
                &row_env,
            );
            let state = Rc::clone(state);
            let action_env = row_env.clone();
            ctx.renderer_mut().register_interactive_pointer_target(
                delete_hit_bounds,
                delete_press_slot,
                move |_renderer, _point, _env| run_row_delete(&state, &action_env, row_id),
            );
        }

        content_rect.x1 = content_rect.x1.min(controls.trailing_x);
        if content_rect.width() > 0.0 && content_rect.height() > 0.0 {
            // Render the row content through a persistent node held in the per-widget
            // cache, keyed by stable row id, instead of re-dispatching it each frame.
            // The cache keeps a row's node only while it stays visible (built on first
            // appearance, evicted by `end_frame` once it scrolls out), so reactive row
            // content stays live across frames while virtualization is preserved. The
            // sub-view's own a11y emits *inside* the row: `list_accessibility` emitted
            // the row's `ListItem` node this frame and linked it to this row's
            // interaction key — parenting the flush under that node keeps every text,
            // control and image in the row a descendant of its `ListItem`. When the
            // row emitted no node (a hidden or otherwise suppressed row), the subtree
            // suppresses the same way the old flush-wide suppression did.
            #[cfg(feature = "accessibility")]
            let row_node_id =
                ctx.renderer_mut()
                    .focus_node_for_key(&crate::renderer::InteractionKey::for_rc(
                        state,
                        row_interaction_base + 4,
                    ));
            #[cfg(feature = "accessibility")]
            let row_parented = {
                if let Some(row_node_id) = row_node_id {
                    ctx.renderer_mut().push_accessibility_parent(row_node_id);
                    true
                } else {
                    ctx.renderer_mut().push_accessibility_suppression();
                    false
                }
            };
            let render_ctx = ctx.render_context();
            {
                let state_ref = state.borrow();
                let mut cache = state_ref.item_cache.borrow_mut();
                let subview = cache
                    .get_mut(&row_id)
                    .expect("hydrolysis List: a drawn row is realized");
                let proposal = bounded_proposal(content_rect);
                state_ref.transitions.place(
                    row_id,
                    crate::renderer::RowPlacement {
                        slot: slot_rect,
                        content: content_rect - slot_rect.origin().to_vec2(),
                        proposal,
                        stretch: subview.stretch_axis(),
                    },
                );
                subview.set_content_anchor(row_content_anchor(&row_env));
                let row = state_ref.transitions.row(row_id);
                match &row.insertion {
                    Some(insertion) => {
                        let owner = state_ref
                            .owner
                            .upgrade()
                            .expect("hydrolysis List: a flushing list's cell is alive");
                        let paint = crate::renderer::TransitionPaint::sample(
                            &insertion.state,
                            kurbo_logical_size(content_rect),
                            state_ref.transitions.direction,
                            true,
                            &insertion.runtime,
                            &owner,
                        );
                        let delta = kurbo::Affine::translate((content_rect.x0, content_rect.y0));
                        let child_ctx = render_ctx.child(
                            delta,
                            kurbo::Rect::from_origin_size(kurbo::Point::ZERO, content_rect.size()),
                        );
                        crate::renderer::flush_transitioned(
                            ctx.renderer_mut(),
                            child_ctx,
                            delta,
                            &paint,
                            |renderer, inner| {
                                subview.place_in_ctx(
                                    renderer,
                                    inner,
                                    &subtree_env,
                                    proposal,
                                    kurbo_logical_size(content_rect),
                                    None,
                                    kurbo::Affine::IDENTITY,
                                );
                            },
                        );
                    }
                    // A list row is scroll content: it lays out with no
                    // §7.1 context (its surface owns the edges).
                    None => subview.place(
                        ctx.renderer_mut(),
                        render_ctx,
                        &subtree_env,
                        proposal,
                        content_rect,
                        None,
                    ),
                }
            }
            #[cfg(feature = "accessibility")]
            if row_parented {
                ctx.renderer_mut().pop_accessibility_parent();
                ctx.renderer_mut().drain_claim_scope(
                    row_node_id.expect("a parented list row always has a registered node"),
                    &subtree_env,
                );
            } else {
                ctx.renderer_mut().pop_accessibility_suppression();
            }
        }

        let draw_separator = match style_geom.separators {
            ListSeparatorMode::All => true,
            ListSeparatorMode::WithinSection => !chrome.is_section_last,
            ListSeparatorMode::None => false,
        };
        if draw_separator {
            let separator = kurbo::Rect::new(
                row_rect.x0 + f64::from(list_metrics.divider_leading_inset),
                row_rect.y1 - 1.0,
                row_rect.x1 - f64::from(list_metrics.divider_trailing_inset),
                row_rect.y1,
            );
            ctx.draw_context(|draw| {
                look.record_separator(&mut *draw, separator);
            });
        }
        ctx.close_scope();
    }
    // Evict content sub-views for rows no longer in the visible window, with
    // their transition state.
    {
        state.borrow().end_row_frame();
    }
    // Retained gesture recognizers follow the same virtualization rule: a row
    // that scrolled out has no in-flight gesture worth remembering, and keeping
    // one per row would defeat the point of a 100k-row lazy list.
    state
        .borrow()
        .row_gestures
        .borrow_mut()
        .retain(|id, _| visible_ids.contains(id));
    state
        .borrow()
        .section_toggles
        .borrow_mut()
        .retain(|id, _| visible_header_ids.contains(id));

    if state
        .borrow()
        .pending_scroll
        .borrow()
        .as_ref()
        .is_some_and(|pending| pending.cached_index.is_some_and(|index| index < row_count))
    {
        let content_height = state
            .borrow()
            .extent_index
            .borrow()
            .total_extent()
            .max(viewport.height());
        let position_report =
            state
                .borrow()
                .position_report(viewport.width(), viewport.height(), content_height);
        let rebound = state.borrow().bind_scroll(
            viewport.width(),
            viewport.height(),
            content_height,
            position_report,
        );
        let moved =
            state
                .borrow()
                .apply_scroll_request(ctx.renderer_mut(), &rebound, row_count, true);
        metrics = rebound.metrics();
        // The rows above recorded, and the semantic pass emitted, the offset
        // this application started from: a request that moved it leaves both
        // stale, so it pulls one more frame. A run it merely refined moves
        // nothing here — its ticks carry the offset as continuation work, and
        // marking every frame of the glide would read the whole flight as an
        // unapplied change.
        if moved {
            ctx.renderer_mut().context_mark_layout();
        }
    }
    state.borrow().record_viewport_anchor(metrics, row_count);

    {
        let renderer = ctx.renderer_mut();
        renderer.pop_lazy_viewport("hydrolysis list");
        renderer.program().end_inner();
        renderer.pop_placement_scope();
    }

    // The focused-field clearance runs after recording, over the retained
    // input targets (§7.1).
    #[cfg(any(feature = "text-field", feature = "secure-field"))]
    state
        .borrow()
        .surface
        .register_clearance(ctx.renderer_mut(), &handle);

    draw_scroll_indicators(
        ctx,
        env,
        viewport,
        metrics,
        ScrollAxis::Vertical,
        &handle,
        state.borrow().surface.extension(),
    );
}

/// Ensures row `row_id` has a retained gesture binding and refreshes it with
/// this frame's geometry.
///
/// The binding is what lets a recognizer outlive the frame that built it: the
/// closures read the row's *current* index and size through this cell, so a
/// drag still in flight after a reorder or a deletion acts on where the row is
/// now rather than on the index captured when the recognizer was created.
fn refresh_row_binding<E: crate::text::TextEngine>(
    state: &Rc<RefCell<ListRenderState<E>>>,
    row_id: ListItemId,
    binding: RowBinding,
) -> (Rc<Cell<RowBinding>>, Rc<RowMotion>) {
    let state_ref = state.borrow();
    let mut gestures = state_ref.row_gestures.borrow_mut();
    let row = gestures.entry(row_id).or_insert_with(RowGestures::empty);
    row.binding.set(binding);
    (Rc::clone(&row.binding), Rc::clone(&row.motion))
}

/// What a row's drag recognizer reads: the row's live geometry, what it
/// moves live and the distance the pointer travels before a drag begins.
struct RowDrag {
    binding: Rc<Cell<RowBinding>>,
    motion: Rc<RowMotion>,
    slop: f32,
}

/// One of a row's edit controls: which chrome of its row it realizes.
#[derive(Clone, Copy)]
enum EditControl {
    MoveUp,
    MoveDown,
    Delete,
}

/// Which control of which row [`realize_control`] realizes, and the
/// interaction key its chrome binds under.
#[derive(Clone, Copy)]
struct ControlSlot<'a> {
    row_id: ListItemId,
    key: &'a crate::renderer::InteractionKey,
    kind: EditControl,
}

/// Binds the interaction target of an edit control's chrome on `bounds`,
/// realizes the chrome the row keeps for it and lists its layers. Returns the
/// control's touch target and press slot, for its tap target.
fn realize_control<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ListRenderState<E>>>,
    control: &ControlSlot<'_>,
    chrome: &Chrome,
    bounds: kurbo::Rect,
    env: &Environment,
) -> (kurbo::Rect, crate::renderer::PressSlot) {
    let ControlSlot { row_id, key, kind } = *control;
    let target = chrome.target(bounds);
    let (_, press_slot, handles) =
        ctx.renderer_mut()
            .bind_live_chrome_target(key.clone(), target, env, false, chrome);
    let flags = ctx.renderer_mut().reported_interaction_state(key);
    let parts = {
        let state_ref = state.borrow();
        let mut gestures = state_ref.row_gestures.borrow_mut();
        let row = gestures.entry(row_id).or_insert_with(RowGestures::empty);
        let kept = match kind {
            EditControl::MoveUp => &mut row.controls.move_up,
            EditControl::MoveDown => &mut row.controls.move_down,
            EditControl::Delete => &mut row.controls.delete,
        };
        LiveChromeParts::ensure(
            &mut kept.parts,
            chrome,
            &ChromeContent::default(),
            bounds,
            flags,
            env,
            &kept.report,
        )
        .clone()
    };
    handles.attach_live(Rc::clone(parts.chrome()));
    let render_ctx = ctx.render_context();
    parts.push_below(ctx.renderer_mut(), render_ctx);
    parts.push_above(ctx.renderer_mut(), render_ctx);
    (target, press_slot)
}

/// Registers `gesture` for one row, reusing the recognizer retained from an
/// earlier frame when there is one.
///
/// Building a fresh recognizer every frame would reset the state machine mid-
/// drag, so a swipe would stall the moment the list re-rendered. `slot` names
/// which of the row's two gestures this is.
fn register_row_gesture<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ListRenderState<E>>>,
    group: usize,
    row_id: ListItemId,
    bounds: kurbo::Rect,
    slot: RowGestureSlot,
    build: impl FnOnce() -> (Gesture, BoxedAction<()>),
) {
    let hit_bounds = bounds;
    let retained = {
        let state_ref = state.borrow();
        let gestures = state_ref.row_gestures.borrow();
        gestures.get(&row_id).and_then(|row| slot.get(row).cloned())
    };
    if let Some(target) = retained {
        ctx.renderer_mut()
            .register_retained_gesture_target(&target, hit_bounds, group);
        return;
    }
    let (gesture, action) = build();
    let target = ctx
        .renderer_mut()
        .register_gesture_target(hit_bounds, group, gesture, action);
    let state_ref = state.borrow();
    let mut gestures = state_ref.row_gestures.borrow_mut();
    let row = gestures.entry(row_id).or_insert_with(RowGestures::empty);
    slot.set(row, target);
}

/// One row's edit-mode controls as the draw pass paints them: the reorder
/// handle, the up and down halves it presses through (each only toward a
/// direction the row can actually move), and the delete control left of them.
/// `trailing_x` is where the row's content resumes. `list_accessibility`
/// registers the same controls' accessibility nodes on these bounds, so a
/// `Click` and a pointer tap land identically (water-rs/hydrolysis#52).
struct RowEditControls {
    reorder: Option<kurbo::Rect>,
    reorder_up: Option<kurbo::Rect>,
    reorder_down: Option<kurbo::Rect>,
    delete: Option<kurbo::Rect>,
    trailing_x: f64,
}

fn row_edit_controls(
    metrics: &crate::theme::ListMetrics,
    row_rect: kurbo::Rect,
    slot_height: f64,
    index: usize,
    movable_span: Option<Range<usize>>,
    delete_enabled: bool,
) -> RowEditControls {
    let mut controls = RowEditControls {
        reorder: None,
        reorder_up: None,
        reorder_down: None,
        delete: None,
        trailing_x: row_rect.x1 - 8.0,
    };
    let mut trailing_x = controls.trailing_x;
    if let Some(span) = movable_span {
        let control_width = f64::from(metrics.move_control_width);
        let vertical_inset = f64::from(metrics.trailing_control_vertical_inset);
        let control_height = vertical_inset
            .mul_add(-2.0, slot_height)
            .max(vertical_inset * 2.0);
        let control_rect = kurbo::Rect::new(
            trailing_x - control_width,
            row_rect.y0 + vertical_inset,
            trailing_x,
            row_rect.y0 + vertical_inset + control_height,
        );
        trailing_x -= control_width + f64::from(metrics.trailing_control_spacing);
        let half_height = control_rect.height() / 2.0;
        controls.reorder = Some(control_rect);
        controls.reorder_up = (index > span.start).then(|| {
            kurbo::Rect::new(
                control_rect.x0,
                control_rect.y0,
                control_rect.x1,
                control_rect.y0 + half_height,
            )
        });
        controls.reorder_down = (index + 1 < span.end).then(|| {
            kurbo::Rect::new(
                control_rect.x0,
                control_rect.y0 + half_height,
                control_rect.x1,
                control_rect.y1,
            )
        });
    }
    if delete_enabled {
        let vertical_inset = f64::from(metrics.trailing_control_vertical_inset);
        let delete_rect = kurbo::Rect::new(
            trailing_x - f64::from(metrics.delete_control_width),
            row_rect.y0 + vertical_inset,
            trailing_x,
            row_rect.y1 - vertical_inset,
        );
        trailing_x = delete_rect.x0 - f64::from(metrics.trailing_control_spacing);
        controls.delete = Some(delete_rect);
    }
    controls.trailing_x = trailing_x;
    controls
}

/// The delete control's one action — shared by its pointer target and its
/// accessibility `Click` node so both delete the row identically.
fn run_row_delete<E: crate::text::TextEngine>(
    state: &RefCell<ListRenderState<E>>,
    env: &Environment,
    row_id: ListItemId,
) -> bool {
    let edits = state.borrow().config.edits().clone();
    if edits.can_delete(row_id) {
        edits.delete(env, row_id);
    }
    true
}

/// One discrete reorder step — shared by the move halves' pointer targets and
/// their accessibility `Click` nodes. The preserve-anchor latch stays set only
/// when the move dirtied the rows: it holds the viewport's index over the
/// membership reconcile so the moved row visibly travels.
fn run_row_move<E: crate::text::TextEngine>(
    state: &RefCell<ListRenderState<E>>,
    env: &Environment,
    from_id: ListItemId,
    to_id: ListItemId,
    from_in_part: usize,
    to_in_part: usize,
) -> bool {
    let edits = {
        let state_ref = state.borrow();
        state_ref.preserve_anchor_index_once.set(true);
        state_ref.config.edits().clone()
    };
    edits.move_row(env, from_id, to_id, from_in_part, to_in_part);
    if !state.borrow().rows_dirty.get() {
        state.borrow().preserve_anchor_index_once.set(false);
    }
    true
}

/// Swipe-to-dismiss across a whole row, committing `on_delete` once the row
/// travels past Material's positional threshold.
fn register_row_swipe_gesture<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ListRenderState<E>>>,
    group: usize,
    row_id: ListItemId,
    drag: RowDrag,
    bounds: kurbo::Rect,
    row_env: &Environment,
) {
    let owner = Rc::clone(state);
    let action_env = row_env.clone();
    let RowDrag {
        binding,
        motion,
        slop,
    } = drag;
    register_row_gesture(
        ctx,
        state,
        group,
        row_id,
        bounds,
        RowGestureSlot::Swipe,
        || {
            let action: BoxedAction<()> = boxed_action(move |env: Environment| {
                let Some(event) = drag_event(&env) else {
                    return;
                };
                let offset = f64::from(event.translation().x);
                let row = binding.get();
                let (gestures, edits) = {
                    let list = owner.borrow();
                    (list.gestures(), list.config.edits().clone())
                };
                match event.phase() {
                    GesturePhase::Started | GesturePhase::Updated => {
                        if motion.track_swipe(
                            offset,
                            row.width,
                            f64::from(gestures.swipe_threshold),
                        ) {
                            owner.borrow().mark_rows_relisted();
                        }
                    }
                    GesturePhase::Ended => {
                        if offset.abs() >= row.width * f64::from(gestures.swipe_threshold) {
                            motion.rest();
                            if edits.can_delete(row_id) {
                                edits.delete(&action_env, row_id);
                            }
                        } else {
                            // Short of the threshold the row springs back instead
                            // of committing, exactly as `SwipeToDismissBox` does.
                            motion.settle(&gestures.settle);
                        }
                    }
                    GesturePhase::Cancelled => motion.settle(&gestures.settle),
                }
            });
            (Gesture::Drag(DragGesture::new(slop)), action)
        },
    );
}

/// Vertical reorder drag on a row's move handle, committing `on_move` on
/// release.
fn register_row_reorder_gesture<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ListRenderState<E>>>,
    group: usize,
    row_id: ListItemId,
    drag: RowDrag,
    bounds: kurbo::Rect,
    row_env: &Environment,
) {
    let owner = Rc::clone(state);
    let action_env = row_env.clone();
    let RowDrag {
        binding,
        motion,
        slop,
    } = drag;
    register_row_gesture(
        ctx,
        state,
        group,
        row_id,
        bounds,
        RowGestureSlot::Reorder,
        || {
            let action: BoxedAction<()> = boxed_action(move |env: Environment| {
                let Some(event) = drag_event(&env) else {
                    return;
                };
                let dy = f64::from(event.translation().y);
                let row = binding.get();
                let gestures = owner.borrow().gestures();
                // How many whole slots the pointer has travelled, a slot
                // stepped once the drag crosses the look's threshold of it.
                // Rows are measured individually, but a drag only ever
                // crosses its neighbours one at a time, so stepping by the
                // dragged row's own height tracks the pointer closely enough
                // to feel direct.
                let step = if row.height > 0.0 {
                    (dy.abs() / row.height + 1.0 - f64::from(gestures.reorder_threshold))
                        .floor()
                        .copysign(dy)
                } else {
                    0.0
                };
                let to = reorder_destination(row.index, step, row.movable_start, row.movable_end);
                match event.phase() {
                    GesturePhase::Started | GesturePhase::Updated => {
                        // The lifted row follows the pointer; every row
                        // between its origin and its destination slides one
                        // slot to make room.
                        let list = owner.borrow();
                        if motion.set_lifted(true) {
                            list.mark_rows_relisted();
                        }
                        motion.displace(kurbo::Vec2::new(0.0, dy), None);
                        list.shift_rows_for_reorder(row_id, row.index, to, Some(&gestures.settle));
                    }
                    GesturePhase::Ended => {
                        let target_and_edits = {
                            let list = owner.borrow();
                            if motion.set_lifted(false) {
                                list.mark_rows_relisted();
                            }
                            motion.displace(kurbo::Vec2::ZERO, None);
                            list.shift_rows_for_reorder(row_id, row.index, row.index, None);
                            if to == row.index {
                                None
                            } else {
                                list.rows_snapshot.borrow().get_id(to).map(|target_id| {
                                    list.preserve_anchor_index_once.set(true);
                                    (target_id, list.config.edits().clone())
                                })
                            }
                        };
                        if let Some((target_id, edits)) = target_and_edits {
                            edits.move_row(
                                &action_env,
                                row_id,
                                target_id,
                                row.index - row.movable_start,
                                to - row.movable_start,
                            );
                            if !owner.borrow().rows_dirty.get() {
                                owner.borrow().preserve_anchor_index_once.set(false);
                            }
                        }
                    }
                    GesturePhase::Cancelled => {
                        let list = owner.borrow();
                        if motion.set_lifted(false) {
                            list.mark_rows_relisted();
                        }
                        motion.displace(kurbo::Vec2::ZERO, Some(&gestures.settle));
                        list.shift_rows_for_reorder(
                            row_id,
                            row.index,
                            row.index,
                            Some(&gestures.settle),
                        );
                    }
                }
            });
            (Gesture::Drag(DragGesture::new(slop)), action)
        },
    );
}

/// Clamps a slot step from `index` into the owning part's movable index span
/// `span_start..span_end`.
fn reorder_destination(index: usize, step: f64, span_start: usize, span_end: usize) -> usize {
    if span_start >= span_end {
        return index;
    }
    let last = span_end - 1;
    if step >= 0.0 {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "step is a whole non-negative slot count, clamped into range below"
        )]
        let forward = step as usize;
        index.saturating_add(forward).min(last)
    } else {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the negated step is a whole non-negative slot count"
        )]
        let backward = (-step) as usize;
        index.saturating_sub(backward).max(span_start)
    }
}

/// Reads the drag payload the gesture engine layered into the action's
/// environment.
///
/// An action without its [`DragEvent`] is a contract violation by the gesture
/// engine, reported; the action is then ignored (`None`).
fn drag_event(env: &Environment) -> Option<DragEvent> {
    let event = env
        .extract::<State<DragEvent>>()
        .ok()
        .map(|State(event)| event);
    if event.is_none() {
        contract_violation!(
            "hydrolysis list drag action is missing its DragEvent; the drag is ignored"
        );
    }
    event
}

/// Which of a row's two retained recognizers is being addressed.
#[derive(Clone, Copy)]
enum RowGestureSlot {
    Swipe,
    Reorder,
}

impl RowGestureSlot {
    const fn get(self, row: &RowGestures) -> Option<&GestureTarget> {
        match self {
            Self::Swipe => row.swipe.as_ref(),
            Self::Reorder => row.reorder.as_ref(),
        }
    }

    fn set(self, row: &mut RowGestures, target: GestureTarget) {
        match self {
            Self::Swipe => row.swipe = Some(target),
            Self::Reorder => row.reorder = Some(target),
        }
    }
}

impl RowGestures {
    fn empty() -> Self {
        Self {
            binding: Rc::new(Cell::new(RowBinding::PLACEHOLDER)),
            swipe: None,
            reorder: None,
            motion: Rc::new(RowMotion::new()),
            controls: RowControlChrome {
                move_up: ControlChrome::new(),
                move_down: ControlChrome::new(),
                delete: ControlChrome::new(),
            },
        }
    }
}

/// Draws a section header — plain text for static sections, or text paired with
/// an animated theme disclosure indicator and pointer toggle target for
/// collapsible sections.
#[expect(
    clippy::too_many_arguments,
    reason = "header drawing receives render context, list state, stable id, text, expansion binding, hit/content rects, and env"
)]
fn draw_section_header<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    state: &Rc<RefCell<ListRenderState<E>>>,
    first_row_id: ListItemId,
    header: Text,
    expanded: Option<Binding<bool>>,
    hit_rect: kurbo::Rect,
    content_rect: kurbo::Rect,
    env: &Environment,
) {
    let Some(binding) = expanded else {
        draw_section_label(ctx, header, content_rect, true, env);
        return;
    };
    let disclosure_look = state.borrow().disclosure_look(&ctx.theme(), env);
    let direction = layout_direction(env).snapshot();
    let control = disclosure_look.control();
    let indicator_size = control.size();
    let spacing = f64::from(control.get_label_spacing());
    let ind_y0 = content_rect.y0 + ((content_rect.height() - indicator_size.height) * 0.5).max(0.0);
    let indicator_ltr = kurbo::Rect::new(
        (content_rect.x1 - indicator_size.width).max(content_rect.x0),
        ind_y0,
        content_rect.x1,
        (ind_y0 + indicator_size.height).min(content_rect.y1),
    );
    let label_ltr = kurbo::Rect::new(
        content_rect.x0,
        content_rect.y0,
        (indicator_ltr.x0 - spacing).max(content_rect.x0),
        content_rect.y1,
    );
    let (indicator_rect, label_rect) = match direction {
        waterui_core::layout::LayoutDirection::LeftToRight => (indicator_ltr, label_ltr),
        waterui_core::layout::LayoutDirection::RightToLeft => (
            crate::widgets::util::mirrored_in(indicator_ltr, content_rect),
            crate::widgets::util::mirrored_in(label_ltr, content_rect),
        ),
    };
    draw_section_label(ctx, header, label_rect, true, env);
    let key = crate::renderer::InteractionKey::for_rc(
        state,
        row_interaction_base(first_row_id) + INTERACTION_SLOT_SECTION_HEADER,
    );
    let (_, press_slot, handles) =
        ctx.renderer_mut()
            .bind_live_toggle_target(key.clone(), hit_rect, env, false, control);
    let flags = ctx.renderer_mut().reported_interaction_state(&key);
    let parts = {
        let state_ref = state.borrow();
        let mut toggles = state_ref.section_toggles.borrow_mut();
        let slot = toggles.entry(first_row_id).or_insert(None);
        LiveToggleParts::ensure(slot, control, indicator_rect, flags, &binding).clone()
    };
    handles.attach_live_toggle(Rc::clone(parts.toggle()));
    let ind_ctx = ctx.render_context();
    parts.push_keyed(
        ctx.renderer_mut(),
        ind_ctx,
        u64::from(crate::num_cast::i32_as_u32(i32::from(first_row_id))),
    );
    ctx.renderer_mut().register_interactive_pointer_target(
        hit_rect,
        press_slot,
        move |_renderer, _point, _env| {
            binding.toggle();
            true
        },
    );
}

/// Draws a section header or footer.
///
/// The chrome reads the `MutedForeground` theme token rather than naming a
/// colour, so a section title matches the platform's own secondary text on
/// every theme.
fn draw_section_label<E: crate::text::TextEngine>(
    ctx: &mut WidgetRenderContext<'_, E>,
    label: Text,
    bounds: kurbo::Rect,
    is_header: bool,
    env: &Environment,
) {
    if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return;
    }
    let styled = ctx
        .renderer_mut()
        .read_signal(&section_chrome_text(label, is_header).resolve(env).content());
    ctx.render_styled_text(
        styled,
        waterui_layout::stack::HorizontalAlignment::Leading,
        env,
        bounds,
    );
}

/// Applies the section chrome's own typography to a header or footer.
///
/// Section chrome is the platform's, not the app's: the title is styled by the
/// list, the way `UITableView` and Material section headers style theirs.
/// Only the text content comes from the app, and it stays reactive.
fn section_chrome_text(label: Text, is_header: bool) -> Text {
    let text = label.color(waterui_graphics::color::Color::new(
        waterui_controls::theme::color::MutedForeground,
    ));
    if is_header {
        text.font(waterui_text::font::Subheadline)
    } else {
        text.font(waterui_text::font::Caption)
    }
}

fn list_content_rect(
    row_rect: kurbo::Rect,
    metrics: crate::theme::ListMetrics,
    insets: Option<&waterui_layout::padding::EdgeInsets>,
    content_size: waterui_core::layout::Size,
    env: &Environment,
) -> kurbo::Rect {
    // Rows propose their full inset width to the content; horizontal
    // alignment belongs to the content itself (composite items cannot be
    // statically classified as stretching, and interactive rows must keep a
    // full-width hit target). The row's `insets` replace the theme's row
    // insets edge for edge; unset, the theme's symmetric inset keeps the
    // same rect. The row's height was measured at this same width.
    let vertical_insets = insets.map_or_else(
        || f64::from(metrics.vertical_inset) * 2.0,
        |insets| f64::from(insets.top() + insets.bottom()),
    );
    let (left_inset, right_inset) = list_row_horizontal_insets(insets, metrics, env);
    let x0 = row_rect.x0 + left_inset;
    let x1 = row_rect.x1 - right_inset;
    let available_height = (row_rect.height() - vertical_insets).max(0.0);
    let height = f64::from(content_size.height).min(available_height);
    let y0 = (row_rect.height() - height).mul_add(0.5, row_rect.y0);
    kurbo::Rect::new(x0, y0, x1, y0 + height)
}

/// Where a row positions its content's response (water-rs/waterui#3888): at
/// the leading edge of the row's content rect — trailing in right-to-left —
/// centred vertically. The content keeps the whole content rect as its
/// allocation, so the row's hit target spans it.
fn row_content_anchor(env: &Environment) -> UnitPoint {
    if layout_direction(env).snapshot().is_right_to_left() {
        UnitPoint::TRAILING
    } else {
        UnitPoint::LEADING
    }
}

/// Emits a retained list's accessibility tree for the semantic walk — the same
/// nodes `list_accessibility` registers, with no bounds and every row present.
#[cfg(feature = "accessibility")]
#[cfg(hydrolysis_cherenkov)]
pub fn emit_list_accessibility<E: crate::text::TextEngine>(
    renderer: &mut crate::renderer::SemanticCore<E>,
    state: &Rc<RefCell<ListRenderState<E>>>,
    env: &Environment,
) {
    let hidden = renderer.is_accessibility_hidden(env);
    if hidden {
        renderer.push_accessibility_suppression();
    }
    list_accessibility(renderer, None, None, state, env);
    if hidden {
        renderer.pop_accessibility_suppression();
    }
}
