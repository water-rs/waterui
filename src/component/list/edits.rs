//! Identity-carrying edit operations (`on_delete` and `on_move`) for lists.
//!
//! Editing attaches to the data-bearing part of a list — a [`ForEach`] inside
//! [`List::content`](super::List::content) via [`ForEachExt`], a single-collection
//! [`List::for_each`](super::List::for_each), or an individual static [`Row`](super::Row)
//! via [`Row::on_delete`](super::Row::on_delete). Handlers receive the item's
//! own identity ([`ListDelete<Id>`] / [`ListMove<Id>`]) rather than a flat
//! display index, static rows are not deletable unless asked for, and moves stay
//! inside their owning part on every backend.
//!
//! On the backend side, [`ListEdits`] mirrors [`super::ListSections`] and
//! [`super::ListRowTags`]: it is keyed by the erased [`RawId`]s that
//! [`ListConfig::contents`](super::ListConfig::contents) reports, answers
//! deletability and movable part spans in `O(log n)` without materializing rows,
//! and routes edit actions back to the owning part's typed handler.

use alloc::rc::Rc;
use core::any::Any;
use core::fmt;
use core::ops::Range;

use nami::collection::Collection;
use waterui_core::extract::{ExtractionState, Extractor, State};
use waterui_core::handler::{Handler, shared_action};
use waterui_core::id::{Id as RawId, Mapping};
use waterui_core::{Environment, Error, View};

use crate::contract_violation;
use crate::views::{AnyViewsSnapshot, ForEach, Views};

use super::content::{ContentTags, ListContent, Tagged};
use super::rows::{ListItemSink, ListRowId};

/// A list reorder operation within one [`ForEach`] part.
///
/// Indices are local to that part, never the flattened list:
///
/// - [`Move::from`] is the moved item's index in the part before removal.
/// - [`Move::to`] is the **post-removal** destination index in the part — the
///   index the moved item occupies once removed from `from` and re-inserted.
///   For an unfiltered backing sequence, apply the move directly as:
///   `let item = items.remove(movement.from()); items.insert(movement.to(), item);`
/// - [`Move::id`] is the moved item's identity (the item at `from` before the
///   move).
/// - [`Move::target`] is the identity of the item at `to` before the move — the
///   slot `id` takes over. For a filtered view over a larger backing list,
///   locate both ids in the backing list and apply the same `remove` / `insert`
///   without re-running the filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Move<Id> {
    id: Id,
    target: Id,
    from: usize,
    to: usize,
}

impl<Id> Move<Id> {
    /// Creates a new move operation within a part.
    ///
    /// `from` is the pre-removal index of `id`; `to` is the post-removal
    /// destination index (the pre-move slot of `target`).
    #[must_use]
    pub const fn new(id: Id, target: Id, from: usize, to: usize) -> Self {
        Self {
            id,
            target,
            from,
            to,
        }
    }

    /// Returns the moved item's identity.
    #[must_use]
    pub const fn id(&self) -> &Id {
        &self.id
    }

    /// Returns the identity of the item occupying the destination slot before
    /// the move.
    #[must_use]
    pub const fn target(&self) -> &Id {
        &self.target
    }

    /// Returns the moved item's source index within its part before removal.
    #[must_use]
    pub const fn from(&self) -> usize {
        self.from
    }

    /// Returns the **post-removal** destination index within its part.
    ///
    /// Removing the item at [`Move::from`] and inserting it at [`Move::to`]
    /// places the item in its new position.
    #[must_use]
    pub const fn to(&self) -> usize {
        self.to
    }
}

/// Per-row delete payload injected into list delete handlers, carrying the
/// deleted item's identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListDelete<Id>(pub Id);

impl<Id: Clone + 'static> Extractor for ListDelete<Id> {
    fn extract(env: &Environment) -> Result<Self, Error> {
        <State<Self> as Extractor>::extract(env).map(|state| state.0)
    }

    fn extract_from_action(env: &Environment, state: &mut ExtractionState) -> Result<Self, Error> {
        <State<Self> as Extractor>::extract_from_action(env, state).map(|state| state.0)
    }
}

/// Per-row move payload injected into list move handlers, carrying the moved
/// item's identity and part-local destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListMove<Id>(pub Move<Id>);

impl<Id: Clone + 'static> Extractor for ListMove<Id> {
    fn extract(env: &Environment) -> Result<Self, Error> {
        <State<Self> as Extractor>::extract(env).map(|state| state.0)
    }

    fn extract_from_action(env: &Environment, state: &mut ExtractionState) -> Result<Self, Error> {
        <State<Self> as Extractor>::extract_from_action(env, state).map(|state| state.0)
    }
}

/// Typed delete callback for a single `ForEach` part.
pub type TypedOnDelete<Id> = Rc<dyn Fn(&Environment, Id)>;

/// Typed move callback for a single `ForEach` part.
pub type TypedOnMove<Id> = Rc<dyn Fn(&Environment, Move<Id>)>;

/// Delete callback attached to one static row.
pub type StaticOnDelete = Rc<dyn Fn(&Environment)>;

/// Wraps a handler as a typed delete callback injecting [`ListDelete<Id>`].
pub fn list_delete_action<Id, H, Args>(handler: H) -> TypedOnDelete<Id>
where
    Id: Clone + 'static,
    H: Handler<Args, ()>,
{
    let action = shared_action(handler);
    Rc::new(move |env, id| action.run(&env.extending(State(ListDelete(id)))))
}

/// Wraps a handler as a typed move callback injecting [`ListMove<Id>`].
pub fn list_move_action<Id, H, Args>(handler: H) -> TypedOnMove<Id>
where
    Id: Clone + 'static,
    H: Handler<Args, ()>,
{
    let action = shared_action(handler);
    Rc::new(move |env, movement| action.run(&env.extending(State(ListMove(movement)))))
}

/// Wraps a handler as a static-row delete callback.
pub fn static_delete_action<H, Args>(handler: H) -> StaticOnDelete
where
    H: Handler<Args, ()>,
{
    let action = shared_action(handler);
    Rc::new(move |env| action.run(env))
}

// ============================================================================
// EditableRows & ForEachExt / EditableForEach
// ============================================================================

/// Marks a uniform single-collection [`Views`] source that can accept
/// whole-list `.on_delete(...)` and `.on_move(...)` handlers carrying its own
/// item identity.
///
/// Mixed or sectioned lists built with [`List::content`](super::List::content)
/// do not implement this trait: attach `.on_delete(...)` and `.on_move(...)` to
/// each [`ForEach`] (or static [`Row`](super::Row)) inside the content instead.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a single uniform `ForEach` collection, so whole-list editing cannot carry one item identity",
    label = "cannot attach whole-list editing here",
    note = "attach `.on_delete(...)` and `.on_move(...)` to each `ForEach` (or static `Row`) inside `List::content(...)` instead"
)]
pub trait EditableRows: Views {}

impl<C, F, V> EditableRows for ForEach<C, F, V>
where
    C: Collection + Clone,
    C::Item: Clone,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
}

/// A [`ForEach`] inside [`List::content`](super::List::content) with
/// identity-carrying delete and/or move handlers attached to its part.
pub struct EditableForEach<C, F, V>
where
    C: Collection,
    F: Fn(C::Item) -> V,
    V: View,
{
    rows: ForEach<C, F, V>,
    on_delete: Option<TypedOnDelete<C::Id>>,
    on_move: Option<TypedOnMove<C::Id>>,
}

impl<C, F, V> fmt::Debug for EditableForEach<C, F, V>
where
    C: Collection,
    F: Fn(C::Item) -> V,
    V: View,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EditableForEach")
            .field("has_delete", &self.on_delete.is_some())
            .field("has_move", &self.on_move.is_some())
            .finish_non_exhaustive()
    }
}

impl<C, F, V> EditableForEach<C, F, V>
where
    C: Collection,
    F: Fn(C::Item) -> V,
    V: View,
{
    /// Handles deletion of items from this `ForEach` part, injecting
    /// [`ListDelete<C::Id>`] into `handler`.
    #[must_use]
    pub fn on_delete<H, Args>(mut self, handler: H) -> Self
    where
        C::Id: 'static + Clone,
        H: Handler<Args, ()>,
    {
        self.on_delete = Some(list_delete_action(handler));
        self
    }

    /// Handles reordering of items within this `ForEach` part, injecting
    /// [`ListMove<C::Id>`] into `handler`.
    #[must_use]
    pub fn on_move<H, Args>(mut self, handler: H) -> Self
    where
        C::Id: 'static + Clone,
        H: Handler<Args, ()>,
    {
        self.on_move = Some(list_move_action(handler));
        self
    }
}

/// Attaches identity-carrying list edit handlers directly to a [`ForEach`].
///
/// The resulting [`EditableForEach`] implements [`ListContent`] and
/// [`ContentTags`], so it composes inside [`List::content`](super::List::content)
/// and [`Section::content`](super::Section::content) alongside static rows and
/// other `ForEach` parts.
pub trait ForEachExt<C, F, V>: Sized
where
    C: Collection,
    F: Fn(C::Item) -> V,
    V: View,
{
    /// Handles deletion of items from this `ForEach` part, injecting
    /// [`ListDelete<C::Id>`] into `handler`.
    #[must_use]
    fn on_delete<H, Args>(self, handler: H) -> EditableForEach<C, F, V>
    where
        C::Id: 'static + Clone,
        H: Handler<Args, ()>;

    /// Handles reordering of items within this `ForEach` part, injecting
    /// [`ListMove<C::Id>`] into `handler`.
    #[must_use]
    fn on_move<H, Args>(self, handler: H) -> EditableForEach<C, F, V>
    where
        C::Id: 'static + Clone,
        H: Handler<Args, ()>;
}

impl<C, F, V> ForEachExt<C, F, V> for ForEach<C, F, V>
where
    C: Collection,
    F: Fn(C::Item) -> V,
    V: View,
{
    fn on_delete<H, Args>(self, handler: H) -> EditableForEach<C, F, V>
    where
        C::Id: 'static + Clone,
        H: Handler<Args, ()>,
    {
        EditableForEach {
            rows: self,
            on_delete: Some(list_delete_action(handler)),
            on_move: None,
        }
    }

    fn on_move<H, Args>(self, handler: H) -> EditableForEach<C, F, V>
    where
        C::Id: 'static + Clone,
        H: Handler<Args, ()>,
    {
        EditableForEach {
            rows: self,
            on_delete: None,
            on_move: Some(list_move_action(handler)),
        }
    }
}

impl<S, C, F, V> ListContent<S> for EditableForEach<C, F, V>
where
    C: Collection + Clone + 'static,
    C::Item: Clone,
    C::Id: 'static + Ord + Clone,
    F: Fn(C::Item) -> V + 'static,
    V: View,
{
    type Row = V;

    fn collect_items<R, W>(self, sink: &mut ListItemSink<R>, wrap: W)
    where
        R: View,
        W: Fn(Self::Row) -> R + Clone + 'static,
    {
        sink.push_editable_rows(self.rows, wrap, self.on_delete, self.on_move);
    }
}

impl<C, F, V> ContentTags<C::Id> for EditableForEach<C, F, V>
where
    C: Collection,
    F: Fn(C::Item) -> V,
    V: View,
{
    type Presence = Tagged;
}

// ============================================================================
// Per-part edit routing (internal) and backend contract (`ListEdits`)
// ============================================================================

/// Erased per-row delete handler inside one part.
type PartDeleteFn = Rc<dyn Fn(&Environment, RawId)>;

/// Erased per-row move handler inside one part: `(env, from_row, to_row, from_in_part, to_in_part)`.
type PartMoveFn = Rc<dyn Fn(&Environment, RawId, RawId, usize, usize)>;

/// Per-part editing state stored inside [`ContentRows`](super::ContentRows).
pub enum PartEdits {
    /// Static rows: individual rows may opt in to deletion, never moving.
    Static {
        positions: Mapping<usize>,
        on_deletes: Rc<[Option<StaticOnDelete>]>,
    },
    /// A `ForEach` part: optional part-wide delete and/or move handler, already
    /// wired to resolve part-local `RawId`s back to `C::Id`.
    Rows {
        on_delete: Option<PartDeleteFn>,
        on_move: Option<PartMoveFn>,
    },
}

impl PartEdits {
    /// Builds [`PartEdits::Rows`] for a `ForEach` erased through `ids`.
    pub(crate) fn for_rows<Id: 'static + Ord + Clone>(
        ids: &Mapping<Id>,
        on_delete: Option<TypedOnDelete<Id>>,
        on_move: Option<TypedOnMove<Id>>,
    ) -> Self {
        let on_delete = on_delete.map(|handler| {
            let ids = ids.clone();
            let delete_fn: PartDeleteFn = Rc::new(move |env, row| {
                let Some(id) = ids.to_data(row) else {
                    contract_violation!(
                        "list delete named row id {row:?} that its part never reported; ignoring"
                    );
                    return;
                };
                handler(env, id);
            });
            delete_fn
        });
        let on_move = on_move.map(|handler| {
            let ids = ids.clone();
            let move_fn: PartMoveFn = Rc::new(move |env, from_row, to_row, from, to| {
                let (Some(id), Some(target)) = (ids.to_data(from_row), ids.to_data(to_row)) else {
                    contract_violation!(
                        "list move named row ids ({from_row:?}, {to_row:?}) that its part never reported; ignoring"
                    );
                    return;
                };
                handler(env, Move::new(id, target, from, to));
            });
            move_fn
        });
        Self::Rows { on_delete, on_move }
    }

    fn has_delete(&self) -> bool {
        match self {
            Self::Static { on_deletes, .. } => on_deletes.iter().any(Option::is_some),
            Self::Rows { on_delete, .. } => on_delete.is_some(),
        }
    }

    const fn has_move(&self) -> bool {
        match self {
            Self::Static { .. } => false,
            Self::Rows { on_move, .. } => on_move.is_some(),
        }
    }

    fn can_delete(&self, row: RawId) -> bool {
        match self {
            Self::Static {
                positions,
                on_deletes,
            } => positions
                .to_data(row)
                .and_then(|pos| on_deletes.get(pos))
                .is_some_and(Option::is_some),
            Self::Rows { on_delete, .. } => on_delete.is_some(),
        }
    }

    fn delete(&self, env: &Environment, row: RawId) {
        match self {
            Self::Static {
                positions,
                on_deletes,
            } => {
                let Some(handler) = positions
                    .to_data(row)
                    .and_then(|pos| on_deletes.get(pos))
                    .and_then(Option::as_ref)
                else {
                    contract_violation!(
                        "list delete invoked on non-deletable static row {row:?}; ignoring"
                    );
                    return;
                };
                handler(env);
            }
            Self::Rows {
                on_delete: Some(handler),
                ..
            } => handler(env, row),
            Self::Rows {
                on_delete: None, ..
            } => {
                contract_violation!(
                    "list delete invoked on a part without an on_delete handler (row {row:?}); ignoring"
                );
            }
        }
    }

    fn move_row(
        &self,
        env: &Environment,
        from_row: RawId,
        to_row: RawId,
        from_in_part: usize,
        to_in_part: usize,
    ) {
        match self {
            Self::Rows {
                on_move: Some(handler),
                ..
            } => handler(env, from_row, to_row, from_in_part, to_in_part),
            Self::Static { .. } | Self::Rows { on_move: None, .. } => {
                contract_violation!(
                    "list move invoked on a non-movable part (from {from_row:?}, to {to_row:?}); ignoring"
                );
            }
        }
    }
}

/// Helper to box an `Any`-erased `Mapping<Id>` without duplicating code in
/// `ListItemSink`.
pub fn erase_ids<Id: 'static + Ord + Clone>(ids: Mapping<Id>) -> Rc<dyn Any> {
    Rc::new(ids)
}

/// Typed per-row delete handler taking a reference to `Id`.
type RowDeleteFn<Id> = Rc<dyn Fn(&Environment, &Id)>;

/// Typed per-row move handler taking `(env, from_id, to_id, from_in_part, to_in_part)`.
type RowMoveFn<Id> = Rc<dyn Fn(&Environment, &Id, &Id, usize, usize)>;

/// Typed edit routing on `List<V>` / `ListBuilder<V>`, erased to [`ListEdits`]
/// once the list's rows are erased at `config()`.
pub struct RowEdits<Id> {
    has_delete: bool,
    has_move: bool,
    part_of: Rc<dyn Fn(&Id) -> usize>,
    can_delete: Rc<dyn Fn(&Id) -> bool>,
    on_delete: RowDeleteFn<Id>,
    can_move: Rc<dyn Fn(&Id) -> bool>,
    on_move: RowMoveFn<Id>,
}

impl<Id> fmt::Debug for RowEdits<Id> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RowEdits")
            .field("has_delete", &self.has_delete)
            .field("has_move", &self.has_move)
            .finish_non_exhaustive()
    }
}

impl<Id: 'static + Ord + Clone> RowEdits<Id> {
    /// No row can be deleted or moved; every row belongs to part 0.
    pub(crate) fn none() -> Self {
        Self {
            has_delete: false,
            has_move: false,
            part_of: Rc::new(|_| 0),
            can_delete: Rc::new(|_| false),
            on_delete: Rc::new(|_, _| {
                contract_violation!("list delete invoked on a list without an on_delete handler");
            }),
            can_move: Rc::new(|_| false),
            on_move: Rc::new(|_, _, _, _, _| {
                contract_violation!("list move invoked on a list without an on_move handler");
            }),
        }
    }

    /// Single-part editing for `List::for_each`: every row belongs to part 0
    /// and carries `Id` directly.
    pub(crate) fn single_part(
        on_delete: Option<TypedOnDelete<Id>>,
        on_move: Option<TypedOnMove<Id>>,
    ) -> Self {
        let has_delete = on_delete.is_some();
        let has_move = on_move.is_some();
        Self {
            has_delete,
            has_move,
            part_of: Rc::new(|_| 0),
            can_delete: Rc::new(move |_| has_delete),
            on_delete: match on_delete {
                Some(handler) => Rc::new(move |env, id: &Id| handler(env, id.clone())),
                None => Rc::new(|_, _| {
                    contract_violation!(
                        "list delete invoked on a list without an on_delete handler"
                    );
                }),
            },
            can_move: Rc::new(move |_| has_move),
            on_move: match on_move {
                Some(handler) => Rc::new(move |env, from_id: &Id, to_id: &Id, from, to| {
                    handler(env, Move::new(from_id.clone(), to_id.clone(), from, to));
                }),
                None => Rc::new(|_, _, _, _, _| {
                    contract_violation!("list move invoked on a list without an on_move handler");
                }),
            },
        }
    }

    /// Erases these edits over the raw row ids `ids` minted when erasing the
    /// list's rows.
    pub(crate) fn erased(self, ids: &Mapping<Id>) -> ListEdits {
        let part_of = {
            let ids = ids.clone();
            let f = self.part_of;
            Rc::new(move |row: RawId| ids.to_data(row).map(|id| f(&id)))
        };
        let can_delete = {
            let ids = ids.clone();
            let f = self.can_delete;
            Rc::new(move |row: RawId| ids.to_data(row).is_some_and(|id| f(&id)))
        };
        let on_delete = {
            let ids = ids.clone();
            let f = self.on_delete;
            Rc::new(move |env: &Environment, row: RawId| {
                let Some(id) = ids.to_data(row) else {
                    contract_violation!(
                        "list delete named row id {row:?} that the list never reported; ignoring"
                    );
                    return;
                };
                f(env, &id);
            })
        };
        let can_move = {
            let ids = ids.clone();
            let f = self.can_move;
            Rc::new(move |row: RawId| ids.to_data(row).is_some_and(|id| f(&id)))
        };
        let on_move = {
            let ids = ids.clone();
            let f = self.on_move;
            Rc::new(
                move |env: &Environment,
                      from_row: RawId,
                      to_row: RawId,
                      from_in_part: usize,
                      to_in_part: usize| {
                    let (Some(from_id), Some(to_id)) = (ids.to_data(from_row), ids.to_data(to_row))
                    else {
                        contract_violation!(
                            "list move named row ids ({from_row:?}, {to_row:?}) that the list never reported; ignoring"
                        );
                        return;
                    };
                    f(env, &from_id, &to_id, from_in_part, to_in_part);
                },
            )
        };
        ListEdits {
            has_delete: self.has_delete,
            has_move: self.has_move,
            part_of,
            can_delete,
            on_delete,
            can_move,
            on_move,
        }
    }
}

impl RowEdits<ListRowId> {
    /// Per-part editing for `List::content`: each part owns its own delete and
    /// move handlers, and moves never cross part boundaries.
    pub(crate) fn from_parts(parts: &Rc<[PartEdits]>) -> Self {
        let has_delete = parts.iter().any(PartEdits::has_delete);
        let has_move = parts.iter().any(PartEdits::has_move);
        let can_delete = {
            let parts = Rc::clone(parts);
            Rc::new(move |id: &ListRowId| {
                parts
                    .get(id.part())
                    .is_some_and(|part| part.can_delete(id.row()))
            })
        };
        let on_delete = {
            let parts = Rc::clone(parts);
            Rc::new(move |env: &Environment, id: &ListRowId| {
                let Some(part) = parts.get(id.part()) else {
                    contract_violation!(
                        "list delete named part {}, but the list has {} parts; ignoring",
                        id.part(),
                        parts.len()
                    );
                    return;
                };
                part.delete(env, id.row());
            })
        };
        let can_move = {
            let parts = Rc::clone(parts);
            Rc::new(move |id: &ListRowId| parts.get(id.part()).is_some_and(PartEdits::has_move))
        };
        let on_move = {
            let parts = Rc::clone(parts);
            Rc::new(
                move |env: &Environment,
                      from_id: &ListRowId,
                      to_id: &ListRowId,
                      from_in_part: usize,
                      to_in_part: usize| {
                    if from_id.part() != to_id.part() {
                        contract_violation!(
                            "list move attempted to cross parts ({} -> {}); ignoring",
                            from_id.part(),
                            to_id.part()
                        );
                        return;
                    }
                    let Some(part) = parts.get(from_id.part()) else {
                        contract_violation!(
                            "list move named part {}, but the list has {} parts; ignoring",
                            from_id.part(),
                            parts.len()
                        );
                        return;
                    };
                    part.move_row(env, from_id.row(), to_id.row(), from_in_part, to_in_part);
                },
            )
        };
        Self {
            has_delete,
            has_move,
            part_of: Rc::new(|id: &ListRowId| id.part()),
            can_delete,
            on_delete,
            can_move,
            on_move,
        }
    }
}

impl RowEdits<RawId> {
    /// Takes erased edits back to the typed side over the same raw row ids.
    pub(crate) fn from_erased(edits: ListEdits) -> Self {
        let has_delete = edits.has_delete;
        let has_move = edits.has_move;
        let part_of = {
            let edits = edits.clone();
            Rc::new(move |row: &RawId| edits.part_of(*row))
        };
        let can_delete = {
            let edits = edits.clone();
            Rc::new(move |row: &RawId| edits.can_delete(*row))
        };
        let on_delete = {
            let edits = edits.clone();
            Rc::new(move |env: &Environment, row: &RawId| edits.delete(env, *row))
        };
        let can_move = {
            let edits = edits.clone();
            Rc::new(move |row: &RawId| edits.can_move(*row))
        };
        let on_move = Rc::new(
            move |env: &Environment,
                  from: &RawId,
                  to: &RawId,
                  from_in_part: usize,
                  to_in_part: usize| {
                edits.move_row(env, *from, *to, from_in_part, to_in_part);
            },
        );
        Self {
            has_delete,
            has_move,
            part_of,
            can_delete,
            on_delete,
            can_move,
            on_move,
        }
    }
}

/// Backend contract for per-row deletion and per-part reordering, keyed by the
/// erased [`RawId`]s that [`ListConfig::contents`](super::ListConfig::contents)
/// reports.
#[derive(Clone)]
pub struct ListEdits {
    has_delete: bool,
    has_move: bool,
    part_of: Rc<dyn Fn(RawId) -> Option<usize>>,
    can_delete: Rc<dyn Fn(RawId) -> bool>,
    on_delete: PartDeleteFn,
    can_move: Rc<dyn Fn(RawId) -> bool>,
    on_move: PartMoveFn,
}

impl fmt::Debug for ListEdits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListEdits")
            .field("has_delete", &self.has_delete)
            .field("has_move", &self.has_move)
            .finish_non_exhaustive()
    }
}

impl ListEdits {
    /// Whether any row or part in this list can be deleted.
    #[must_use]
    pub const fn has_delete(&self) -> bool {
        self.has_delete
    }

    /// Whether any part in this list can be reordered.
    #[must_use]
    pub const fn has_move(&self) -> bool {
        self.has_move
    }

    /// The monotonically non-decreasing part index for `row`.
    fn part_of(&self, row: RawId) -> usize {
        (self.part_of)(row).unwrap_or_else(|| {
            contract_violation!(
                "list row id {row:?} was never reported by the list, so it is treated as part 0"
            );
            0
        })
    }

    /// Whether `row` has an attached delete handler.
    #[must_use]
    pub fn can_delete(&self, row: RawId) -> bool {
        self.has_delete && (self.can_delete)(row)
    }

    /// Invokes the delete handler for `row`, delivering the item's typed
    /// identity to its owning part.
    pub fn delete(&self, env: &Environment, row: RawId) {
        (self.on_delete)(env, row);
    }

    /// Whether `row` belongs to a movable part.
    #[must_use]
    pub fn can_move(&self, row: RawId) -> bool {
        self.has_move && (self.can_move)(row)
    }

    /// Returns the owning part index when `row` is movable, or `None` when it
    /// cannot move. Two rows can be reordered relative to each other if and
    /// only if this returns the same `Some(part)`.
    #[must_use]
    pub fn movable_part(&self, row: RawId) -> Option<usize> {
        self.can_move(row).then(|| self.part_of(row))
    }

    /// Returns the contiguous flat index span `part_start..part_end` within
    /// `range` that `index` may move inside, or `None` if `index` is out of
    /// range or not movable.
    ///
    /// Runs in `O(log n)` using the monotonic part layout without materializing
    /// any row views.
    #[must_use]
    pub fn movable_span_in(
        &self,
        range: Range<usize>,
        id_at: impl Fn(usize) -> Option<RawId>,
        index: usize,
    ) -> Option<Range<usize>> {
        if !self.has_move || !range.contains(&index) {
            return None;
        }
        let row = id_at(index)?;
        let part = self.movable_part(row)?;
        let part_at = |i: usize| {
            let Some(id) = id_at(i) else {
                contract_violation!(
                    "list row {i} has no id in its snapshot, so it is treated as part 0"
                );
                return 0;
            };
            self.part_of(id)
        };
        let (mut low, mut high) = (range.start, index);
        while low < high {
            let mid = low + (high - low) / 2;
            if part_at(mid) < part {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        let start = low;

        let (mut low, mut high) = (index + 1, range.end);
        while low < high {
            let mid = low + (high - low) / 2;
            if part_at(mid) <= part {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        let end = low;
        Some(start..end)
    }

    /// Returns the contiguous flat index span within `rows` that `index` may
    /// move inside, or `None` if `index` is not movable.
    #[must_use]
    pub fn movable_span<V>(
        &self,
        rows: &AnyViewsSnapshot<V>,
        index: usize,
    ) -> Option<Range<usize>> {
        self.movable_span_in(rows.range(), |i| rows.get_id(i), index)
    }

    /// Returns the contiguous flat index span within `ids` that `index` may
    /// move inside, or `None` if `index` is not movable.
    #[must_use]
    pub fn movable_span_in_slice(&self, ids: &[RawId], index: usize) -> Option<Range<usize>> {
        self.movable_span_in(0..ids.len(), |i| ids.get(i).copied(), index)
    }

    /// Invokes the owning part's move handler, moving `from` to the pre-move
    /// slot of `to` at part-local indices `(from_in_part, to_in_part)` where
    /// `to_in_part` is the post-removal destination index.
    pub fn move_row(
        &self,
        env: &Environment,
        from: RawId,
        to: RawId,
        from_in_part: usize,
        to_in_part: usize,
    ) {
        if self.movable_part(from).is_none() || self.movable_part(from) != self.movable_part(to) {
            contract_violation!(
                "list move attempted between incompatible rows ({from:?} -> {to:?}); ignoring"
            );
            return;
        }
        (self.on_move)(env, from, to, from_in_part, to_in_part);
    }

    /// Re-keys every lookup through `ids`, the mapping a re-erasure of the
    /// contents minted, so backends query the ids `contents.get_id` returns.
    pub(crate) fn rekeyed(self, ids: &Mapping<RawId>) -> Self {
        RowEdits::from_erased(self).erased(ids)
    }
}
