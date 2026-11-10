//! The rows a list displays and the sections they group into.
//!
//! A list is a sequence of sections, each owning its rows. The rows of every
//! section reach the backend as one flat, identity-keyed collection — the
//! shape virtualization, selection, editing and scrolling all index — and
//! [`ListSections`] answers which section a row belongs to from the row's id
//! alone, so a backend groups rows without materializing any of them.
//!
//! A [`List::content`](super::List::content) tree compiles to [`ContentRows`]:
//! a concatenation of *parts*, each either a run of static row builders or
//! one reactive collection (a `ForEach`). Every part belongs to one section,
//! and parts stay in declaration order, so the rows of a section are always
//! contiguous. A part forwards its own changes, re-based onto the rows before
//! it; a section's header therefore attaches to whichever row currently
//! opens it, and a section with no rows has no span and draws no chrome.
//!
//! Each part also keeps what its rows are selected by — a static row's
//! `.tag(…)` value, a collection item's id — erased until a selection names
//! the type; [`ContentTags`] makes that type agree with the content, and
//! requires a row selected by it, at compile time.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::{Any, type_name};
use core::cell::RefCell;
use core::fmt;
use core::marker::PhantomData;
use core::ops::{Range, RangeBounds};

use nami::collection::{CollectionChange, Edit};
use nami::watcher::{BoxWatcherGuard, Context, WatcherGuard};
use nami::{Computed, SignalExt};
use waterui_core::Environment;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::{Id as RawId, Mapping};

use crate::views::{AnyViewsSnapshot, SharedAnyViews, ViewSnapshot, Views};

use super::content::{ContentTags, HoldsTag};
use super::selection::SelectableRows;
use super::{ListItem, ListSection};

// ============================================================================
// Backend contract: sections over erased row ids
// ============================================================================

/// How a list's rows group into sections, answered from row ids.
///
/// Sections are indexed in display order. A section either carries
/// [`ListSection`] chrome — a header, a footer, the platform's grouped
/// presentation — or is a run of rows outside any `Section`, which carries
/// none. The rows of one section are contiguous in the list's contents, so
/// [`ListSections::spans`] finds every section's rows by binary search over
/// row ids, never by materializing a row.
#[derive(Clone)]
pub struct ListSections {
    section_of: Rc<dyn Fn(RawId) -> Option<usize>>,
    sections: Rc<[Option<ListSection>]>,
}

impl fmt::Debug for ListSections {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListSections")
            .field("sections", &self.sections.len())
            .finish_non_exhaustive()
    }
}

impl ListSections {
    /// The index of the section the row `id` belongs to.
    ///
    /// # Panics
    ///
    /// Panics if `id` is not an id the list's contents reported.
    #[must_use]
    pub fn section_of(&self, id: RawId) -> usize {
        (self.section_of)(id)
            .unwrap_or_else(|| panic!("list row id {id:?} was never reported by the list"))
    }

    /// The chrome of section `index`: `None` for rows outside any `Section`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is not a section index [`ListSections::section_of`]
    /// returned.
    #[must_use]
    pub fn section(&self, index: usize) -> Option<&ListSection> {
        self.sections[index].as_ref()
    }

    /// The sections that hold rows in `rows`, in order, with the rows each
    /// covers. The spans partition `rows.range()`; a section with no rows
    /// has no span, so it draws neither header nor footer.
    ///
    /// # Panics
    ///
    /// Panics if `rows` is not a snapshot of the list these sections
    /// describe: a row without an id, or with an id the list never reported.
    #[must_use]
    pub fn spans(&self, rows: &impl ViewSnapshot<Id = RawId>) -> Vec<ListSectionSpan> {
        let range = rows.range();
        let section_at = |index: usize| {
            self.section_of(
                rows.get_id(index)
                    .unwrap_or_else(|| panic!("list row {index} has no id in its snapshot")),
            )
        };
        let mut spans = Vec::new();
        let mut start = range.start;
        while start < range.end {
            let section = section_at(start);
            // Sections are monotonic along the rows: binary search for the
            // first row of a later section.
            let (mut low, mut high) = (start + 1, range.end);
            while low < high {
                let mid = low + (high - low) / 2;
                if section_at(mid) == section {
                    low = mid + 1;
                } else {
                    high = mid;
                }
            }
            spans.push(ListSectionSpan {
                rows: start..low,
                section: self.sections[section].clone(),
            });
            start = low;
        }
        spans
    }

    /// Re-keys the row ids through `ids`, the mapping a re-erasure of the
    /// contents minted, so the sections answer the ids the backend sees.
    pub(crate) fn rekeyed(self, ids: &Mapping<RawId>) -> Self {
        let ids = ids.clone();
        let section_of = self.section_of;
        Self {
            section_of: Rc::new(move |id| ids.to_data(id).and_then(|inner| section_of(inner))),
            sections: self.sections,
        }
    }

    /// Resolves every section's header and footer against `env`.
    pub(crate) fn resolved(self, env: &Environment) -> Self {
        Self {
            section_of: self.section_of,
            sections: self
                .sections
                .iter()
                .map(|section| section.clone().map(|section| section.resolved(env)))
                .collect(),
        }
    }
}

/// One section's rows in a snapshot of a list's contents.
#[derive(Debug, Clone)]
#[expect(
    clippy::exhaustive_structs,
    reason = "fixed-shape plain data a backend reads field by field"
)]
pub struct ListSectionSpan {
    /// The rows the section covers, never empty.
    pub rows: Range<usize>,
    /// The section's chrome: `None` for rows outside any `Section`.
    pub section: Option<ListSection>,
}

// ============================================================================
// Typed section layout, erased at `config()`
// ============================================================================

/// The section layout of a typed row collection, keyed by its own row ids;
/// [`RowSections::erased`] turns it into [`ListSections`] once the rows are
/// erased.
pub struct RowSections<Id> {
    section_of: Rc<dyn Fn(&Id) -> usize>,
    sections: Rc<[Option<ListSection>]>,
}

impl<Id> fmt::Debug for RowSections<Id> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RowSections")
            .field("sections", &self.sections.len())
            .finish_non_exhaustive()
    }
}

impl<Id: 'static + Ord + Clone> RowSections<Id> {
    /// One section without chrome holding every row — a list built from a
    /// plain view collection.
    pub(crate) fn plain() -> Self {
        Self {
            section_of: Rc::new(|_| 0),
            sections: Rc::from([None]),
        }
    }

    /// The layout over the ids `ids` erased the rows into.
    pub(crate) fn erased(self, ids: &Mapping<Id>) -> ListSections {
        let ids = ids.clone();
        let section_of = self.section_of;
        ListSections {
            section_of: Rc::new(move |id| ids.to_data(id).map(|id| section_of(&id))),
            sections: self.sections,
        }
    }
}

impl RowSections<RawId> {
    /// Takes erased sections back to a typed layout over the same raw ids.
    pub(crate) fn from_erased(sections: ListSections) -> Self {
        let lookup = sections.clone();
        Self {
            section_of: Rc::new(move |id| lookup.section_of(*id)),
            sections: sections.sections,
        }
    }
}

// ============================================================================
// Content rows: the concatenation a `ListContent` tree compiles to
// ============================================================================

/// A row's identity in a list built by [`List::content`](super::List::content):
/// the part of the content that produced it and its identity within that
/// part — a static row's position, or a collection item's own id.
///
/// Rows are told apart by it; they are selected by their tags (see
/// [`SelectableRows`]), never by this id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ListRowId {
    part: usize,
    row: RawId,
}

/// The rows of a [`List::content`](super::List::content) tree: every part of
/// every section, concatenated in declaration order.
///
/// `C` is the content the rows were collected from. It holds no data; it
/// carries the content's type to [`ListBuilder::selection`](super::ListBuilder::selection),
/// which accepts a selection type only when the content is
/// [`TaggedBy`](super::TaggedBy) it.
pub struct ContentRows<C> {
    parts: Rc<[SharedAnyViews<ListItem>]>,
    tags: Rc<[PartTags]>,
    _content: PhantomData<fn() -> C>,
}

/// What the rows of one part are selected by, erased until a selection
/// names the type.
enum PartTags {
    /// A run of static rows: each row's `.tag(…)` value, by position, and
    /// the mapping from the part's erased row ids to those positions.
    Static {
        positions: Mapping<usize>,
        tags: Rc<[Option<Rc<dyn Any>>]>,
    },
    /// A collection: the `Mapping<Id>` from the part's erased row ids to its
    /// items' ids.
    Items(Rc<dyn Any>),
}

/// [`PartTags`] once the selection type `T` is known.
enum TypedPartTags<T> {
    Static {
        positions: Mapping<usize>,
        tags: Rc<[Option<T>]>,
    },
    Items(Mapping<T>),
}

/// The panic for a part whose tags are not of the type its list is selected
/// by — reachable only through a `ContentTags` implementation that disagrees
/// with the rows its content collects.
fn mistagged<T>() -> ! {
    panic!(
        "a list selected by `{}` holds rows tagged by another type: a `ContentTags` implementation does not match the rows its content collects",
        type_name::<T>()
    )
}

impl PartTags {
    fn typed<T: 'static + Clone>(&self) -> TypedPartTags<T> {
        match self {
            Self::Static { positions, tags } => TypedPartTags::Static {
                positions: positions.clone(),
                tags: tags
                    .iter()
                    .map(|tag| {
                        tag.as_ref().map(|tag| {
                            tag.downcast_ref::<T>()
                                .cloned()
                                .unwrap_or_else(|| mistagged::<T>())
                        })
                    })
                    .collect(),
            },
            Self::Items(ids) => TypedPartTags::Items(
                ids.downcast_ref::<Mapping<T>>()
                    .cloned()
                    .unwrap_or_else(|| mistagged::<T>()),
            ),
        }
    }
}

impl<T: Ord + Clone> TypedPartTags<T> {
    /// The tag of the part's row `row`.
    fn tag(&self, row: RawId) -> Option<T> {
        fn unreported(row: RawId) -> ! {
            panic!("list row id {row:?} was never reported by its part")
        }
        match self {
            Self::Static { positions, tags } => {
                tags[positions.to_data(row).unwrap_or_else(|| unreported(row))].clone()
            }
            Self::Items(ids) => Some(ids.to_data(row).unwrap_or_else(|| unreported(row))),
        }
    }
}

/// The rows of `List::content` are selected by `T` when the content is
/// tagged by `T`: a static row by its `.tag(…)` value, a `ForEach` row by
/// its item's id, and an untagged static row not at all. Content with no row
/// selected by `T` is not selectable by it.
impl<C, T> SelectableRows<T> for ContentRows<C>
where
    C: ContentTags<T>,
    C::Presence: HoldsTag,
    T: 'static + Ord + Clone,
{
    fn row_tags(&self) -> impl Fn(&ListRowId) -> Option<T> + 'static {
        let parts: Rc<[TypedPartTags<T>]> = self.tags.iter().map(PartTags::typed).collect();
        move |id: &ListRowId| parts[id.part].tag(id.row)
    }
}

impl<C> fmt::Debug for ContentRows<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContentRows")
            .field("parts", &self.parts.len())
            .finish_non_exhaustive()
    }
}

/// An immutable snapshot of [`ContentRows`]: one snapshot per part, plus
/// where each part starts.
#[derive(Clone)]
pub struct ContentRowsSnapshot {
    parts: Rc<[AnyViewsSnapshot<ListItem>]>,
    starts: Rc<[usize]>,
    len: usize,
}

impl fmt::Debug for ContentRowsSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContentRowsSnapshot")
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

impl ContentRowsSnapshot {
    fn new(parts: Rc<[AnyViewsSnapshot<ListItem>]>) -> Self {
        let mut len = 0;
        let starts = parts
            .iter()
            .map(|part| {
                let start = len;
                len += part.len();
                start
            })
            .collect();
        Self { parts, starts, len }
    }

    /// The part holding row `index`, and the row's index within it.
    fn locate(&self, index: usize) -> Option<(usize, usize)> {
        if index >= self.len {
            return None;
        }
        // The last part starting at or before `index`: empty parts share
        // their successor's start, and the successor is the one holding it.
        let part = self.starts.partition_point(|start| *start <= index) - 1;
        Some((part, index - self.starts[part]))
    }
}

impl ViewSnapshot for ContentRowsSnapshot {
    type Id = ListRowId;
    type View = ListItem;

    fn range(&self) -> Range<usize> {
        0..self.len
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        let (part, local) = self.locate(index)?;
        let row = self.parts[part].get_id(local)?;
        Some(ListRowId { part, row })
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        let (part, local) = self.locate(index)?;
        self.parts[part].get_view(local)
    }
}

/// The guards of every part a [`ContentRows`] watcher registered on.
pub struct ContentRowsGuard(Vec<BoxWatcherGuard>);

impl fmt::Debug for ContentRowsGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ContentRowsGuard")
            .field(&self.0.len())
            .finish()
    }
}

impl WatcherGuard for ContentRowsGuard {}

/// `edit`, moved `offset` rows down.
fn shifted(edit: &Edit, offset: usize) -> Edit {
    let shift = |range: &Range<usize>| range.start + offset..range.end + offset;
    match edit {
        Edit::Remove(range) => Edit::Remove(shift(range)),
        Edit::Insert(range) => Edit::Insert(shift(range)),
        Edit::Update(range) => Edit::Update(shift(range)),
        Edit::Move { from, to } => Edit::Move {
            from: from + offset,
            to: to + offset,
        },
    }
}

impl<C> Views for ContentRows<C> {
    type Id = ListRowId;
    type Guard = ContentRowsGuard;
    type View = ListItem;
    type Snapshot = ContentRowsSnapshot;

    fn len(&self) -> Computed<usize> {
        self.parts
            .iter()
            .map(Views::len)
            .reduce(|total, len| total.zip(&len).map(|(total, len)| total + len).computed())
            .unwrap_or_else(|| Computed::constant(0))
    }

    fn snapshot(&self) -> Self::Snapshot {
        ContentRowsSnapshot::new(self.parts.iter().map(Views::snapshot).collect())
    }

    /// Watches every part. A notification carries the change of one part,
    /// re-based onto the rows before it, and a snapshot in which every other
    /// part is still the state this watcher last delivered — so the
    /// snapshot is exactly the previous one with this change applied, even
    /// when one mutation notifies several parts in turn.
    ///
    /// `_range` is not narrowed: a part's rows move with every part before
    /// it, so each part is watched whole.
    fn watch(
        &self,
        _range: impl RangeBounds<usize>,
        watcher: impl Fn(Context<Self::Snapshot>, &CollectionChange) + 'static,
    ) -> Self::Guard {
        let delivered: Rc<RefCell<Vec<AnyViewsSnapshot<ListItem>>>> = Rc::new(RefCell::new(
            self.parts.iter().map(Views::snapshot).collect(),
        ));
        let watcher = Rc::new(watcher);
        ContentRowsGuard(
            self.parts
                .iter()
                .enumerate()
                .map(|(part, rows)| {
                    let delivered = Rc::clone(&delivered);
                    let watcher = Rc::clone(&watcher);
                    let guard: BoxWatcherGuard = Box::new(rows.watch(.., move |ctx, change| {
                        let (snapshot, offset) = {
                            let mut delivered = delivered.borrow_mut();
                            let offset = delivered[..part].iter().map(ViewSnapshot::len).sum();
                            let (part_snapshot, metadata) = ctx.into_parts();
                            delivered[part] = part_snapshot;
                            (
                                Context::new(
                                    ContentRowsSnapshot::new(delivered.iter().cloned().collect()),
                                    metadata,
                                ),
                                offset,
                            )
                        };
                        let change = CollectionChange::from_edits(
                            change.edits().iter().map(|edit| shifted(edit, offset)),
                        );
                        watcher(snapshot, &change);
                    }));
                    guard
                })
                .collect(),
        )
    }
}

/// A run of static rows: builders identified by their position.
struct StaticRows {
    builders: Rc<[AnyViewBuilder<ListItem>]>,
}

/// The snapshot of [`StaticRows`] — the same builders, which never change.
#[derive(Clone)]
struct StaticRowsSnapshot {
    builders: Rc<[AnyViewBuilder<ListItem>]>,
}

impl ViewSnapshot for StaticRowsSnapshot {
    type Id = usize;
    type View = ListItem;

    fn range(&self) -> Range<usize> {
        0..self.builders.len()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        (index < self.builders.len()).then_some(index)
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.builders.get(index).map(AnyViewBuilder::build)
    }
}

impl Views for StaticRows {
    type Id = usize;
    type Guard = ();
    type View = ListItem;
    type Snapshot = StaticRowsSnapshot;

    fn len(&self) -> Computed<usize> {
        Computed::constant(self.builders.len())
    }

    fn snapshot(&self) -> Self::Snapshot {
        StaticRowsSnapshot {
            builders: Rc::clone(&self.builders),
        }
    }

    fn watch(
        &self,
        _range: impl RangeBounds<usize>,
        _watcher: impl Fn(Context<Self::Snapshot>, &CollectionChange) + 'static,
    ) -> Self::Guard {
    }
}

// ============================================================================
// Sink
// ============================================================================

/// One static row a [`ListItemSink`] collected.
struct StaticRow {
    builder: AnyViewBuilder<ListItem>,
    /// The row's `.tag(…)` value; `None` when the row is not selectable.
    tag: Option<Rc<dyn Any>>,
}

/// One part a [`ListItemSink`] is collecting.
enum SinkPart {
    /// Consecutive static rows of one section.
    Static(Vec<StaticRow>),
    /// One reactive collection and the `Mapping<Id>` its rows were erased
    /// through.
    Rows {
        rows: SharedAnyViews<ListItem>,
        ids: Rc<dyn Any>,
    },
}

/// Accumulator handed to [`ListContent::collect_items`](super::ListContent::collect_items).
///
/// It records the parts the content produces — runs of static row builders
/// and reactive row collections — and the section each belongs to. Rows
/// outside any [`Section`](super::Section) form sections without chrome.
pub struct ListItemSink {
    parts: Vec<SinkPart>,
    part_sections: Vec<usize>,
    sections: Vec<Option<ListSection>>,
    /// The section new rows join; `None` opens a section without chrome.
    open: Option<usize>,
}

impl Default for ListItemSink {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ListItemSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListItemSink")
            .field("parts", &self.parts.len())
            .field("sections", &self.sections.len())
            .finish_non_exhaustive()
    }
}

impl ListItemSink {
    /// Creates an empty sink.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            parts: Vec::new(),
            part_sections: Vec::new(),
            sections: Vec::new(),
            open: None,
        }
    }

    fn open_section(&mut self) -> usize {
        *self.open.get_or_insert_with(|| {
            self.sections.push(None);
            self.sections.len() - 1
        })
    }

    /// Pushes one static row built from a cloneable builder. The row carries
    /// no tag, so it is not selectable.
    pub fn push(&mut self, builder: AnyViewBuilder<ListItem>) {
        self.push_static(builder, None);
    }

    /// Pushes one static row selected by `tag`.
    pub(crate) fn push_tagged<T: 'static>(&mut self, builder: AnyViewBuilder<ListItem>, tag: T) {
        self.push_static(builder, Some(Rc::new(tag)));
    }

    fn push_static(&mut self, builder: AnyViewBuilder<ListItem>, tag: Option<Rc<dyn Any>>) {
        let section = self.open_section();
        if let (Some(SinkPart::Static(rows)), Some(last)) =
            (self.parts.last_mut(), self.part_sections.last())
            && *last == section
        {
            rows.push(StaticRow { builder, tag });
            return;
        }
        self.parts
            .push(SinkPart::Static(alloc::vec![StaticRow { builder, tag }]));
        self.part_sections.push(section);
    }

    /// Pushes a reactive collection of rows, each selected by its own id.
    /// Its rows are inserted, removed and moved in place as the collection
    /// changes.
    pub fn push_rows<V: Views<View = ListItem> + 'static>(&mut self, rows: V) {
        let section = self.open_section();
        let (rows, ids) = SharedAnyViews::new_with_ids(rows);
        self.parts.push(SinkPart::Rows {
            rows,
            ids: Rc::new(ids),
        });
        self.part_sections.push(section);
    }

    /// Opens a section with `chrome`; the rows pushed until
    /// [`ListItemSink::end_section`] belong to it.
    pub(crate) fn begin_section(&mut self, chrome: ListSection) {
        self.sections.push(Some(chrome));
        self.open = Some(self.sections.len() - 1);
    }

    /// Closes the open section: later rows start a section without chrome.
    pub(crate) const fn end_section(&mut self) {
        self.open = None;
    }

    /// The collected rows of the content `C` and the section each belongs
    /// to.
    pub(crate) fn finish<C>(self) -> (ContentRows<C>, RowSections<ListRowId>) {
        let (parts, tags): (Vec<_>, Vec<_>) = self
            .parts
            .into_iter()
            .map(|part| match part {
                SinkPart::Static(rows) => {
                    let (builders, tags): (Vec<_>, Vec<_>) =
                        rows.into_iter().map(|row| (row.builder, row.tag)).unzip();
                    let (rows, positions) = SharedAnyViews::new_with_ids(StaticRows {
                        builders: Rc::from(builders),
                    });
                    (
                        rows,
                        PartTags::Static {
                            positions,
                            tags: Rc::from(tags),
                        },
                    )
                }
                SinkPart::Rows { rows, ids } => (rows, PartTags::Items(ids)),
            })
            .unzip();
        let part_sections: Rc<[usize]> = Rc::from(self.part_sections);
        (
            ContentRows {
                parts: Rc::from(parts),
                tags: Rc::from(tags),
                _content: PhantomData,
            },
            RowSections {
                section_of: Rc::new(move |id: &ListRowId| part_sections[id.part]),
                sections: Rc::from(self.sections),
            },
        )
    }
}
