//! The `list` leaf: `Native<ListConfig>` rendered through the kit's table
//! surface.
//!
//! Mirrors `WuiList`: `UIKit` renders a `UITableView` with `.insetGrouped`
//! sections, `AppKit` an `NSScrollView` + `NSTableView` over a flat entry
//! list (section header / row / footer). Section grouping, row
//! construction, selection state, editing, reorder drag and the scroll
//! controller all follow the Swift port; membership changes apply as
//! batched delete/insert only when the list is a single plain section and
//! in a window — a full reload otherwise.

use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::rc::{Rc, Weak};
#[cfg(target_os = "macos")]
use alloc::string::ToString;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::fmt;
use std::collections::{HashMap, HashSet};

use cocoa_ui::geometry::EdgeInsets as KitInsets;
use cocoa_ui::{Retained, view};
use waterui::animation::Animation;
use waterui::component::list::{
    ListConfig, ListItem, ListSection, ListSelection, Move, OnDelete, OnMove,
};
use waterui::id::Id as RawId;
use waterui::layout::padding::EdgeInsets;
use waterui::layout::scroll::{Axis, ScrollController, ScrollPosition, ScrollRequest};
use waterui::reactive::Signal;
use waterui::reactive::binding::Binding;
use waterui::reactive::collection::{CollectionChange, diff, has_duplicate_ids};
use waterui::reactive::watcher::{BoxWatcherGuard, Metadata};
#[cfg(target_os = "macos")]
use waterui::resolve::Resolvable;
use waterui::text::{StyledStr, Text};
use waterui::views::{AnyViewsSnapshot, SharedAnyViews, ViewSnapshot, Views};
use waterui_backend_core::Environment;
use waterui_backend_core::scroll::animated_row_scroll_approach;
use waterui_core::Computed;
use waterui_core::interaction::Disabled;
use waterui_core::layout::{Point, ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::{NativeLeaf, RenderContext, Renderer};
use crate::proposal::{self, Placement};
use crate::transition::ghost::Ghost;
use crate::transition::live::{LiveChild, Removal};
use crate::transition::slots::{GhostKey, Slot, SlotSequence};

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{
    DeleteButton, ListTableView as TableView, RowContainer, SectionHeader, SectionKind,
    TableRowView, TableSource,
};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{IndexPath, SectionKind, TableCell, TableSource, TableView};

/// A row's identity: the collection id `SharedAnyViews` answers for an
/// index.
type ItemId = RawId;

/// The table as its platform view — `UITableView` on `UIKit`, the wrapping
/// `NSScrollView` on `AppKit`.
fn as_view(table: &TableView) -> &cocoa_ui::PlatformView {
    table
}

/// `wuiListMinRowHeight`: an explicit minimum wins, the platform theme's
/// stock row height otherwise.
fn min_row_height(configured: Option<f32>, theme: f64) -> f64 {
    configured.map_or(theme, f64::from)
}

/// `wuiListRowHeight`: measured content plus the row's vertical insets,
/// floored at the resolved minimum.
fn row_height(content_height: f64, insets: KitInsets, minimum: f64) -> f64 {
    (content_height + insets.top + insets.bottom).max(minimum)
}

/// `peekListItemSection`: the `ListSection` marker `index` carries, when
/// it starts a section.
fn peek_section(snapshot: &AnyViewsSnapshot<ListItem>, index: usize) -> Option<ListSection> {
    snapshot.get_view(index).and_then(ListItem::into_section)
}

/// A resolved section `Text` as its live styled-string signal.
fn resolve_text(text: &Text, env: &Environment) -> Computed<StyledStr> {
    text.resolve(env).into_content()
}

/// `computeListSectionGroups`'s answer: a band's texts and how many items
/// it covers; groups partition the item indexes in order, so a count is
/// all a group needs.
#[derive(Debug)]
struct SectionGroup {
    /// The section's resolved label text, watched live.
    label: Option<Computed<StyledStr>>,
    /// The section's resolved footer text, watched live.
    footer: Option<Computed<StyledStr>>,
    /// How many items the group covers.
    count: usize,
}

/// `computeListSectionGroups`: a section marker starts a group, items
/// accumulate into the pending group, and the pending group flushes when
/// the next marker — or the end — arrives.
fn compute_section_groups(
    snapshot: &AnyViewsSnapshot<ListItem>,
    positions: &HashMap<ItemId, usize>,
    item_ids: &[ItemId],
    env: &Environment,
) -> Vec<SectionGroup> {
    let mut groups = Vec::new();
    let mut pending: Option<SectionGroup> = None;
    for &id in item_ids {
        let index = positions[&id];
        if let Some(section) = peek_section(snapshot, index) {
            if let Some(group) = pending.take() {
                groups.push(group);
            }
            pending = Some(SectionGroup {
                label: section.label().map(|text| resolve_text(text, env)),
                footer: section.get_footer().map(|text| resolve_text(text, env)),
                count: 0,
            });
        }
        match &mut pending {
            Some(group) => group.count += 1,
            None => {
                pending = Some(SectionGroup {
                    label: None,
                    footer: None,
                    count: 1,
                });
            }
        }
    }
    if let Some(group) = pending.take() {
        groups.push(group);
    }
    groups
}

/// `resolveListSectionGroups`: with `uses_sections` on, group by the
/// items' markers; off, one unlabeled group covers everything.
fn resolve_section_groups(
    snapshot: &AnyViewsSnapshot<ListItem>,
    positions: &HashMap<ItemId, usize>,
    item_ids: &[ItemId],
    uses_sections: bool,
    env: &Environment,
) -> Vec<SectionGroup> {
    if uses_sections {
        compute_section_groups(snapshot, positions, item_ids, env)
    } else if item_ids.is_empty() {
        Vec::new()
    } else {
        vec![SectionGroup {
            label: None,
            footer: None,
            count: item_ids.len(),
        }]
    }
}

/// `isSinglePlainSection`: one group carrying no chrome is the shape batch
/// updates are safe for.
const fn is_single_plain_section(groups: &[SectionGroup]) -> bool {
    groups.len() == 1 && groups[0].label.is_none() && groups[0].footer.is_none()
}

/// The row edits that turn one plain section's rows into another's, in
/// batch-update terms: `deletes` and the source of each move index the old
/// rows; `inserts` and the destination of each move index the new rows.
#[derive(Debug, PartialEq, Eq)]
struct RowEdits {
    deletes: Vec<usize>,
    inserts: Vec<usize>,
    moves: Vec<(usize, usize)>,
}

/// `singleSectionRowDiff`: the deletes, inserts and moves that turn `old`
/// into `new`, aligned by id, or `None` when either list carries a duplicate
/// id — a row update cannot name which of two equal ids moved.
fn single_section_row_diff(old: &[ItemId], new: &[ItemId]) -> Option<RowEdits> {
    if has_duplicate_ids(old, |_, id| *id) || has_duplicate_ids(new, |_, id| *id) {
        return None;
    }
    let change = diff(old, new, |_, id| *id, |_, _| true);
    Some(RowEdits {
        deletes: change.removals().flatten().collect(),
        inserts: change.insertions().flatten().collect(),
        moves: change.moves().collect(),
    })
}

/// `AppKit` applies the edits of an update block one after another, each
/// against the rows the previous one left: after removing the `removed` old
/// ids, these are the moves — `(from, to)` in the current rows — that put the
/// remaining rows in their `new` order, before the `inserted` new ids go in
/// at their final positions.
#[cfg(target_os = "macos")]
fn sequential_moves(
    old: &[ItemId],
    new: &[ItemId],
    removed: &HashSet<ItemId>,
    inserted: &HashSet<ItemId>,
) -> Vec<(usize, usize)> {
    let mut current: Vec<ItemId> = old
        .iter()
        .copied()
        .filter(|id| !removed.contains(id))
        .collect();
    let target = new.iter().copied().filter(|id| !inserted.contains(id));
    let mut moves = Vec::new();
    for (to, id) in target.enumerate() {
        if current[to] != id {
            let from = to
                + 1
                + current[to + 1..]
                    .iter()
                    .position(|candidate| *candidate == id)
                    .expect("a surviving row is still among the current rows");
            current.remove(from);
            current.insert(to, id);
            moves.push((from, to));
        }
    }
    moves
}

/// The snapshot's ids, in order.
fn ids_snapshot(snapshot: &AnyViewsSnapshot<ListItem>) -> Vec<ItemId> {
    snapshot
        .range()
        .filter_map(|index| snapshot.get_id(index))
        .collect()
}

/// Each snapshot id's collection-wide position — item ids stay stable
/// across display reordering, so row realization always finds a row's
/// retained data.
fn positions_snapshot(snapshot: &AnyViewsSnapshot<ListItem>) -> HashMap<ItemId, usize> {
    snapshot
        .range()
        .filter_map(|index| snapshot.get_id(index).map(|id| (id, index)))
        .collect()
}

#[cfg(all(target_os = "ios", feature = "navigation"))]
/// `containsNavigationLink`: walk the primary-content chain looking for a
/// navigation-link wrapper — a matched node gives the row a disclosure
/// indicator. The Rust path tags the wrapper with an accessibility
/// identifier rather than a Swift class name. Only the `navigation` port
/// sets that identifier, so the probe exists only alongside it.
fn contains_navigation_link(view: &cocoa_ui::PlatformView) -> bool {
    let mut current = view::retain_base(view);
    loop {
        if view::accessibility_identifier(&current).as_deref()
            == Some(crate::components::navigation::metadata::LINK_HINT_IDENTIFIER)
        {
            return true;
        }
        match view::primary_content(&current) {
            Some(next) => current = next,
            None => return false,
        }
    }
}

#[cfg(target_os = "ios")]
/// Whether this table is the navigation page's first declared scroll surface.
fn is_navigation_scroll_surface(table: &TableView) -> bool {
    let Some(controller) = cocoa_ui::uikit::view_controller::enclosing_controller(table) else {
        return false;
    };
    if controller.navigationController().is_none() {
        return false;
    }
    controller
        .view()
        .and_then(|root| crate::primary_content::declared_scroll_view(&root))
        .is_some_and(|surface| {
            let surface: &cocoa_ui::PlatformView = surface.as_ref();
            let table: &cocoa_ui::PlatformView = table;
            core::ptr::eq(surface, table)
        })
}

/// The selection mode `ListConfig` carries — `WuiList`'s
/// `SelectionController`.
enum SelectionMode {
    /// Rows are not selectable.
    None,
    /// A single selected row; `None` inside the binding means nothing is
    /// selected.
    Single(Binding<Option<ItemId>>),
    /// The set of selected row ids.
    Multiple(Binding<BTreeSet<ItemId>>),
}

impl fmt::Debug for SelectionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SelectionMode")
    }
}

impl SelectionMode {
    /// Whether rows accept taps as selection.
    const fn allows_selection(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// Whether several rows may be selected at once.
    #[cfg(target_os = "ios")]
    const fn allows_multiple(&self) -> bool {
        matches!(self, Self::Multiple(_))
    }

    /// `SelectionController.selectedIds`: the binding's current selection.
    fn selected_ids(&self) -> BTreeSet<ItemId> {
        match self {
            Self::None => BTreeSet::new(),
            Self::Single(binding) => {
                let mut set = BTreeSet::new();
                if let Some(id) = binding.snapshot() {
                    set.insert(id);
                }
                set
            }
            Self::Multiple(binding) => binding.snapshot(),
        }
    }

    /// The table's selected ids written back into the binding.
    fn write(&self, ids: &BTreeSet<ItemId>) {
        match self {
            Self::None => {}
            Self::Single(binding) => binding.set(ids.iter().next().copied()),
            Self::Multiple(binding) => binding.set(ids.clone()),
        }
    }
}

/// A `waterui` `EdgeInsets` (leading/trailing) as the kit's directional
/// `left`/`right`.
fn kit_insets(insets: &EdgeInsets) -> KitInsets {
    KitInsets {
        top: f64::from(insets.top()),
        bottom: f64::from(insets.bottom()),
        left: f64::from(insets.leading()),
        right: f64::from(insets.trailing()),
    }
}

/// `AppKit`: the band a flat row presents.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlatEntry {
    /// A section header for `groups[section]`.
    Header(usize),
    /// The item at flat index `index`.
    Row(usize),
    /// A removal ghost's display slot — the row stays while the
    /// transition runs; `Shared::ghosts` owns the content.
    Ghost(GhostKey),
    /// A section footer for `groups[section]`.
    Footer(usize),
}

/// `rebuildFlatLayout`: the section header → rows → section footer
/// sequence `flatLayout` holds. The display order drives the row band:
/// ghosts land in their anchor's section, head ghosts in the first.
#[cfg(target_os = "macos")]
fn rebuild_flat_layout(state: &mut Shared) {
    state.flat_layout.clear();
    let rows = state.display_rows();
    let sections = state.groups.len().max(usize::from(!rows.is_empty()));
    for section in 0..sections {
        if let Some(group) = state.groups.get(section)
            && group.label.is_some()
        {
            state.flat_layout.push(FlatEntry::Header(section));
        }
        for (member_section, member) in &rows {
            if *member_section == section {
                state.flat_layout.push(match member {
                    DisplayRow::Live(flat) => FlatEntry::Row(*flat),
                    DisplayRow::Ghost(key) => FlatEntry::Ghost(*key),
                });
            }
        }
        if let Some(group) = state.groups.get(section)
            && group.footer.is_some()
        {
            state.flat_layout.push(FlatEntry::Footer(section));
        }
    }
}

/// `AppKit`: re-resolves each materialized row's separator after a
/// batched membership change — the update reuses the surviving row
/// views, so a row that became or stopped being its section's last
/// member needs its flag reset (water-rs/waterui#3592). The other paths
/// re-ask `rowViewForRow:` through `reloadData`, which resolves the flag
/// fresh.
#[cfg(target_os = "macos")]
fn refresh_separators(state: &Rc<RefCell<Shared>>, table: &TableView) {
    table.enumerate_row_views({
        let state = Rc::clone(state);
        move |row, row_view| {
            if let Some(row_view) = row_view.downcast_ref::<TableRowView>() {
                row_view.set_shows_separator(state.borrow().shows_separator(row));
            }
        }
    });
}

/// What one display row is: a live row's flat item index, or a removal
/// ghost's key.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DisplayRow {
    /// The live item at flat index `flat`.
    Live(usize),
    /// The removal ghost `key` holds.
    Ghost(GhostKey),
}

/// The batch edits over the display order plus what the table calls
/// need on top: the re-materialized rows and the inserts that play
/// their own transition with no row animation.
struct DisplayEdits {
    edits: RowEdits,
    /// `(old row, new row)` display positions to re-materialize.
    reloads: Vec<(usize, usize)>,
    /// Display positions whose insertion plays its own transition.
    claimed: HashSet<usize>,
    /// The keyed display orders — the move replay `AppKit` needs.
    #[cfg(target_os = "macos")]
    old_keys: Vec<ItemId>,
    /// The keyed display orders — the move replay `AppKit` needs.
    #[cfg(target_os = "macos")]
    new_keys: Vec<ItemId>,
}

/// A row inserted by the pending change, rendered at diff time:
/// `configure_cell`/`viewFor` mounts it through `LiveChild::mount_inserted`.
/// `claimed` records whether the leaf declared a transition so the batch
/// inserts it with no row animation.
struct PendingRow {
    insets: Option<EdgeInsets>,
    deletable: Computed<bool>,
    leaf: NativeLeaf,
    claimed: bool,
}

/// A ghost row's cell payload — a marker only: `Shared::ghosts` owns the
/// `Ghost`, so reusing a cell that showed a ghost drops nothing of the
/// transition.
struct GhostPayload(#[allow(dead_code)] GhostKey);

impl fmt::Debug for GhostPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GhostPayload").finish_non_exhaustive()
    }
}

/// The row height a ghost freezes at — `measured_heights` when the row's
/// last measurement stands, else the retired host's grandparent bounds:
/// the contract the table last applied, read off the cell (the host's
/// superview is the cell's content view / `RowContainer`).
fn frozen_row_height(shared: &Shared, id: ItemId, host: &cocoa_ui::PlatformView) -> f64 {
    shared
        .measured_heights
        .get(&id)
        .copied()
        .unwrap_or_else(|| {
            view::superview(host)
                .and_then(|parent| view::superview(&parent))
                .map_or(shared.resolved_min_height, |cell| {
                    view::bounds(&cell).size.height
                })
        })
}

/// The insets `view`'s frame keeps inside its superview — how a ghost
/// cell reproduces the departed row's content pinning.
fn pinned_insets(view: &cocoa_ui::PlatformView) -> KitInsets {
    let Some(parent) = view::superview(view) else {
        return KitInsets {
            top: 0.0,
            bottom: 0.0,
            left: 0.0,
            right: 0.0,
        };
    };
    let frame = view::frame(view);
    let bounds = view::bounds(&parent);
    KitInsets {
        top: frame.origin.y,
        left: frame.origin.x,
        bottom: bounds.size.height - frame.origin.y - frame.size.height,
        right: bounds.size.width - frame.origin.x - frame.size.width,
    }
}

/// Whether `id`'s configured cell still holds a claimed `LiveChild` —
/// the gate a departing row must pass to keep a ghost slot.
fn claimed_and_configured(shared: &Shared, id: ItemId) -> bool {
    shared
        .live_rows
        .get(&id)
        .and_then(Weak::upgrade)
        .is_some_and(|slot| {
            slot.borrow()
                .as_ref()
                .is_some_and(|child| child.claim().is_some())
        })
}

/// The list's live state, shared between the source, watchers, and the
/// layout face.
struct Shared {
    /// The lazily materialized collection — watches register here and
    /// `snapshot()` captures state; membership and row answers never
    /// read it live.
    contents: SharedAnyViews<ListItem>,
    /// The retained row-data snapshot aligned with `item_ids` — every
    /// `get_id`/`get_view` answer comes from it, so row realization
    /// materializes exact retained data even when the source mutates
    /// mid-materialization.
    snapshot: AnyViewsSnapshot<ListItem>,
    /// `item_ids`-keyed positions inside `snapshot` — survives local
    /// deletes and moves, so a row's retained view stays exact while
    /// display order and snapshot membership differ mid-transition.
    positions: HashMap<ItemId, usize>,
    /// The environment callbacks and section texts resolve through.
    env: Environment,
    /// Renders item contents into leaves.
    renderer: Renderer,
    /// The collection's current ids, in order.
    item_ids: Vec<ItemId>,
    /// What the table displays: the live ids in `item_ids` order
    /// interleaved with removal ghosts. Every data-source answer reads
    /// this; `item_ids` stays the semantic order (§5.4).
    display: SlotSequence<ItemId>,
    /// Each configured cell's shared child slot, by item id — the `Weak`
    /// dies when the cell's payload drops, so an entry that no longer
    /// upgrades names a row with no live cell.
    live_rows: HashMap<ItemId, Weak<RefCell<Option<LiveChild>>>>,
    /// Removal ghosts the table still shows, keyed by display slot: the
    /// ghost run plus the row's frozen height.
    ghosts: HashMap<GhostKey, (Ghost, f64)>,
    /// Each ghost's departed id — the display diff keys a ghost by it, so
    /// a ghosted row stays the same table row (no delete, no reload).
    ghost_ids: HashMap<GhostKey, ItemId>,
    /// Rows inserted by the pending change, rendered at diff time:
    /// `configure_cell`/`viewFor` mounts them through
    /// `LiveChild::mount_inserted`; an entry left at the batch's end
    /// belonged to an off-screen row and drops without playing.
    pending_inserted: HashMap<ItemId, PendingRow>,
    /// The section groups covering `item_ids`, in order.
    groups: Vec<SectionGroup>,
    /// `AppKit`: `flatLayout` — the table's flat entry list.
    #[cfg(target_os = "macos")]
    flat_layout: Vec<FlatEntry>,
    /// Selection bookkeeping.
    selection: SelectionMode,
    /// `SelectionController.applyingToTable` — guards re-entrancy while a
    /// binding change applies to the table.
    applying_selection: Cell<bool>,
    /// The editing signal.
    #[cfg(target_os = "macos")]
    editing: Computed<bool>,
    /// Delete callback.
    on_delete: Option<OnDelete>,
    /// Reorder callback.
    on_move: Option<OnMove>,
    /// `uses_sections` — whether item markers group the list.
    uses_sections: bool,
    /// The theme's row insets for `None` inputs.
    #[cfg(target_os = "macos")]
    theme_insets: KitInsets,
    /// The resolved minimum row height.
    resolved_min_height: f64,
    /// `measuredRowHeights`: the row contract the height delegate reports,
    /// refreshed from both `viewFor`/`cellForRow` layout passes.
    measured_heights: HashMap<ItemId, f64>,
    /// The `Disabled` signal each rendered row resolved under, by item id —
    /// recorded on the row's own render pass, so a `.disabled` binding flip
    /// takes effect without a reload (water-rs/waterui#2244).
    disabled_rows: RefCell<HashMap<ItemId, Computed<bool>>>,
}

impl fmt::Debug for Shared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Shared").finish_non_exhaustive()
    }
}

impl Shared {
    /// Every live row's index path in display order (`UIKit`).
    #[cfg(target_os = "ios")]
    fn live_index_paths(&self) -> Vec<IndexPath> {
        let mut counts: Vec<usize> = Vec::new();
        self.display_rows()
            .into_iter()
            .filter_map(|(section, member)| {
                while counts.len() <= section {
                    counts.push(0);
                }
                let row = counts[section];
                counts[section] += 1;
                matches!(member, DisplayRow::Live(_)).then_some(IndexPath { section, row })
            })
            .collect()
    }

    /// `updateVirtualIds` + `reloadSectionGroups`: take `snapshot` as
    /// current — ids, positions, groups and the height cache all follow
    /// it.
    fn apply_snapshot(&mut self, snapshot: AnyViewsSnapshot<ListItem>) {
        self.item_ids = ids_snapshot(&snapshot);
        self.positions = positions_snapshot(&snapshot);
        self.snapshot = snapshot;
        self.regroup();
        let seen: HashSet<ItemId> = self.item_ids.iter().copied().collect();
        self.measured_heights.retain(|id, _| seen.contains(id));
        self.disabled_rows
            .get_mut()
            .retain(|id, _| seen.contains(id));
    }

    /// Regroups over the current `item_ids` — after a local delete or
    /// move mutates them.
    fn regroup(&mut self) {
        self.groups = resolve_section_groups(
            &self.snapshot,
            &self.positions,
            &self.item_ids,
            self.uses_sections,
            &self.env,
        );
    }

    /// Whether `flat`'s row resolved under `.disabled` on its last render
    /// — such a row is inert the way a disabled `Button` cannot be
    /// pressed: pointer and accessibility-driven selection both refuse it
    /// while its label and disabled state still report
    /// (water-rs/waterui#2244). Rows never rendered answer `false`.
    fn row_disabled(&self, flat: usize) -> bool {
        self.disabled_rows
            .borrow()
            .get(&self.item_ids[flat])
            .is_some_and(Signal::snapshot)
    }

    /// `writeSelectionFromTable`: `flat` item indexes selected on the
    /// table written into the binding — skipped while a binding change is
    /// applying to the table.
    fn write_selection(&self, flats: impl Iterator<Item = usize>) {
        if self.applying_selection.get() {
            return;
        }
        let mut ids = BTreeSet::new();
        for flat in flats {
            if let Some(&id) = self.item_ids.get(flat) {
                ids.insert(id);
            }
        }
        self.selection.write(&ids);
    }

    /// Renders `flat`'s item, returning its insets, deletable signal and leaf.
    fn render_row(&self, flat: usize) -> (Option<EdgeInsets>, Computed<bool>, NativeLeaf) {
        let item = self
            .snapshot
            .get_view(self.positions[&self.item_ids[flat]])
            .expect("list item index is in bounds");
        let (content, deletable, _, insets) = item.into_parts();
        // The row's `Disabled` signal comes from the scope this render
        // resolves the content under — the same pass, not a second walk.
        let (leaf, resolved_env) = self.renderer.render_resolved(content);
        self.disabled_rows
            .borrow_mut()
            .insert(self.item_ids[flat], Disabled::resolve(&resolved_env, false));
        (insets, deletable, leaf)
    }

    /// The table's display rows: `(section, member)` in display order. A
    /// ghost lands in its anchor's section — the section of the live row
    /// it follows — and head ghosts land in the first section.
    fn display_rows(&self) -> Vec<(usize, DisplayRow)> {
        let mut rows = Vec::new();
        let mut flat = 0;
        let mut section = 0;
        let mut remaining = self.groups.first().map_or(0, |group| group.count);
        let mut anchor = 0;
        for slot in self.display.iter() {
            match slot {
                Slot::Live(_) => {
                    while remaining == 0 {
                        section += 1;
                        remaining = self.groups[section].count;
                    }
                    remaining -= 1;
                    anchor = section;
                    rows.push((section, DisplayRow::Live(flat)));
                    flat += 1;
                }
                Slot::Ghost(key) => rows.push((anchor, DisplayRow::Ghost(key))),
            }
        }
        rows
    }

    /// The display member `section`/`row` names (`UIKit`).
    #[cfg(target_os = "ios")]
    fn display_at(&self, section: usize, row: usize) -> Option<DisplayRow> {
        self.display_rows()
            .into_iter()
            .filter(|(member_section, _)| *member_section == section)
            .nth(row)
            .map(|(_, member)| member)
    }

    /// The `(section, row)` `wanted` displays at — `row` counting within
    /// `section`'s displayed members (`UIKit`).
    #[cfg(target_os = "ios")]
    fn display_index_of(&self, wanted: DisplayRow) -> Option<(usize, usize)> {
        let mut counts: Vec<usize> = Vec::new();
        for (section, member) in self.display_rows() {
            while counts.len() <= section {
                counts.push(0);
            }
            let row = counts[section];
            counts[section] += 1;
            if member == wanted {
                return Some((section, row));
            }
        }
        None
    }

    /// Each section's displayed member count: its live rows plus the
    /// ghosts anchored into it. With no live rows at all the ghosts
    /// still form one section (`UIKit`).
    #[cfg(target_os = "ios")]
    fn display_section_counts(&self) -> Vec<usize> {
        let mut counts: Vec<usize> = self.groups.iter().map(|group| group.count).collect();
        for (section, member) in self.display_rows() {
            if matches!(member, DisplayRow::Ghost(_)) {
                if counts.len() <= section {
                    counts.resize(section + 1, 0);
                }
                counts[section] += 1;
            }
        }
        counts
    }

    /// Reconciles `display` against `new_ids`: departed rows that pass
    /// the configured-and-claimed gate keep their slot and retire into
    /// ghosts now; the rest release. Returns the pre-edit display order
    /// and the inserted ids — `apply_snapshot` has not run yet, so
    /// `item_ids` still names the old membership.
    fn reconcile_display(
        &mut self,
        new_ids: &[ItemId],
        state: &Rc<RefCell<Self>>,
        table: &TableView,
    ) -> (Vec<Slot<ItemId>>, Vec<ItemId>) {
        // The display order before `reconcile` — the batch diff indexes
        // the table's current rows with it.
        let old_display: Vec<Slot<ItemId>> = self.display.iter().collect();
        // A departed row keeps a ghost slot only while its configured
        // cell still holds a claimed `LiveChild` — an unmounted or
        // unclaimed row releases normally.
        let ghostable: HashSet<ItemId> = self
            .item_ids
            .iter()
            .copied()
            .filter(|id| claimed_and_configured(self, *id))
            .collect();
        let reconciled = self
            .display
            .reconcile(new_ids, |id| ghostable.contains(&id));
        // Ghost heights freeze at the row's last measurement, read before
        // `apply_snapshot` retains the cache down to the surviving ids.
        let frozen: HashMap<ItemId, f64> = reconciled
            .departed
            .iter()
            .filter(|(_, key)| key.is_some())
            .filter_map(|(id, _)| {
                self.measured_heights
                    .get(id)
                    .copied()
                    .map(|height| (*id, height))
            })
            .collect();
        // Retire each departed row that kept a slot; rejects return their
        // keys so the display diff names only real table edits.
        for (id, key) in reconciled.departed {
            if let Some(key) = key {
                self.retire_row(id, key, state, table, frozen.get(&id).copied());
            } else {
                self.pending_inserted.remove(&id);
                if let Some(slot) = self.live_rows.remove(&id).and_then(|weak| weak.upgrade()) {
                    drop(slot.borrow_mut().take());
                }
            }
        }
        (old_display, reconciled.inserted)
    }

    /// Renders each inserted row into `pending_inserted` — a claimed row
    /// mounts through `LiveChild::mount_inserted` when its cell
    /// configures, and the batch inserts it with no row animation.
    fn render_inserted(&mut self, inserted: &[ItemId]) {
        for &id in inserted {
            if let Some(flat) = self.flat_of_id(id) {
                let (insets, deletable, mut leaf) = self.render_row(flat);
                let claimed = leaf.take_transition().is_some_and(|claim| {
                    leaf.set_transition(claim);
                    !leaf.layout().is_empty()
                });
                self.pending_inserted.insert(
                    id,
                    PendingRow {
                        insets,
                        deletable,
                        leaf,
                        claimed,
                    },
                );
            }
        }
    }

    /// The batch edits over the display order — `None` for a
    /// multi-section or duplicate-key shape the batch path cannot name.
    /// The diff keys a ghost as its departed id: a ghosted row is the
    /// same table row, so it reports neither a delete nor a reload.
    fn display_edits(
        &self,
        old_display: &[Slot<ItemId>],
        dirty: &HashSet<ItemId>,
        old_ids: &HashSet<ItemId>,
    ) -> Option<DisplayEdits> {
        if !is_single_plain_section(&self.groups) {
            return None;
        }
        let new_display: Vec<Slot<ItemId>> = self.display.iter().collect();
        let key_of = |display: &[Slot<ItemId>]| -> Vec<ItemId> {
            display
                .iter()
                .map(|slot| match slot {
                    Slot::Live(id) => *id,
                    Slot::Ghost(key) => self.ghost_ids[key],
                })
                .collect()
        };
        #[cfg(target_os = "macos")]
        let old_keys = key_of(old_display);
        #[cfg(target_os = "macos")]
        let new_keys = key_of(&new_display);
        let edits = single_section_row_diff(&key_of(old_display), &key_of(&new_display))?;
        // Display positions of each live id, before and after — the row
        // indexes the table calls speak in.
        let live_pos = |display: &[Slot<ItemId>]| -> HashMap<ItemId, usize> {
            display
                .iter()
                .enumerate()
                .filter_map(|(index, slot)| match slot {
                    Slot::Live(id) => Some((*id, index)),
                    Slot::Ghost(_) => None,
                })
                .collect()
        };
        let old_pos = live_pos(old_display);
        let new_pos = live_pos(&new_display);
        // Surviving ids whose content was updated keep their mounted leaf
        // unless re-materialized; the diff's inserts only name new ids,
        // which re-materialize through their own path.
        let reloads = self
            .item_ids
            .iter()
            .filter(|id| dirty.contains(id) && old_ids.contains(id))
            .map(|id| (old_pos[id], new_pos[id]))
            .collect();
        // A display insert names a live row — ghosts are keyed as their
        // departed id — so only `Slot::Live` positions can be claimed.
        let claimed = edits
            .inserts
            .iter()
            .filter(|row| match new_display[**row] {
                Slot::Live(id) => self
                    .pending_inserted
                    .get(&id)
                    .is_some_and(|pending| pending.claimed),
                Slot::Ghost(_) => false,
            })
            .copied()
            .collect();
        Some(DisplayEdits {
            edits,
            reloads,
            claimed,
            #[cfg(target_os = "macos")]
            old_keys,
            #[cfg(target_os = "macos")]
            new_keys,
        })
    }

    /// Retires the departed row `id` into its reserved ghost `key`: the
    /// configured cell's `LiveChild` comes out of the shared slot and
    /// `LiveChild::remove` retires it — `None` back means the table's own
    /// row delete carries the removal, so the reserved slot is released.
    fn retire_row(
        &mut self,
        id: ItemId,
        key: GhostKey,
        state: &Rc<RefCell<Self>>,
        table: &TableView,
        frozen: Option<f64>,
    ) {
        self.pending_inserted.remove(&id);
        let slot = self.live_rows.remove(&id).and_then(|weak| weak.upgrade());
        let child = slot.and_then(|slot| slot.borrow_mut().take());
        let Some(child) = child else {
            // `wants_ghost` required a configured, claimed child — an
            // absent one only means the cell's payload already dropped.
            self.display.remove_ghost(key);
            return;
        };
        let weak_state = Rc::downgrade(state);
        let weak_table = cocoa_ui::objc2::rc::Weak::new(table);
        let ctx = self.renderer.context();
        let removal = Removal {
            env: ctx.env(),
            // The table sizes the row: a list ghost freezes at its
            // measured height rather than a layout `GhostSlot`.
            slot: None,
            on_complete: Box::new(move || {
                cocoa_ui::main_queue::enqueue_local(
                    cocoa_ui::MainThreadMarker::new()
                        .expect("ghost completion runs on the main thread"),
                    move |_mtm| {
                        let (Some(state), Some(table)) = (weak_state.upgrade(), weak_table.load())
                        else {
                            return;
                        };
                        let index = {
                            let mut shared = state.borrow_mut();
                            #[cfg(target_os = "ios")]
                            let index = shared
                                .display_index_of(DisplayRow::Ghost(key))
                                .map(|(section, row)| IndexPath { section, row });
                            #[cfg(target_os = "macos")]
                            let index = shared
                                .flat_layout
                                .iter()
                                .position(|entry| *entry == FlatEntry::Ghost(key));
                            let Some(index) = index else {
                                return;
                            };
                            shared.display.remove_ghost(key);
                            shared.ghost_ids.remove(&key);
                            shared.ghosts.remove(&key);
                            index
                        };
                        // The ghost already finished visually — the row
                        // deletes with no animation so only siblings
                        // close the gap.
                        table.apply_row_updates(&[index], &[], &[], &[], false);
                        #[cfg(target_os = "macos")]
                        refresh_separators(&state, &table);
                    },
                );
            }),
        };
        if let Some(ghost) = child.remove(removal) {
            self.ghost_ids.insert(key, id);
            let height = frozen.unwrap_or_else(|| frozen_row_height(self, id, ghost.host()));
            self.ghosts.insert(key, (ghost, height));
        } else {
            self.display.remove_ghost(key);
        }
    }

    /// The insets the item asks for, or the theme's.
    #[cfg(target_os = "macos")]
    fn insets_for(&self, insets: Option<&EdgeInsets>) -> KitInsets {
        insets.map_or(self.theme_insets, kit_insets)
    }

    /// Whether the flat entry at `row` paints the hairline under it —
    /// every member but its section's last, matching `SwiftUI`'s inset
    /// list (water-rs/waterui#3592).
    #[cfg(target_os = "macos")]
    fn shows_separator(&self, row: usize) -> bool {
        let member = |entry: Option<&FlatEntry>| {
            matches!(entry, Some(FlatEntry::Row(_) | FlatEntry::Ghost(_)))
        };
        member(self.flat_layout.get(row)) && member(self.flat_layout.get(row + 1))
    }

    /// `id`'s flat index — linear, row lookups are short.
    fn flat_of_id(&self, id: ItemId) -> Option<usize> {
        self.item_ids.iter().position(|candidate| *candidate == id)
    }

    /// Measures `layout` at `width` and answers the row height.
    #[cfg(target_os = "macos")]
    fn measure_row(&self, layout: &dyn SubView, width: f64, insets: KitInsets) -> f64 {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "kit geometry is f64; layout proposals are f32"
        )]
        let proposal_width = (width > 0.0).then_some(width as f32);
        let content = layout.measure(ProposalSize::new(proposal_width, None)).size;
        row_height(f64::from(content.height), insets, self.resolved_min_height)
    }
}

/// What a mounted live row's payload keeps: the shared slot the row's
/// `LiveChild` occupies — a departure takes it into a ghost — plus the
/// guards its per-item signals return.
struct RowPayload {
    /// The row's `LiveChild`, shared with `Shared::live_rows` — a
    /// ghosted departure takes it out of here.
    #[allow(dead_code)]
    row: Rc<RefCell<Option<LiveChild>>>,
    /// Guards for the row's per-item watchers — dropped with the cell.
    #[allow(dead_code)]
    guards: Vec<BoxWatcherGuard>,
}

impl fmt::Debug for RowPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RowPayload").finish_non_exhaustive()
    }
}

/// Whether `metadata` carries an explicit animation — the `setEditing` /
/// reload animation switches read it.
fn metadata_animated(metadata: &Metadata) -> bool {
    metadata.try_get::<Animation>().is_some()
}

/// Coalesced `contents` notifications: each watch event records the newest
/// snapshot plus the stable ids needing re-materialization here — outside
/// `Shared`, so an emission that lands during row materialization never
/// touches its borrow. One scheduled main-queue flush applies the newest
/// recorded state; reentrant emissions land as the next batch and schedule
/// their own flush, so an older snapshot is never replayed after a newer.
#[derive(Default)]
struct PendingRows {
    scheduled: bool,
    snapshot: Option<AnyViewsSnapshot<ListItem>>,
    metadata: Option<Metadata>,
    dirty: HashSet<ItemId>,
}

impl PendingRows {
    /// Every recorded emission retains its own snapshot and marks the
    /// positions it touched: updates name occupants whose content changed
    /// and insertions may reintroduce a surviving id across coalesced
    /// emissions — both index this emission's own snapshot.
    fn record(
        &mut self,
        snapshot: AnyViewsSnapshot<ListItem>,
        metadata: Metadata,
        change: &CollectionChange,
    ) {
        self.dirty.extend(
            change
                .updates()
                .chain(change.insertions())
                .flatten()
                .filter_map(|index| snapshot.get_id(index)),
        );
        self.snapshot = Some(snapshot);
        self.metadata = Some(metadata);
    }

    /// Drains the pending batch; the flush owns delivery from here.
    fn take(&mut self) -> Option<(AnyViewsSnapshot<ListItem>, Metadata, HashSet<ItemId>)> {
        self.scheduled = false;
        self.snapshot.take().map(|snapshot| {
            (
                snapshot,
                self.metadata
                    .take()
                    .expect("snapshot and metadata are recorded together"),
                core::mem::take(&mut self.dirty),
            )
        })
    }

    /// Returns a batch the flush could not apply: anything recorded since
    /// the take is newer and stays authoritative for snapshot/metadata.
    fn merge_back(
        &mut self,
        snapshot: AnyViewsSnapshot<ListItem>,
        metadata: Metadata,
        dirty: HashSet<ItemId>,
    ) {
        self.dirty.extend(dirty);
        if self.snapshot.is_none() {
            self.snapshot = Some(snapshot);
            self.metadata = Some(metadata);
        }
    }
}

/// Schedules the single coalesced flush; events arriving while a flush is
/// already queued fold into its batch.
fn schedule_contents_flush(
    state: &Rc<RefCell<Shared>>,
    pending: &Rc<RefCell<PendingRows>>,
    table: &Retained<TableView>,
) {
    {
        let mut pending = pending.borrow_mut();
        if pending.scheduled {
            return;
        }
        pending.scheduled = true;
    }
    let state = Rc::clone(state);
    let pending = Rc::clone(pending);
    let table = cocoa_ui::objc2::rc::Weak::new(&**table);
    cocoa_ui::main_queue::enqueue_local(
        cocoa_ui::MainThreadMarker::new().expect("contents watcher runs on the main thread"),
        move |_mtm| {
            if let Some(table) = table.load() {
                apply_contents_change(&state, &pending, &table);
            }
        },
    );
}

/// `updateFromRust`: membership changes apply as batched row updates only
/// for a single plain section in a window; every other shape reloads.
/// `applyBindingSelection` runs afterward either way.
///
/// Applying a change may emit back into `contents` (row mounts mutate the
/// collection); those emissions record into `pending` and flush after this
/// apply, never overwriting it with older state.
fn apply_contents_change(
    state: &Rc<RefCell<Shared>>,
    pending: &Rc<RefCell<PendingRows>>,
    table: &Retained<TableView>,
) {
    let Some((snapshot, metadata, dirty)) = pending.borrow_mut().take() else {
        return;
    };
    let state_inner = Rc::clone(state);
    let pending_retry = Rc::clone(pending);
    let table_inner = table.clone();
    let metadata_retry = metadata.clone();
    crate::animation::with_platform_animation(&metadata, move || {
        let Ok(mut borrowed) = state_inner.try_borrow_mut() else {
            // `Shared` is still borrowed by materialization: the batch goes
            // back behind anything newer and the flush is rescheduled.
            pending_retry
                .borrow_mut()
                .merge_back(snapshot, metadata_retry, dirty);
            schedule_contents_flush(&state_inner, &pending_retry, &table_inner);
            return;
        };
        let new_ids = ids_snapshot(&snapshot);
        let (old_display, inserted) =
            borrowed.reconcile_display(&new_ids, &state_inner, &table_inner);
        let old_ids = core::mem::take(&mut borrowed.item_ids);
        borrowed.apply_snapshot(snapshot);
        let new_set: HashSet<ItemId> = borrowed.item_ids.iter().copied().collect();
        borrowed.live_rows.retain(|id, _| new_set.contains(id));
        #[cfg(target_os = "macos")]
        rebuild_flat_layout(&mut borrowed);
        borrowed.render_inserted(&inserted);
        let old_set: HashSet<ItemId> = old_ids.iter().copied().collect();
        // A dirty surviving id re-measures whatever the section shape —
        // evict before choosing the batched or whole-table path.
        for id in dirty.iter().filter(|id| old_set.contains(id)) {
            borrowed.measured_heights.remove(id);
        }
        let edits = borrowed.display_edits(&old_display, &dirty, &old_set);
        drop(borrowed);
        if table_inner.in_window()
            && let Some(edits) = edits
        {
            #[cfg(target_os = "ios")]
            {
                let DisplayEdits {
                    edits,
                    reloads,
                    claimed,
                    ..
                } = edits;
                let claimed_row = |row: usize| claimed.contains(&row);
                let row = |row| IndexPath { section: 0, row };
                let delete_paths: Vec<IndexPath> = edits.deletes.into_iter().map(row).collect();
                let (claimed, plain): (Vec<usize>, Vec<usize>) =
                    edits.inserts.into_iter().partition(|row| claimed_row(*row));
                let insert_paths: Vec<IndexPath> = plain.into_iter().map(row).collect();
                let claimed_paths: Vec<IndexPath> = claimed.into_iter().map(row).collect();
                let move_paths: Vec<(IndexPath, IndexPath)> = edits
                    .moves
                    .into_iter()
                    .map(|(from, to)| (row(from), row(to)))
                    .collect();
                table_inner.apply_row_updates(
                    &delete_paths,
                    &insert_paths,
                    &claimed_paths,
                    &move_paths,
                    true,
                );
                if !reloads.is_empty() {
                    let reload_paths: Vec<IndexPath> = reloads
                        .into_iter()
                        .map(|(_, new_row)| row(new_row))
                        .collect();
                    table_inner.reload_rows(&reload_paths, true);
                }
            }
            #[cfg(target_os = "macos")]
            {
                let DisplayEdits {
                    edits,
                    reloads,
                    claimed,
                    old_keys,
                    new_keys,
                } = edits;
                let claimed_row = |row: usize| claimed.contains(&row);
                // AppKit re-materializes a row as delete+insert: the
                // updated rows join the same update block so `viewFor`
                // rebuilds them, and so take no part in the moves.
                let mut deletes = edits.deletes;
                let (claimed, mut plain): (Vec<usize>, Vec<usize>) =
                    edits.inserts.into_iter().partition(|row| claimed_row(*row));
                deletes.extend(reloads.iter().map(|&(old_row, _)| old_row));
                plain.extend(reloads.iter().map(|&(_, new_row)| new_row));
                let removed: HashSet<ItemId> = deletes.iter().map(|&row| old_keys[row]).collect();
                let inserted: HashSet<ItemId> = plain
                    .iter()
                    .chain(claimed.iter())
                    .map(|&row| new_keys[row])
                    .collect();
                let moves = sequential_moves(&old_keys, &new_keys, &removed, &inserted);
                table_inner.apply_row_updates(&deletes, &moves, &plain, &claimed, true);
                refresh_separators(&state_inner, &table_inner);
            }
        } else {
            table_inner.reload_data();
        }
        // Entries no cell consumed belonged to rows inserted off screen:
        // they drop without playing, the way an unmaterialized lazy row
        // plays nothing.
        state_inner.borrow_mut().pending_inserted.clear();
        apply_binding_selection(&state_inner, &table_inner);
    });
}

/// `applyBindingSelection`: push the binding's selection onto the table
/// under the `applying` write-guard, animated `false`.
fn apply_binding_selection(state: &Rc<RefCell<Shared>>, table: &TableView) {
    let borrowed = state.borrow();
    if borrowed.applying_selection.get() {
        return;
    }
    borrowed.applying_selection.set(true);
    let selected = borrowed.selection.selected_ids();
    #[cfg(target_os = "ios")]
    {
        let current: HashSet<usize> = table
            .selected_index_paths()
            .iter()
            .filter_map(|ip| match borrowed.display_at(ip.section, ip.row) {
                Some(DisplayRow::Live(flat)) => Some(flat),
                _ => None,
            })
            .collect();
        let wanted: HashSet<usize> = selected
            .iter()
            .filter_map(|id| borrowed.flat_of_id(*id))
            .collect();
        for flat in &current {
            if !wanted.contains(flat)
                && let Some((section, row)) = borrowed.display_index_of(DisplayRow::Live(*flat))
            {
                table.deselect_row(IndexPath { section, row }, false);
            }
        }
        for flat in &wanted {
            if !current.contains(flat)
                && let Some((section, row)) = borrowed.display_index_of(DisplayRow::Live(*flat))
            {
                table.select_row(IndexPath { section, row }, false);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let wanted: Vec<usize> = selected
            .iter()
            .filter_map(|id| borrowed.flat_of_id(*id))
            .filter_map(|flat| {
                borrowed
                    .flat_layout
                    .iter()
                    .position(|entry| *entry == FlatEntry::Row(flat))
            })
            .collect();
        table.set_selected_rows(&wanted);
    }
    borrowed.applying_selection.set(false);
}

/// The `deletable` watch a row installs: `onDeletableChange` reloads the
/// row's index path on each change. The guard lives in the row's payload
/// and dies with it.
fn watch_deletable(
    deletable: &Computed<bool>,
    id: ItemId,
    state: &Rc<RefCell<Shared>>,
    table: &TableView,
) -> BoxWatcherGuard {
    let table = cocoa_ui::objc2::rc::Weak::new(table);
    let state = Rc::downgrade(state);
    deletable.watch(move |ctx| {
        let (Some(table), Some(state)) = (table.load(), state.upgrade()) else {
            return;
        };
        let flat = state.borrow().flat_of_id(id);
        let Some(flat) = flat else {
            return;
        };
        let animated = metadata_animated(ctx.metadata());
        #[cfg(target_os = "ios")]
        if let Some((section, row)) = state.borrow().display_index_of(DisplayRow::Live(flat)) {
            table.reload_rows(&[IndexPath { section, row }], animated);
        }
        #[cfg(target_os = "macos")]
        {
            let _ = animated;
            let row = state
                .borrow()
                .flat_layout
                .iter()
                .position(|entry| *entry == FlatEntry::Row(flat));
            if let Some(row) = row {
                table.note_height_changed(row..row + 1);
                table.reload_data();
            }
        }
    })
}

/// `UIKit`: `updateSeparatorInsets` — the separator's leading edge tracks
/// the leftmost text-bearing view inside the cell's content, falling back
/// to the leftmost content view.
#[cfg(target_os = "ios")]
fn update_separator_insets(cell: &TableCell) {
    let content_view = cell.contentView();
    let leaf = leftmost_text_x(&content_view).or_else(|| leftmost_content_x(&content_view));
    let leading = leaf.map_or(0.0, |view| {
        view::convert_point(&view, cocoa_ui::geometry::Point::new(0.0, 0.0), cell).x
    });
    cell.set_separator_inset(KitInsets {
        top: 0.0,
        left: leading,
        bottom: 0.0,
        right: 16.0,
    });
}

/// `leftmostTextX`: the first subview chain that lands on a kit text view
/// — the label classes — and the view it ends on.
#[cfg(target_os = "ios")]
fn leftmost_text_x(view: &cocoa_ui::PlatformView) -> Option<Retained<cocoa_ui::PlatformView>> {
    for subview in view::subviews(view) {
        let name = view::class_name(&subview);
        if matches!(
            name,
            "CocoaUiLabel" | "CocoaUiTextField" | "CocoaUiSecureTextField"
        ) {
            return Some(subview);
        }
        if let Some(found) = leftmost_text_x(&subview) {
            return Some(found);
        }
    }
    None
}

/// `leftmostContentX`: the same walk restricted to `WaterUI.`/`CocoaUi`
/// classes.
#[cfg(target_os = "ios")]
fn leftmost_content_x(view: &cocoa_ui::PlatformView) -> Option<Retained<cocoa_ui::PlatformView>> {
    for subview in view::subviews(view) {
        let name = view::class_name(&subview);
        if name.contains("WaterUI.") || name.starts_with("CocoaUi") {
            return Some(subview);
        }
        if let Some(found) = leftmost_content_x(&subview) {
            return Some(found);
        }
    }
    None
}

/// The list's `SubView` face: `sizeThatFits` answers the proposal where it
/// constrains, the table's content size where it does not — a scroll
/// surface that stretches to fill its slot.
struct ListSubView {
    table: Retained<TableView>,
}

impl fmt::Debug for ListSubView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListSubView").finish_non_exhaustive()
    }
}

impl SubView for ListSubView {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "kit geometry is f64; layout proposals are f32"
    )]
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        #[cfg(target_os = "ios")]
        let content = self.table.contentSize();
        #[cfg(target_os = "macos")]
        let content = self.table.fitting();
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(content.width as f32),
            proposal.height.unwrap_or(content.height as f32),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn priority(&self) -> i32 {
        0
    }
}

#[cfg(target_os = "ios")]
#[allow(clippy::wildcard_imports)]
mod platform_impl {
    use super::*;
    use cocoa_ui::objc2_ui_kit::{NSLayoutConstraint, UILayoutPriorityRequired};
    use cocoa_ui::uikit::TableHeaderFooterView;

    /// Insets resolved by this cell's current native layout.
    fn native_insets(cell: &TableCell) -> KitInsets {
        let margins = cell.contentView().directionalLayoutMargins();
        KitInsets {
            top: margins.top,
            bottom: margins.bottom,
            left: margins.leading,
            right: margins.trailing,
        }
    }

    /// The same resolved geometry drives the content constraints and row fitting.
    struct RowLayout {
        mounted: alloc::rc::Weak<RefCell<Option<LiveChild>>>,
        explicit_insets: Option<KitInsets>,
        applied_insets: Cell<KitInsets>,
        height: Retained<NSLayoutConstraint>,
        minimum: f64,
        disclosure: bool,
        id: ItemId,
        state: Rc<RefCell<Shared>>,
        table: cocoa_ui::objc2::rc::Weak<TableView>,
    }

    impl RowLayout {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "native points are f64; proposals are f32"
        )]
        fn update(&self, cell: &TableCell) {
            let Some(slot) = self.mounted.upgrade() else {
                return;
            };
            let slot = slot.borrow();
            let Some(child) = slot.as_ref() else {
                // The row's `LiveChild` left for a ghost — the retired
                // host stays pinned; nothing re-measures it.
                return;
            };
            let insets = self.explicit_insets.unwrap_or_else(|| native_insets(cell));
            if self.applied_insets.replace(insets) != insets {
                cell.configure(child.view(), insets, self.disclosure);
            }
            let width = cell.contentView().bounds().size.width - insets.left - insets.right;
            let proposal = ProposalSize::new((width > 0.0).then_some(width as f32), None);
            let measured = child.layout().measure(proposal).size;
            let contract = row_height(f64::from(measured.height), insets, self.minimum);
            let height = contract - insets.top - insets.bottom;
            if self.height.constant().to_bits() != height.to_bits() {
                self.height.setConstant(height);
            }
            // Publish the contract `heightForRow` reports. When the resolved
            // pass changes it — the first query runs before the cell's real
            // margins exist — re-ask the height on the next main-queue turn;
            // an updates batch inside `layoutSubviews` would re-enter layout.
            // The mutable borrow ends before `proposal::deliver`, whose
            // callbacks may synchronously re-enter the shared state.
            let stale = {
                let mut state = self.state.borrow_mut();
                state.measured_heights.insert(self.id, contract) != Some(contract)
            };
            if stale && let Some(table) = self.table.load() {
                cocoa_ui::main_queue::enqueue_local(
                    cocoa_ui::MainThreadMarker::new().expect("row updates run on the main thread"),
                    move |_mtm| table.apply_row_updates(&[], &[], &[], &[], false),
                );
            }
            proposal::deliver(child.view(), Placement::platform_hosted(proposal));
            update_separator_insets(cell);
        }
    }

    /// The `UIKit` data source + delegate: sections and rows answer from
    /// `Shared.groups`.
    pub(super) struct Source {
        state: Rc<RefCell<Shared>>,
    }

    impl Source {
        /// `state` is the list's shared state.
        pub(super) const fn new(state: Rc<RefCell<Shared>>) -> Self {
            Self { state }
        }

        /// `index`'s display member — a live flat index or a ghost key.
        fn slot(&self, index: IndexPath) -> DisplayRow {
            self.state
                .borrow()
                .display_at(index.section, index.row)
                .expect("index path lands inside the display rows")
        }
    }

    /// `accessibilityActivate` on a row: toggle its selection the way a
    /// tap would.
    fn activate_row(state: &Rc<RefCell<Shared>>, table: &TableView, index: IndexPath) {
        let selected: HashSet<usize> = {
            let borrowed = state.borrow();
            table
                .selected_index_paths()
                .iter()
                .filter_map(|ip| match borrowed.display_at(ip.section, ip.row) {
                    Some(DisplayRow::Live(flat)) => Some(flat),
                    _ => None,
                })
                .collect()
        };
        let Some(DisplayRow::Live(flat)) = state.borrow().display_at(index.section, index.row)
        else {
            return;
        };
        // A `.disabled` row is inert — accessibility activation must not
        // select it, the way a pointer press must not.
        if state.borrow().row_disabled(flat) {
            return;
        }
        if selected.contains(&flat) {
            table.deselect_row(index, false);
        } else {
            table.select_row(index, false);
        }
        write_from_table(state, table);
    }

    /// `writeSelectionFromTable`: the table's selected index paths as
    /// flats into `Shared::write_selection`.
    fn write_from_table(state: &Rc<RefCell<Shared>>, table: &TableView) {
        let borrowed = state.borrow();
        borrowed.write_selection(table.selected_index_paths().iter().filter_map(|ip| {
            match borrowed.display_at(ip.section, ip.row) {
                Some(DisplayRow::Live(flat)) => Some(flat),
                _ => None,
            }
        }));
    }

    impl TableSource for Source {
        fn sections(&self, _table: &TableView) -> usize {
            self.state.borrow().display_section_counts().len()
        }

        fn rows_in_section(&self, _table: &TableView, section: usize) -> usize {
            self.state.borrow().display_section_counts()[section]
        }

        fn configure_cell(&self, table: &TableView, cell: &TableCell, index: IndexPath) {
            let DisplayRow::Live(flat) = self.slot(index) else {
                let DisplayRow::Ghost(key) = self.slot(index) else {
                    return;
                };
                {
                    let state = self.state.borrow();
                    if let Some((ghost, _)) = state.ghosts.get(&key) {
                        let insets = pinned_insets(ghost.host());
                        cell.configure(ghost.host(), insets, false);
                    }
                }
                cell.set_layout_handler(|_| {});
                cell.set_activate_handler(|_| {});
                cell.set_payload(Box::new(GhostPayload(key)));
                return;
            };
            let (item_insets, deletable, leaf, insertion) = {
                let mut state = self.state.borrow_mut();
                let id = state.item_ids[flat];
                if let Some(pending) = state.pending_inserted.remove(&id) {
                    (pending.insets, pending.deletable, pending.leaf, true)
                } else {
                    drop(state);
                    let (insets, deletable, leaf) = self.state.borrow().render_row(flat);
                    (insets, deletable, leaf, false)
                }
            };
            let id = self.state.borrow().item_ids[flat];
            let explicit_insets = item_insets.as_ref().map(kit_insets);
            let insets = explicit_insets.unwrap_or_else(|| native_insets(cell));
            // Without the `navigation` port nothing tags the wrapper, so no
            // row can carry a link.
            #[cfg(feature = "navigation")]
            let shows_disclosure = contains_navigation_link(leaf.view());
            #[cfg(not(feature = "navigation"))]
            let shows_disclosure = false;
            let child = if insertion {
                LiveChild::mount_inserted(leaf, cell, None, &self.state.borrow().env)
            } else {
                LiveChild::mount(leaf, cell, None)
            };
            cell.configure(child.view(), insets, shows_disclosure);
            let height = child.view().heightAnchor().constraintEqualToConstant(0.0);
            // The measured layout is the row contract; keep UIKit from stretching
            // the hosted view while fitting the automatic row height.
            height.setPriority(UILayoutPriorityRequired);
            let cell_row = Rc::new(RefCell::new(Some(child)));
            self.state
                .borrow_mut()
                .live_rows
                .insert(id, Rc::downgrade(&cell_row));
            let layout = RowLayout {
                mounted: Rc::downgrade(&cell_row),
                explicit_insets,
                applied_insets: Cell::new(insets),
                height,
                minimum: self.state.borrow().resolved_min_height,
                disclosure: shows_disclosure,
                id,
                state: Rc::clone(&self.state),
                table: cocoa_ui::objc2::rc::Weak::new(table),
            };
            layout.update(cell);
            layout.height.setActive(true);
            // Cocoa invokes this after UITableViewCell's superclass layout, when
            // style, readable width and safe-area margins have been resolved.
            cell.set_layout_handler(move |cell| layout.update(cell));

            let guard = watch_deletable(&deletable, id, &self.state, table);
            cell.set_activate_handler({
                let state = Rc::clone(&self.state);
                let table = cocoa_ui::objc2::rc::Weak::new(table);
                move |cell| {
                    let Some(table) = table.load() else {
                        return;
                    };
                    if let Some(index) = table.index_path_for_cell(cell) {
                        activate_row(&state, &table, index);
                    }
                }
            });
            cell.set_payload(Box::new(RowPayload {
                row: cell_row,
                guards: vec![guard],
            }));
        }

        fn configure_header_footer(
            &self,
            _table: &TableView,
            view: &TableHeaderFooterView,
            kind: SectionKind,
            section: usize,
        ) -> bool {
            let computed = {
                let state = self.state.borrow();
                let Some(group) = state.groups.get(section) else {
                    return false;
                };
                match kind {
                    SectionKind::Header => group.label.clone(),
                    SectionKind::Footer => group.footer.clone(),
                }
            };
            let Some(signal) = computed else {
                return false;
            };
            let weak = cocoa_ui::objc2::rc::Weak::new(view);
            let apply = move |styled: &StyledStr| {
                let Some(view) = weak.load() else {
                    return;
                };
                crate::measure_memo::invalidate();
                let plain = cocoa_ui::text::strip_bidi_controls(&styled.to_plain());
                view.set_text(&objc2_foundation::NSString::from_str(&plain), kind);
            };
            apply(&signal.snapshot());
            let guard = signal.watch(move |ctx| apply(ctx.value()));
            view.set_payload(Box::new(vec![guard]));
            true
        }

        fn row_height(&self, table: &TableView, index: IndexPath) -> f64 {
            // The row contract is an explicit height: the platform's
            // automatic-dimension fitting prices the separator in, growing
            // every row by the separator's point over the contract. A
            // ghost answers its frozen height.
            let DisplayRow::Live(flat) = self.slot(index) else {
                let DisplayRow::Ghost(key) = self.slot(index) else {
                    return 0.0;
                };
                return self
                    .state
                    .borrow()
                    .ghosts
                    .get(&key)
                    .map_or(0.0, |(_, height)| *height);
            };
            let borrowed = self.state.borrow();
            let id = borrowed.item_ids[flat];
            if let Some(height) = borrowed.measured_heights.get(&id) {
                return *height;
            }
            // `heightForRow` fires before the cell exists, so the platform's
            // margins aren't readable off a `contentView` yet — the theme's
            // stock row insets answer the same vertical contract, and the
            // first layout pass writes back the resolved value.
            let (item_insets, _deletable, leaf) = borrowed.render_row(flat);
            let insets = item_insets.as_ref().map_or_else(
                || {
                    TableView::theme_row_insets(
                        cocoa_ui::MainThreadMarker::new()
                            .expect("row heights resolve on the main thread"),
                    )
                },
                kit_insets,
            );
            drop(borrowed);
            let width = table.bounds_width() - insets.left - insets.right;
            #[expect(
                clippy::cast_possible_truncation,
                reason = "native points are f64; proposals are f32"
            )]
            let proposal = ProposalSize::new((width > 0.0).then_some(width as f32), None);
            let measured = leaf.layout().measure(proposal).size;
            let contract = row_height(
                f64::from(measured.height),
                insets,
                self.state.borrow().resolved_min_height,
            );
            self.state
                .borrow_mut()
                .measured_heights
                .insert(id, contract);
            contract
        }

        fn section_header_height(&self, table: &TableView, section: usize) -> f64 {
            let anonymous_first = section == 0
                && self
                    .state
                    .borrow()
                    .groups
                    .first()
                    .is_some_and(|group| group.label.is_none());
            if !anonymous_first {
                return f64::NAN;
            }
            // Navigation owns the first scroll surface's top spacing. Later
            // independent lists keep the inset-grouped anonymous-section reserve.
            if is_navigation_scroll_surface(table) {
                table.set_section_header_top_padding(0.0);
                // Grouped tables interpret zero as their default header height.
                // The least positive native height suppresses the header instead.
                f64::MIN_POSITIVE
            } else {
                35.0
            }
        }

        fn is_row_deletable(&self, _table: &TableView, index: IndexPath) -> bool {
            let state = self.state.borrow();
            if state.on_delete.is_none() {
                return false;
            }
            let Some(DisplayRow::Live(flat)) = state.display_at(index.section, index.row) else {
                return false;
            };
            state
                .item_ids
                .get(flat)
                .and_then(|id| state.positions.get(id).copied())
                .and_then(|pos| state.snapshot.get_view(pos))
                .is_some_and(|item| item.get_deletable().snapshot())
        }

        fn delete_row(&self, table: &TableView, index: IndexPath) {
            let Some(DisplayRow::Live(flat)) =
                self.state.borrow().display_at(index.section, index.row)
            else {
                return;
            };
            {
                let mut state = self.state.borrow_mut();
                let id = state.item_ids.remove(flat);
                state.regroup();
                let ids = state.item_ids.clone();
                // The table's own delete animation carries this row, so
                // no ghost is reserved for it.
                state.display.reconcile(&ids, |_| false);
                if let Some(slot) = state.live_rows.remove(&id).and_then(|weak| weak.upgrade()) {
                    drop(slot.borrow_mut().take());
                }
            }
            table.reload_data();
            let state = self.state.borrow();
            if let Some(on_delete) = &state.on_delete {
                on_delete(&state.env, flat);
            }
        }

        fn can_move_row(&self, _table: &TableView, index: IndexPath) -> bool {
            let state = self.state.borrow();
            state.on_move.is_some()
                && state.groups.len() == 1
                && index.section == 0
                && matches!(
                    state.display_at(index.section, index.row),
                    Some(DisplayRow::Live(_))
                )
        }

        fn move_row(&self, _table: &TableView, from: IndexPath, to: IndexPath) {
            let (Some(DisplayRow::Live(source)), Some(DisplayRow::Live(destination))) = (
                self.state.borrow().display_at(from.section, from.row),
                self.state.borrow().display_at(to.section, to.row),
            ) else {
                return;
            };
            let mut state = self.state.borrow_mut();
            let id = state.item_ids.remove(source);
            state.item_ids.insert(destination, id);
            state.regroup();
            let ids = state.item_ids.clone();
            state.display.reconcile(&ids, |_| false);
            let adjusted = if destination > source {
                destination - 1
            } else {
                destination
            };
            if let Some(on_move) = &state.on_move {
                on_move(&state.env, Move::new(source, adjusted));
            }
        }

        fn should_select(&self, _table: &TableView, index: IndexPath) -> bool {
            let state = self.state.borrow();
            match state.display_at(index.section, index.row) {
                Some(DisplayRow::Live(flat)) => !state.row_disabled(flat),
                _ => false,
            }
        }

        fn did_select_row(&self, table: &TableView, index: IndexPath) {
            let Some(DisplayRow::Live(flat)) =
                self.state.borrow().display_at(index.section, index.row)
            else {
                return;
            };
            let state = self.state.borrow();
            if let Some(&id) = state.item_ids.get(flat)
                && state.selection.allows_multiple()
                && state.selection.selected_ids().contains(&id)
            {
                table.deselect_row(index, false);
            }
            drop(state);
            write_from_table(&self.state, table);
        }

        fn did_deselect_row(&self, table: &TableView, _index: IndexPath) {
            write_from_table(&self.state, table);
        }
    }
}

#[cfg(target_os = "macos")]
#[allow(clippy::wildcard_imports)]
mod platform_impl {
    use super::*;
    use waterui::theme::color::MutedForeground;

    /// The `AppKit` data source + delegate + drag source: flat entries
    /// answer from `Shared.flat_layout`.
    pub(super) struct Source {
        state: Rc<RefCell<Shared>>,
        mtm: cocoa_ui::MainThreadMarker,
    }

    impl Source {
        /// `state` is the list's shared state.
        pub(super) const fn new(
            state: Rc<RefCell<Shared>>,
            mtm: cocoa_ui::MainThreadMarker,
        ) -> Self {
            Self { state, mtm }
        }

        /// The muted-foreground `WorkingColor` as an `AppKit` color.
        fn platform_color(
            color: &waterui::graphics::color::WorkingColor,
        ) -> Retained<cocoa_ui::objc2_app_kit::NSColor> {
            {
                let [red, green, blue, alpha] = color.components;
                cocoa_ui::appkit::colors::extended_linear_display_p3(
                    f64::from(red),
                    f64::from(green),
                    f64::from(blue),
                    f64::from(alpha),
                )
            }
        }

        /// A `ResolvedFont` as an `AppKit` font: the system face at the
        /// resolved size and weight.
        fn platform_font(
            &self,
            resolved: &waterui::text::font::ResolvedFont,
        ) -> Retained<cocoa_ui::Font> {
            cocoa_ui::font::system(
                self.mtm,
                f64::from(resolved.size()),
                platform_weight(resolved.weight()),
            )
        }

        /// Builds a section band view for `signal`, styled muted +
        /// caption/footnote — `WuiListSectionHeaderView`.
        /// `viewFor` for a live row: consume the pending insertion or
        /// render fresh, mount through `LiveChild`, and answer the row's
        /// `RowContainer`.
        fn live_row_view(
            &self,
            table: &TableView,
            flat: usize,
        ) -> Retained<cocoa_ui::PlatformView> {
            let id = self.state.borrow().item_ids[flat];
            let (item_insets, deletable, leaf, insertion) = {
                let mut state = self.state.borrow_mut();
                if let Some(pending) = state.pending_inserted.remove(&id) {
                    (pending.insets, pending.deletable, pending.leaf, true)
                } else {
                    drop(state);
                    let (insets, deletable, leaf) = self.state.borrow().render_row(flat);
                    (insets, deletable, leaf, false)
                }
            };
            let insets = self.state.borrow().insets_for(item_insets.as_ref());
            // `viewFor` is where a row's height lands in
            // `measuredRowHeights` — measure against the table's
            // current width.
            let width = view::bounds(table).size.width - insets.left - insets.right;
            let measured = self
                .state
                .borrow()
                .measure_row(leaf.layout(), width, insets);
            self.state
                .borrow_mut()
                .measured_heights
                .insert(id, measured);

            let editing = self.state.borrow().editing.snapshot();
            let row_deletable = deletable.snapshot();
            let shows_delete = editing && self.state.borrow().on_delete.is_some() && row_deletable;

            let container = RowContainer::new(self.mtm);
            let child = if insertion {
                LiveChild::mount_inserted(leaf, &container, None, &self.state.borrow().env)
            } else {
                LiveChild::mount(leaf, &container, None)
            };
            let delete = shows_delete.then(|| DeleteButton {
                title: "Delete".to_string(),
                handler: Box::new({
                    let state = Rc::clone(&self.state);
                    let table = cocoa_ui::objc2::rc::Weak::new(table);
                    let mtm = self.mtm;
                    move || {
                        if let Some(table) = table.load() {
                            let source = Self::new(Rc::clone(&state), mtm);
                            source.delete_at(&table, id);
                        }
                    }
                }),
            });
            container.configure(self.mtm, child.view(), insets, delete);
            // The row's slot is constraint-resolved inside `insets`
            // and, while editing, the inline delete button —
            // deliver that width, again on every layout pass, so a
            // proposal-driven child tracks its true slot across
            // resizes and the button's appearance.
            container.set_layout_handler(move |container| {
                let Some(content) = container.content() else {
                    return;
                };
                let width = view::bounds(&content).size.width;
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "kit geometry is f64; layout proposals are f32"
                )]
                proposal::deliver(
                    &content,
                    Placement::platform_hosted(ProposalSize::new(Some(width as f32), None)),
                );
            });
            #[expect(
                clippy::cast_possible_truncation,
                reason = "kit geometry is f64; layout proposals are f32"
            )]
            proposal::deliver(
                child.view(),
                Placement::platform_hosted(ProposalSize::new(Some(width as f32), None)),
            );
            let cell_row = Rc::new(RefCell::new(Some(child)));
            self.state
                .borrow_mut()
                .live_rows
                .insert(id, Rc::downgrade(&cell_row));
            let guard = watch_deletable(&deletable, id, &self.state, table);
            container.set_payload(Box::new(RowPayload {
                row: cell_row,
                guards: vec![guard],
            }));
            view::retain_base(&*container)
        }

        fn band_view(
            &self,
            kind: SectionKind,
            signal: &Computed<StyledStr>,
        ) -> Retained<cocoa_ui::PlatformView> {
            let band = SectionHeader::new(self.mtm, kind);
            let weak = cocoa_ui::objc2::rc::Weak::from_retained(&band);
            let apply = move |styled: &StyledStr| {
                let Some(band) = weak.load() else {
                    return;
                };
                crate::measure_memo::invalidate();
                let plain = cocoa_ui::text::strip_bidi_controls(&styled.to_plain());
                band.set_text(&objc2_foundation::NSString::from_str(&plain));
            };
            apply(&signal.snapshot());
            let text_guard = signal.watch(move |ctx| apply(ctx.value()));

            let muted = MutedForeground.resolve(&self.state.borrow().env);
            band.set_text_color(&Self::platform_color(&muted.snapshot()));
            let weak = cocoa_ui::objc2::rc::Weak::from_retained(&band);
            let color_guard = muted.watch(move |ctx| {
                let Some(band) = weak.load() else {
                    return;
                };
                band.set_text_color(&Self::platform_color(ctx.value()));
            });

            let font = match kind {
                SectionKind::Header => waterui::text::font::Caption
                    .resolve(&self.state.borrow().env)
                    .snapshot(),
                SectionKind::Footer => waterui::text::font::Footnote
                    .resolve(&self.state.borrow().env)
                    .snapshot(),
            };
            band.set_font(&self.platform_font(&font));
            let guards: Vec<BoxWatcherGuard> = vec![Box::new(text_guard), Box::new(color_guard)];
            band.set_payload(Box::new(guards));
            view::retain_base(&*band)
        }

        /// The delete action a row's inline button fires — the same
        /// sequence `commit editingStyle .delete` runs.
        fn delete_at(&self, table: &TableView, id: ItemId) {
            let flat = {
                let borrowed = self.state.borrow();
                borrowed.flat_of_id(id)
            };
            let Some(flat) = flat else {
                return;
            };
            {
                let mut state = self.state.borrow_mut();
                let id = state.item_ids.remove(flat);
                state.regroup();
                let ids = state.item_ids.clone();
                // The table's own delete animation carries this row, so
                // no ghost is reserved for it.
                state.display.reconcile(&ids, |_| false);
                if let Some(slot) = state.live_rows.remove(&id).and_then(|weak| weak.upgrade()) {
                    drop(slot.borrow_mut().take());
                }
                rebuild_flat_layout(&mut state);
            }
            table.reload_data();
            let state = self.state.borrow();
            if let Some(on_delete) = &state.on_delete {
                on_delete(&state.env, flat);
            }
        }
    }

    impl TableSource for Source {
        fn rows(&self, _table: &TableView) -> usize {
            self.state.borrow().flat_layout.len()
        }

        fn view_for_row(
            &self,
            table: &TableView,
            row: usize,
        ) -> Option<Retained<cocoa_ui::PlatformView>> {
            let entry = *self.state.borrow().flat_layout.get(row)?;
            match entry {
                FlatEntry::Header(section) => {
                    let signal = self
                        .state
                        .borrow()
                        .groups
                        .get(section)
                        .and_then(|group| group.label.clone());
                    signal.map(|signal| self.band_view(SectionKind::Header, &signal))
                }
                FlatEntry::Footer(section) => {
                    let signal = self
                        .state
                        .borrow()
                        .groups
                        .get(section)
                        .and_then(|group| group.footer.clone());
                    signal.map(|signal| self.band_view(SectionKind::Footer, &signal))
                }
                FlatEntry::Ghost(key) => {
                    let container = RowContainer::new(self.mtm);
                    {
                        let state = self.state.borrow();
                        if let Some((ghost, _)) = state.ghosts.get(&key) {
                            let insets = pinned_insets(ghost.host());
                            container.configure(self.mtm, ghost.host(), insets, None);
                        }
                    }
                    container.set_payload(Box::new(GhostPayload(key)));
                    Some(view::retain_base(&*container))
                }
                FlatEntry::Row(flat) => Some(self.live_row_view(table, flat)),
            }
        }

        fn row_view(&self, _table: &TableView, row: usize) -> Option<Retained<TableRowView>> {
            let shows_separator = {
                let state = self.state.borrow();
                let entry = *state.flat_layout.get(row)?;
                if !matches!(entry, FlatEntry::Row(_) | FlatEntry::Ghost(_)) {
                    return None;
                }
                state.shows_separator(row)
            };
            let row_view = TableRowView::new(self.mtm);
            row_view.set_shows_separator(shows_separator);
            row_view.set_separator_handler(Some(Rc::new(|row_view| {
                // `resolvedSeparatorLeading`: the leftmost text x inside
                // the row's cell view — 16 + the insets' surplus over the
                // row-content inset.
                let leading = view::subviews(row_view)
                    .first()
                    .and_then(|cell| leftmost_text_x_appkit(cell))
                    .map_or(16.0, |x| 16.0 + (x - 10.0).max(0.0));
                (leading, 16.0)
            })));
            Some(row_view)
        }

        fn row_height(&self, table: &TableView, row: usize) -> f64 {
            let (flat, id, insets) = {
                let state = self.state.borrow();
                match state.flat_layout.get(row) {
                    Some(FlatEntry::Header(_)) => return 38.0,
                    Some(FlatEntry::Footer(_)) => return 32.0,
                    Some(FlatEntry::Ghost(key)) => {
                        return state.ghosts.get(key).map_or(30.0, |(_, height)| *height);
                    }
                    Some(FlatEntry::Row(flat)) => {
                        let flat = *flat;
                        let Some(id) = state.item_ids.get(flat).copied() else {
                            return 30.0;
                        };
                        if let Some(height) = state.measured_heights.get(&id) {
                            return *height;
                        }
                        let Some(pos) = state.positions.get(&id).copied() else {
                            return 30.0;
                        };
                        let Some(item) = state.snapshot.get_view(pos) else {
                            return 30.0;
                        };
                        (flat, id, state.insets_for(item.get_insets()))
                    }
                    None => return 30.0,
                }
            };
            // `tableView:heightOfRow:` fires before `viewFor:` for a row,
            // so the measurement cache `viewFor` fills cannot be the only
            // source — measure the rendered content on demand, the same
            // arithmetic `viewFor` stores back. The row renders through
            // `render_row`, as every other row path does: it resolves the
            // row's environment wrapper (the selection theming) so the
            // row's own `.transition(...)` stays at the leaf root, unmounted.
            let (_insets, _deletable, leaf) = self.state.borrow().render_row(flat);
            let width = view::bounds(table).size.width - insets.left - insets.right;
            let mut state = self.state.borrow_mut();
            let measured = state.measure_row(leaf.layout(), width, insets);
            state.measured_heights.insert(id, measured);
            measured
        }

        fn is_group_row(&self, _table: &TableView, row: usize) -> bool {
            !matches!(
                self.state.borrow().flat_layout.get(row),
                Some(FlatEntry::Row(_) | FlatEntry::Ghost(_))
            )
        }

        fn should_select(&self, _table: &TableView, row: usize) -> bool {
            let state = self.state.borrow();
            state.selection.allows_selection()
                && match state.flat_layout.get(row) {
                    Some(&FlatEntry::Row(flat)) => !state.row_disabled(flat),
                    _ => false,
                }
        }

        fn selection_did_change(&self, table: &TableView) {
            let state = self.state.borrow();
            let selected = table.selected_rows();
            state.write_selection(selected.iter().filter_map(
                |row| match state.flat_layout.get(*row) {
                    Some(FlatEntry::Row(flat)) => Some(*flat),
                    _ => None,
                },
            ));
        }

        fn dragged_payload(&self, _table: &TableView, row: usize) -> Option<String> {
            let entry = *self.state.borrow().flat_layout.get(row)?;
            let FlatEntry::Row(flat) = entry else {
                return None;
            };
            Some(flat.to_string())
        }

        fn validate_drop(&self, _table: &TableView, row: usize, on_row: bool) -> bool {
            let state = self.state.borrow();
            !on_row
                && state.on_move.is_some()
                && matches!(state.flat_layout.get(row), Some(FlatEntry::Row(_)))
        }

        fn accept_drop(&self, table: &TableView, row: usize, payload: &str) -> bool {
            let Ok(source) = payload.parse::<usize>() else {
                return false;
            };
            let Some(FlatEntry::Row(destination)) =
                self.state.borrow().flat_layout.get(row).copied()
            else {
                return false;
            };
            let mut state = self.state.borrow_mut();
            let Some(id) = state.item_ids.get(source).copied() else {
                return false;
            };
            state.item_ids.remove(source);
            let destination = destination.min(state.item_ids.len());
            let adjusted = if destination > source {
                destination - 1
            } else {
                destination
            };
            state.item_ids.insert(destination, id);
            state.regroup();
            let ids = state.item_ids.clone();
            state.display.reconcile(&ids, |_| false);
            rebuild_flat_layout(&mut state);
            drop(state);
            table.reload_data();
            let state = self.state.borrow();
            if let Some(on_move) = &state.on_move {
                on_move(&state.env, Move::new(source, adjusted));
            }
            true
        }
    }

    /// `AppKit`'s `leftmostTextX` — the separator leading guide reads the
    /// leftmost text-bearing view inside the cell view.
    fn leftmost_text_x_appkit(view: &cocoa_ui::PlatformView) -> Option<f64> {
        for subview in view::subviews(view) {
            let name = view::class_name(&subview);
            if matches!(
                name,
                "CocoaUiLabel" | "CocoaUiTextField" | "CocoaUiSecureTextField"
            ) {
                return Some(view::frame(&subview).origin.x);
            }
            if let Some(found) = leftmost_text_x_appkit(&subview) {
                return Some(found + view::frame(&subview).origin.x);
            }
        }
        None
    }
}

/// Maps a `FontWeight` to the kit's weight scale — the same table
/// `date_picker` uses.
#[cfg(target_os = "macos")]
const fn platform_weight(weight: waterui::text::font::FontWeight) -> f64 {
    use cocoa_ui::font::weight;
    use waterui::text::font::FontWeight;
    match weight {
        FontWeight::Thin => weight::THIN,
        FontWeight::UltraLight => weight::ULTRA_LIGHT,
        FontWeight::Light => weight::LIGHT,
        FontWeight::Normal => weight::REGULAR,
        FontWeight::Medium => weight::MEDIUM,
        FontWeight::SemiBold => weight::SEMI_BOLD,
        FontWeight::Bold => weight::BOLD,
        FontWeight::UltraBold => weight::HEAVY,
        FontWeight::Black => weight::BLACK,
    }
}

/// Closes an animated request's distance to the approach bound before
/// the animation starts. The current item is the first whose row bottom
/// lies below the viewport's top — read from the offset, so a viewport
/// showing only a section header or footer still has one. A `target` item
/// further than
/// [`ANIMATED_ROW_SCROLL_APPROACH`](waterui_backend_core::scroll::ANIMATED_ROW_SCROLL_APPROACH)
/// from it is jumped to within the bound through the unanimated row jump
/// — which also ends any flight in progress — so the animation glides
/// over the final stretch only.
fn approach(state: &Rc<RefCell<Shared>>, table: &TableView, target: usize) {
    // Flatten under a short borrow, released before reading row geometry
    // and jumping — both can measure rows, which borrows `state` mutably.
    // Each item's row, indexed by flat item.
    #[cfg(target_os = "ios")]
    let rows: Vec<IndexPath> = state.borrow().live_index_paths();
    #[cfg(target_os = "macos")]
    let rows: Vec<usize> = state
        .borrow()
        .flat_layout
        .iter()
        .enumerate()
        .filter_map(|(row, entry)| matches!(entry, FlatEntry::Row(_)).then_some(row))
        .collect();
    let top = table.viewport_top();
    let current = rows.partition_point(|&row| table.row_bottom(row) <= top);
    let Some(item) = animated_row_scroll_approach(current, target) else {
        return;
    };
    #[cfg(target_os = "ios")]
    {
        table.scroll_to_row(rows[item], false);
        table.layout_if_needed();
    }
    #[cfg(target_os = "macos")]
    table.scroll_row_to_top(rows[item]);
}

/// Wires a `ScrollController<usize>` into the table: a generation bump
/// scrolls the target row to the top — with the request's animation, if
/// it carries one: `Animation::Default` is the native animated row
/// scroll, and an explicit animation drives the offset on the frame
/// clock along the core curve, landing where the jump would have. An
/// animated request first closes in through [`approach`], so a far
/// target animates only its final stretch.
fn wire_controller(
    leaf: &mut NativeLeaf,
    table: &Retained<TableView>,
    state: &Rc<RefCell<Shared>>,
    controller: &ScrollController<usize>,
) {
    let request = controller.request();
    let generation = controller.generation();
    leaf.watch(&request, |_| {});
    let apply = |state: &Rc<RefCell<Shared>>, table: &TableView, request: &ScrollRequest<usize>| {
        // Resolve the row under a short borrow and release it before the
        // scroll call: driving the table can run `layoutSubviews` → row
        // measuring, which borrows `state` again — mutably.
        #[cfg(target_os = "ios")]
        let target = state
            .borrow()
            .display_index_of(DisplayRow::Live(request.target))
            .map(|(section, row)| IndexPath { section, row });
        #[cfg(target_os = "macos")]
        let target = state
            .borrow()
            .flat_layout
            .iter()
            .position(|entry| *entry == FlatEntry::Row(request.target));
        let Some(target) = target else {
            return;
        };
        #[cfg(target_os = "ios")]
        table.layout_if_needed();
        if request.animation.is_some() {
            approach(state, table, request.target);
        }
        match request.animation.as_ref() {
            #[cfg(target_os = "ios")]
            None => table.scroll_to_row(target, false),
            #[cfg(target_os = "ios")]
            Some(Animation::Default) => table.scroll_to_row(target, true),
            #[cfg(target_os = "ios")]
            Some(animation) => {
                table.animate_scroll_to_row(
                    target,
                    animation.duration().as_secs_f64(),
                    crate::animation::progress(animation),
                );
            }
            #[cfg(target_os = "macos")]
            None => table.scroll_row_to_top(target),
            #[cfg(target_os = "macos")]
            Some(Animation::Default) => table.scroll_row_to_top_animated(target),
            #[cfg(target_os = "macos")]
            Some(animation) => {
                table.animate_scroll_row_to_top(
                    target,
                    animation.duration().as_secs_f64(),
                    crate::animation::progress(animation),
                );
            }
        }
    };
    if generation.snapshot() > 0 {
        apply(state, table, &request.snapshot());
    }
    leaf.watch(&generation, {
        let state = Rc::downgrade(state);
        let table = cocoa_ui::objc2::rc::Weak::from_retained(table);
        move |ctx| {
            if *ctx.value() > 0
                && let (Some(state), Some(table)) = (state.upgrade(), table.load())
            {
                apply(&state, &table, &request.snapshot());
            }
        }
    });
}

/// Renders a `ListConfig` into the kit's table surface.
#[expect(
    clippy::too_many_lines,
    reason = "one render pass: state init, source wiring, watches"
)]
fn render(mut config: ListConfig, ctx: &RenderContext<'_>) -> NativeLeaf {
    let mtm = ctx.mtm();
    let table = TableView::new(mtm);
    // A navigation split marks its sidebar column with `SidebarContent`:
    // the table takes the source-list chrome — clear background over the
    // split's material, and the unemphasised selection rather than the
    // accent fill (water-rs/waterui#1495).
    #[cfg(target_os = "macos")]
    if ctx
        .env()
        .get::<waterui::component::list::SidebarContent>()
        .is_some()
    {
        table.set_sidebar(true);
    }
    #[cfg(target_os = "ios")]
    let stock_height = {
        table.setSelfSizingInvalidation(
            cocoa_ui::objc2_ui_kit::UITableViewSelfSizingInvalidation::EnabledIncludingConstraints,
        );
        table.stock_row_height()
    };
    #[cfg(target_os = "macos")]
    let (theme_insets, stock_height) = (
        KitInsets {
            top: 4.0,
            bottom: 4.0,
            left: 10.0,
            right: 10.0,
        },
        24.0,
    );
    let state = Rc::new(RefCell::new(Shared {
        contents: config.contents().clone(),
        snapshot: config.contents().snapshot(),
        positions: HashMap::new(),
        env: ctx.env().clone(),
        renderer: ctx.renderer(),
        item_ids: Vec::new(),
        display: SlotSequence::new(Vec::new()),
        live_rows: HashMap::new(),
        ghosts: HashMap::new(),
        ghost_ids: HashMap::new(),
        pending_inserted: HashMap::new(),
        groups: Vec::new(),
        #[cfg(target_os = "macos")]
        flat_layout: Vec::new(),
        selection: match &config.selection() {
            ListSelection::None => SelectionMode::None,
            ListSelection::Single(binding) => SelectionMode::Single(binding.clone()),
            ListSelection::Multiple(binding) => SelectionMode::Multiple(binding.clone()),
        },
        applying_selection: Cell::new(false),
        #[cfg(target_os = "macos")]
        editing: config.editing().clone(),
        on_delete: config.take_on_delete(),
        on_move: config.take_on_move(),
        uses_sections: config.uses_sections(),
        #[cfg(target_os = "macos")]
        theme_insets,
        resolved_min_height: min_row_height(config.min_row_height(), stock_height),
        measured_heights: HashMap::new(),
        disabled_rows: RefCell::new(HashMap::new()),
    }));

    #[cfg(target_os = "ios")]
    {
        let borrowed = state.borrow();
        table.set_allows_selection(borrowed.selection.allows_selection());
        table.set_allows_multiple_selection(borrowed.selection.allows_multiple());
    }
    #[cfg(target_os = "macos")]
    if state.borrow().on_move.is_some() {
        table.set_drag_types(&["dev.waterui.listitem"]);
    }

    #[cfg(target_os = "ios")]
    let source = platform_impl::Source::new(Rc::clone(&state));
    #[cfg(target_os = "macos")]
    let source = platform_impl::Source::new(Rc::clone(&state), mtm);
    table.set_source(Rc::new(source));

    // Initial load — `reloadChildrenFromRust`. The display order seeds
    // from the initial membership: nothing plays on the first build, so
    // every row mounts plain.
    {
        let snapshot = state.borrow().snapshot.clone();
        let mut borrowed = state.borrow_mut();
        borrowed.apply_snapshot(snapshot);
        let initial_ids = borrowed.item_ids.clone();
        borrowed.display.reconcile(&initial_ids, |_| false);
        #[cfg(target_os = "macos")]
        rebuild_flat_layout(&mut borrowed);
        drop(borrowed);
        table.reload_data();
    }

    // `AppKit`: `layout`/`tileIfNeeded` — a width change tracks the column
    // and re-asks row heights.
    #[cfg(target_os = "macos")]
    {
        let last_width = Rc::new(Cell::new(-1.0f64));
        table.set_layout_handler({
            move |table| {
                let width = view::bounds(table).size.width;
                if (width - last_width.get()).abs() > 0.5 {
                    last_width.set(width);
                    if let Some(column) = table.table_view().tableColumns().firstObject() {
                        column.setWidth(width);
                    }
                    table.note_height_changed(0..table.row_count());
                    table.reload_data();
                }
            }
        });
    }

    let mut leaf = NativeLeaf::new(
        as_view(&table),
        ListSubView {
            table: table.clone(),
        },
    );

    // Membership watch — `watchAnyViewsIds` / `updateFromRust`. The callback
    // only records into `pending` (no `Shared` borrow, so reentrant
    // emissions during materialization are safe) and schedules the flush.
    let pending = Rc::new(RefCell::new(PendingRows::default()));
    let watcher = state.borrow().contents.watch(.., {
        let state = Rc::clone(&state);
        let pending = Rc::clone(&pending);
        let table = table.clone();
        move |ctx, change| {
            let snapshot = ctx.value().clone();
            let metadata = ctx.metadata().clone();
            pending.borrow_mut().record(snapshot, metadata, change);
            schedule_contents_flush(&state, &pending, &table);
        }
    });
    leaf.keep(watcher);

    // `editing` pushes `setEditing(animated:)`; AppKit shows its inline
    // delete buttons via the per-row `deletable` watch instead.
    #[cfg(target_os = "ios")]
    leaf.watch(config.editing(), {
        let table = table.clone();
        move |ctx| {
            table.set_editing(*ctx.value(), metadata_animated(ctx.metadata()));
        }
    });

    // Binding → table selection.
    match &config.selection() {
        ListSelection::None => {}
        ListSelection::Single(binding) => {
            leaf.watch(binding, {
                let state = Rc::clone(&state);
                let table = table.clone();
                move |_| apply_binding_selection(&state, &table)
            });
        }
        ListSelection::Multiple(binding) => {
            leaf.watch(binding, {
                let state = Rc::clone(&state);
                let table = table.clone();
                move |_| apply_binding_selection(&state, &table)
            });
        }
    }

    if let Some(controller) = config.scroll_controller() {
        wire_controller(&mut leaf, &table, &state, controller);
        wire_position_report(&mut leaf, &table, controller, ctx.mtm());
    }

    leaf.keep(state);
    leaf
}

/// `UIKit`: the table is itself the scroll surface — every offset change
/// (`scrollViewDidScroll:`) and every layout pass (bounds, content-size and
/// adjusted-inset changes all land there) reports the completed geometry:
/// the offset normalized by the adjusted content insets, the viewport
/// excluding them, the content extent as `UIKit` keeps it. Writes dedupe
/// against the sink's current value; nothing reads a pending request.
#[cfg(target_os = "ios")]
#[expect(
    clippy::cast_possible_truncation,
    reason = "kit geometry is f64; the report binding is f32"
)]
fn wire_position_report(
    leaf: &mut NativeLeaf,
    table: &Retained<TableView>,
    controller: &ScrollController<usize>,
    _mtm: cocoa_ui::MainThreadMarker,
) {
    let position = controller.position_report();
    // Nothing publishes until the first `layoutSubviews` — the surface's
    // geometry before then is placeholder, not a measured position.
    let measured = Rc::new(Cell::new(false));
    let report = {
        let measured = Rc::clone(&measured);
        move |table: &TableView| {
            if !measured.get() {
                return;
            }
            let inset = table.adjustedContentInset();
            let raw = table.contentOffset();
            let bounds = table.bounds().size;
            let extent = table.contentSize();
            let current = ScrollPosition::new(
                Axis::Vertical,
                Point::new((raw.x + inset.left) as f32, (raw.y + inset.top) as f32),
                Size::new(
                    (bounds.width - inset.left - inset.right).max(0.0) as f32,
                    (bounds.height - inset.top - inset.bottom).max(0.0) as f32,
                ),
                Size::new(extent.width as f32, extent.height as f32),
            );
            if position.snapshot() != Some(current) {
                position.set(Some(current));
            }
        }
    };
    table.on_scroll({
        let report = report.clone();
        move |table| report(table)
    });
    table.set_layout_handler({
        let measured = Rc::clone(&measured);
        let report = report.clone();
        move |table| {
            measured.set(true);
            report(table);
        }
    });
    // Clears the source, scroll and layout slots — and cancels the owned
    // flight — at the leaf's release boundary.
    leaf.keep(cocoa_ui::HandlerTeardown::new(Retained::clone(table)));
}

/// `AppKit`: `ListTableView` is itself the `NSScrollView` — its clip bounds
/// are the offset and viewport, its `InnerTable` document's frame the
/// content extent. Scrolls and resizes reach the reporter through the
/// clip's bounds-changed notifications; reflow through the document's
/// frame-changed notifications, whose first delivery is also the first
/// real layout — nothing publishes before it. Writes dedupe against the
/// sink's current value; nothing reads a pending request. Both
/// observations are leaf-kept, so they unregister at release.
#[cfg(target_os = "macos")]
#[expect(
    clippy::cast_possible_truncation,
    reason = "kit geometry is f64; the report binding is f32"
)]
fn wire_position_report(
    leaf: &mut NativeLeaf,
    table: &Retained<TableView>,
    controller: &ScrollController<usize>,
    mtm: cocoa_ui::MainThreadMarker,
) {
    // `table` is itself the `NSScrollView` — `ListTableView` wraps its
    // `InnerTable` document — so it is observed directly, not through an
    // ancestor lookup. The document's frame is the content extent; its
    // first frame change is the first real layout, and nothing publishes
    // before it.
    let scroll_view = table
        .clone()
        .downcast::<cocoa_ui::scroll::ScrollView>()
        .ok()
        .expect("apple list: ListTableView is an NSScrollView");
    let document = table.table_view();
    let position = controller.position_report();
    let measured = Rc::new(Cell::new(false));
    let report = {
        let scroll_view = scroll_view.clone();
        let document = document.clone();
        let measured = Rc::clone(&measured);
        move || {
            if !measured.get() {
                return;
            }
            let bounds = scroll_view.contentView().bounds();
            let extent = document.frame().size;
            let current = ScrollPosition::new(
                Axis::Vertical,
                Point::new(bounds.origin.x as f32, bounds.origin.y as f32),
                Size::new(bounds.size.width as f32, bounds.size.height as f32),
                Size::new(extent.width as f32, extent.height as f32),
            );
            if position.snapshot() != Some(current) {
                position.set(Some(current));
            }
        }
    };
    leaf.keep(cocoa_ui::scroll::observe_scroll_viewport(&scroll_view, {
        let report = report.clone();
        move || report()
    }));
    document.setPostsFrameChangedNotifications(true);
    leaf.keep(cocoa_ui::notification::observe_object(
        mtm,
        // SAFETY: reads a notification-name constant `AppKit` owns for the
        // process's lifetime.
        &cocoa_ui::notification::NotificationName::framework(unsafe {
            cocoa_ui::objc2_app_kit::NSViewFrameDidChangeNotification
        }),
        document.as_ref(),
        {
            let measured = Rc::clone(&measured);
            move || {
                measured.set(true);
                report();
            }
        },
    ));
}

/// Installs the `list` handler on the dispatcher: `Native<ListConfig>` →
/// the platform's table surface.
pub fn install(dispatcher: &mut crate::dispatch::Dispatcher) {
    dispatcher.register_native::<ListConfig>(render);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ids(raw: &[i32]) -> Vec<ItemId> {
        raw.iter()
            .map(|id| RawId::try_from(*id).expect("test ids are non-zero"))
            .collect()
    }

    #[test]
    fn single_section_row_diff_computes_membership_changes() {
        // 2 removed (index 1), 3 and 4 inserted (indexes 0 and 2).
        let edits =
            single_section_row_diff(&ids(&[1, 2]), &ids(&[3, 1, 4])).expect("ids are unique");
        assert_eq!(
            edits,
            RowEdits {
                deletes: vec![1],
                inserts: vec![0, 2],
                moves: Vec::new(),
            }
        );
    }

    #[test]
    fn single_section_row_diff_reports_a_reorder_as_moves() {
        let edits =
            single_section_row_diff(&ids(&[1, 2, 3]), &ids(&[3, 1, 2])).expect("ids are unique");
        assert_eq!(
            edits,
            RowEdits {
                deletes: Vec::new(),
                inserts: Vec::new(),
                moves: vec![(2, 0)],
            }
        );
    }

    #[test]
    fn single_section_row_diff_rejects_duplicates() {
        assert!(single_section_row_diff(&ids(&[1, 1]), &ids(&[1])).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn sequential_moves_replay_a_reorder_around_removals_and_insertions() {
        let old = ids(&[1, 2, 3, 4]);
        let new = ids(&[5, 4, 1, 3]);
        let removed: HashSet<ItemId> = ids(&[2]).into_iter().collect();
        let inserted: HashSet<ItemId> = ids(&[5]).into_iter().collect();
        let moves = sequential_moves(&old, &new, &removed, &inserted);
        // After removing 2 the rows are [1, 3, 4]; moving 4 to the front
        // gives [4, 1, 3], and inserting 5 at 0 gives the new order.
        assert_eq!(moves, vec![(2, 0)]);
    }

    #[test]
    fn min_row_height_prefers_the_configured_floor() {
        assert_eq!(
            min_row_height(Some(40.0), 24.0).to_bits(),
            40.0_f64.to_bits()
        );
        assert_eq!(min_row_height(None, 24.0).to_bits(), 24.0_f64.to_bits());
    }

    #[test]
    fn row_height_floors_at_the_minimum() {
        let insets = KitInsets {
            top: 4.0,
            bottom: 4.0,
            left: 10.0,
            right: 10.0,
        };
        assert_eq!(row_height(10.0, insets, 24.0).to_bits(), 24.0_f64.to_bits());
        assert_eq!(row_height(40.0, insets, 24.0).to_bits(), 48.0_f64.to_bits());
    }
}
