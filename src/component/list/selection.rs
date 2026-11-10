//! Selecting a list's rows by the app's own values.
//!
//! A list selects a row by the value the row is selected by — its *tag*: a
//! `ForEach` row by its item's id, a static row of
//! [`List::content`](super::List::content) by the value passed to
//! `.tag(…)`, and a row without one not at all. The app's selection binding
//! is typed in that value; [`ListBuilder::selection`](super::ListBuilder::selection)
//! erases it through a [`Mapping`] the way `Picker::new` erases its
//! selection, so a backend reads and writes erased tag ids and asks
//! [`ListRowTags`] which tag a row carries.

use alloc::collections::BTreeSet;
use alloc::rc::Rc;
use core::fmt;

use nami::Binding;
use nami::collection::Collection;
use waterui_core::id::{Id as RawId, Mapping};

use crate::views::{ForEach, Views};

use super::ListItem;

/// A tag the app selects a row by, erased through the list's value
/// [`Mapping`].
///
/// It is a different type from a row id: a backend cannot select a row by
/// comparing a selection value with the row's id, and asks
/// [`ListRowTags::tag`] for the tag a row carries instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ListTagId(RawId);

impl ListTagId {
    pub(crate) const fn new(id: RawId) -> Self {
        Self(id)
    }

    pub(crate) const fn into_raw(self) -> RawId {
        self.0
    }
}

/// Selection a list binds to its rows' tags: no selection, a single row, or
/// a set of rows — at most one mode per list.
///
/// In a [`ListConfig`](super::ListConfig) the `Id` is a tag erased through
/// the list's value [`Mapping`]: the same id [`ListRowTags::tag`] answers
/// for every row carrying that value.
#[derive(Clone, Default)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each selection mode variant by variant; a new selection mode must fail their build instead of falling into a wildcard arm"
)]
pub enum ListSelection<Id: 'static> {
    /// Rows are not selectable.
    #[default]
    None,
    /// A single selected tag — `None` inside the binding means nothing is
    /// selected.
    Single(Binding<Option<Id>>),
    /// The set of selected tags.
    Multiple(Binding<BTreeSet<Id>>),
}

impl<Id: 'static> fmt::Debug for ListSelection<Id> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(core::any::type_name::<Self>())
    }
}

impl ListSelection<ListTagId> {
    /// A single selection over the app's `selection`, erased through
    /// `values`.
    pub(crate) fn single<T: 'static + Ord + Clone>(
        selection: &Binding<Option<T>>,
        values: &Mapping<T>,
    ) -> Self {
        Self::Single(Binding::mapping(
            selection,
            {
                let to = values.clone();
                move |selected| selected.map(|value| ListTagId::new(to.to_id(value)))
            },
            {
                let from = values.clone();
                move |binding, erased: Option<ListTagId>| {
                    binding.set(erased.map(|tag| {
                        from.to_data(tag.into_raw())
                            .expect("a list selection tag was not taken from the list's rows")
                    }));
                }
            },
        ))
    }

    /// A multiple selection over the app's `selection`, erased through
    /// `values`: reads register each value, and backend writes resolve the
    /// tags they took from [`ListRowTags`] back to it.
    pub(crate) fn multiple<T: 'static + Ord + Clone>(
        selection: &Binding<BTreeSet<T>>,
        values: &Mapping<T>,
    ) -> Self {
        let to = values.clone();
        let from = values.clone();
        Self::Multiple(Binding::mapping(
            selection,
            move |selected| {
                selected
                    .into_iter()
                    .map(|value| ListTagId::new(to.to_id(value)))
                    .collect()
            },
            move |binding, erased: BTreeSet<ListTagId>| {
                binding.set(
                    erased
                        .into_iter()
                        .map(|tag| {
                            from.to_data(tag.into_raw())
                                .expect("a list selection tag was not taken from the list's rows")
                        })
                        .collect(),
                );
            },
        ))
    }
}

/// Rows a list selects by values of `T`.
///
/// A [`ForEach`] row is selected by its item's id. The rows of
/// [`List::content`](super::List::content) are selected by `T` when every
/// selectable row in the content is — `ForEach` items with ids of type `T`,
/// and static rows tagged with `.tag(value)` where `value: T` — and at least
/// one row is. A row that carries no value is not selectable, so content of
/// only such rows is selectable by no type.
#[diagnostic::on_unimplemented(
    message = "the rows of `{Self}` are not selected by `{T}`",
    label = "rows not selected by `{T}`",
    note = "a `ForEach` row is selected by its item's id, a static row of `List::content` by the value given to `.tag(…)`; every selectable row of one list is selected by the same type"
)]
pub trait SelectableRows<T>: Views<View = ListItem> {
    /// The value row `id` is selected by, or `None` when it is not
    /// selectable.
    fn row_tags(&self) -> impl Fn(&Self::Id) -> Option<T> + 'static;
}

impl<C, F> SelectableRows<C::Id> for ForEach<C, F, ListItem>
where
    C: Collection + Clone,
    C::Item: Clone,
    C::Id: 'static,
    F: 'static + Fn(C::Item) -> ListItem,
{
    fn row_tags(&self) -> impl Fn(&C::Id) -> Option<C::Id> + 'static {
        |id: &C::Id| Some(id.clone())
    }
}

/// A row's erased tag, looked up from its id.
type TagOf<Id> = Rc<dyn Fn(&Id) -> Option<ListTagId>>;

/// Every row's tag on the typed side of a list, already erased into the ids
/// its selection holds; [`RowTags::erased`] keys it by erased row ids once
/// the rows are erased.
pub struct RowTags<Id> {
    tag_of: TagOf<Id>,
}

impl<Id: 'static + Ord + Clone> RowTags<Id> {
    /// No row carries a tag: the list is not selectable.
    pub(crate) fn none() -> Self {
        Self {
            tag_of: Rc::new(|_| None),
        }
    }

    /// The tags `tag_of` gives the rows, erased through `values`.
    pub(crate) fn new<T: 'static + Ord + Clone>(
        tag_of: impl Fn(&Id) -> Option<T> + 'static,
        values: &Mapping<T>,
    ) -> Self {
        let values = values.clone();
        Self {
            tag_of: Rc::new(move |id| tag_of(id).map(|value| ListTagId::new(values.to_id(value)))),
        }
    }

    /// The tags over the row ids `ids` erased the rows into.
    pub(crate) fn erased(self, ids: &Mapping<Id>) -> ListRowTags {
        let ids = ids.clone();
        let tag_of = self.tag_of;
        ListRowTags {
            tag_of: Rc::new(move |row| {
                let id = ids.to_data(row).unwrap_or_else(|| {
                    panic!("list row id {row:?} was never reported by the list")
                });
                tag_of(&id)
            }),
        }
    }
}

impl RowTags<RawId> {
    /// Takes erased tags back to the typed side over the same raw row ids.
    pub(crate) fn from_erased(tags: ListRowTags) -> Self {
        Self {
            tag_of: Rc::new(move |row| tags.tag(*row)),
        }
    }
}

/// The tag each row of a list carries, answered from the row's id.
///
/// A tag is erased the way [`ListConfig::selection`](super::ListConfig::selection)
/// holds it: a row is selected exactly when its tag is in the selection,
/// selecting a row writes its tag, and a row with no tag is not selectable.
/// Rows carrying the same value share one tag, so they are selected together.
#[derive(Clone)]
pub struct ListRowTags {
    tag_of: Rc<dyn Fn(RawId) -> Option<ListTagId>>,
}

impl fmt::Debug for ListRowTags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListRowTags").finish_non_exhaustive()
    }
}

impl ListRowTags {
    /// The tag row `row` is selected by, or `None` when the row is not
    /// selectable.
    ///
    /// # Panics
    ///
    /// Panics if `row` is not an id the list's contents reported.
    #[must_use]
    pub fn tag(&self, row: RawId) -> Option<ListTagId> {
        (self.tag_of)(row)
    }

    /// Re-keys the row ids through `ids`, the mapping a re-erasure of the
    /// contents minted, so the tags answer the ids the backend sees.
    pub(crate) fn rekeyed(self, ids: &Mapping<RawId>) -> Self {
        RowTags::from_erased(self).erased(ids)
    }
}
