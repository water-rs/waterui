//! Collection of view-related utilities for managing and transforming UI components.
//!
//! This module provides types and traits for working with collections of views in a type-safe
//! and efficient manner. It includes utilities for type erasure, transformation, and identity
//! tracking of view collections.

use crate::id::Id as RawId;
use crate::id::Mapping;
use crate::{AnyView, View};
use alloc::fmt::Debug;
use alloc::{boxed::Box, rc::Rc, vec::Vec};
use core::any::type_name;
use core::fmt;
use core::marker::PhantomData;
use core::ops::{Bound, Range, RangeBounds};
use nami::collection::{Collection, CollectionChange, clamp_range, resolve_range};
use nami::watcher::{BoxWatcherGuard, Context, WatcherGuard};
use nami::{Computed, Signal};

/// An immutable snapshot of a view collection's state at one instant.
///
/// A snapshot owns — or shares ownership of — the row data captured when it
/// was taken, so it can be retained across later mutations of the source
/// collection. Indices passed to [`ViewSnapshot::get_id`] and
/// [`ViewSnapshot::get_view`] are collection-wide: the snapshot answers only
/// inside [`ViewSnapshot::range`] and returns `None` outside it.
///
/// Snapshots never produce views eagerly; `get_view` materializes only the
/// requested row.
pub trait ViewSnapshot {
    /// The type of unique identifier for items in the snapshot, matching the
    /// `Id` the source `Views` collection reports.
    type Id: 'static + Ord + Clone;
    /// The view type the snapshot materializes for each element.
    type View: View;
    /// The captured index window, in collection-wide index space.
    fn range(&self) -> Range<usize>;
    /// Returns the number of captured positions. Defaults to the range length.
    fn len(&self) -> usize {
        let range = self.range();
        range.end - range.start
    }
    /// Returns `true` if the snapshot captured no positions.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Returns the unique identifier for the item at the collection-wide
    /// `index`, or `None` if `index` is outside `range()`.
    fn get_id(&self, index: usize) -> Option<Self::Id>;
    /// Returns the view at the collection-wide `index`, or `None` if `index`
    /// is outside `range()`.
    fn get_view(&self, index: usize) -> Option<Self::View>;
}

/// A trait for collections that can provide unique identifiers for their elements.
///
/// `Views` extends the `Collection` trait by adding identity tracking capabilities.
/// This allows for efficient diffing and reconciliation of UI elements during updates.
///
/// Element access goes through snapshots: [`Views::snapshot`] captures the
/// complete current state, and [`Views::watch`] emissions hand the watcher an
/// owning snapshot of the collection as it stands when the notification is
/// delivered. A snapshot's rows and ids always come from one state of the
/// source, so a retained snapshot never mixes the data of two mutations.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a WaterUI view collection",
    label = "expected a collection of views",
    note = "`Views` is implemented by `ForEach`, `Constant`, `Vec` and arrays of views, and `AnyViews`. To turn a data collection into views use `ForEach::new(data, f)` — exposed as `Lazy::for_each` / `List::for_each` — over any `nami` collection: a `List` of `Identifiable` items, or an array or static slice, whose rows are identified by index."
)]
pub trait Views {
    /// The type of unique identifier for items in the collection; `Ord` is
    /// all identity alignment needs.
    type Id: 'static + Ord + Clone;
    /// The type of guard returned when registering a watcher.
    type Guard: WatcherGuard;
    /// The view type that this collection produces for each element.
    type View: View;
    /// The immutable snapshot type this collection captures.
    type Snapshot: ViewSnapshot<Id = Self::Id, View = Self::View> + Clone + 'static;

    /// Returns the number of items in the collection as a reactive value.
    fn len(&self) -> Computed<usize>;

    /// Returns `true` if the collection contains no elements.
    fn is_empty(&self) -> bool {
        self.len().snapshot() == 0
    }

    /// Captures the complete `0..len` state as an immutable snapshot.
    ///
    /// Taking a snapshot clones the row data only; it never invokes a view
    /// generator or `View::body`.
    fn snapshot(&self) -> Self::Snapshot;

    /// Registers a watcher for changes in the specified range of the collection.
    ///
    /// Each emission hands the watcher an owning snapshot covering the
    /// requested range of the collection, plus the [`CollectionChange`] —
    /// insertions, removals, moves and in-place updates, aligned by id — that
    /// the notification reported, both in collection-wide index space. The
    /// watcher may retain the snapshot beyond the callback.
    ///
    /// Registering delivers nothing: the watcher is called only when the
    /// collection changes. Read the current contents with
    /// [`Views::snapshot`] when mounting.
    ///
    /// Returns a guard that will unregister the watcher when dropped.
    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl Fn(Context<Self::Snapshot>, &CollectionChange) + 'static,
    ) -> Self::Guard;
}

/// A type-erased container for `Views` collections.
///
/// `AnyViews` provides a uniform interface to different views collections
/// by wrapping them in a type-erased container. This enables working with
/// heterogeneous view collections through a common interface.
pub struct AnyViews<V>(Box<dyn AnyViewsImpl<View = V>>);

impl<V> Debug for AnyViews<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

/// A reference-counted, type-erased container for `Views` collections.
/// `SharedAnyViews` allows multiple owners to share access
/// to the same views collection through reference counting.
pub struct SharedAnyViews<V>(Rc<dyn AnyViewsImpl<View = V>>);

impl<V> Clone for SharedAnyViews<V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<V> SharedAnyViews<V> {
    /// Creates a new type-erased shared view collection from any type implementing the `Views` trait.
    ///
    /// This function wraps the provided collection in a type-erased container using reference counting, allowing
    /// different view collection implementations to be used through a common interface with shared ownership.
    ///
    /// # Parameters
    /// * `contents` - Any collection implementing the `Views` trait with the appropriate item type
    ///
    /// # Returns
    /// A new `SharedAnyViews` instance containing the provided collection
    pub fn new(contents: impl Views<View = V> + 'static) -> Self {
        Self(Rc::new(IntoAnyViews::new(contents)))
    }

    /// Creates a new `SharedAnyViews` together with the id mapping that
    /// erases the collection's `C::Id` values into the ids `get_id` returns.
    ///
    /// The returned [`Mapping`] is the same generator the erased collection
    /// consults, so a selection keyed by `C::Id` can be mapped to and from
    /// the erased ids in both directions — the way `Mapping::binding` maps a
    /// `Picker` selection. Crate-internal: public only for `waterui` itself.
    #[doc(hidden)]
    #[must_use]
    pub fn new_with_ids<C>(contents: C) -> (Self, Mapping<C::Id>)
    where
        C: Views<View = V> + 'static,
    {
        let (contents, ids) = IntoAnyViews::new_with_ids(contents);
        (Self(Rc::new(contents)), ids)
    }
}

impl<V> From<AnyViews<V>> for SharedAnyViews<V> {
    fn from(value: AnyViews<V>) -> Self {
        Self(Rc::from(value.0))
    }
}

impl<V> Debug for SharedAnyViews<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<V: View> Views for SharedAnyViews<V> {
    type Id = RawId;
    type Guard = BoxWatcherGuard;
    type View = V;
    type Snapshot = AnyViewsSnapshot<V>;

    fn len(&self) -> Computed<usize> {
        self.0.len()
    }

    fn snapshot(&self) -> Self::Snapshot {
        self.0.snapshot()
    }

    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl Fn(Context<Self::Snapshot>, &CollectionChange) + 'static,
    ) -> Self::Guard {
        self.0.watch(
            (range.start_bound().cloned(), range.end_bound().cloned()),
            Box::new(watcher),
        )
    }
}

/// An immutable, type-erased view collection snapshot.
///
/// `AnyViewsSnapshot` is the `Snapshot` of both [`AnyViews`] and
/// [`SharedAnyViews`]. It shares the erased mapping the source collection
/// was built with, so ids stay stable across snapshots and callbacks. The
/// inner `Rc` makes cloning O(1).
pub struct AnyViewsSnapshot<V>(Rc<dyn AnyViewSnapshotImpl<View = V>>);

impl<V> Clone for AnyViewsSnapshot<V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<V> Debug for AnyViewsSnapshot<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<V: View> ViewSnapshot for AnyViewsSnapshot<V> {
    type Id = RawId;
    type View = V;

    fn range(&self) -> Range<usize> {
        self.0.range()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.0.get_id(index)
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.0.get_view(index)
    }
}

/// The boxed watcher callback an erased collection registers — it receives
/// an owning erased snapshot plus the notification's change report.
type BoxAnyViewsWatcher<V> = Box<dyn Fn(Context<AnyViewsSnapshot<V>>, &CollectionChange) + 'static>;

trait AnyViewsImpl {
    type View: View;

    fn len(&self) -> Computed<usize>;
    fn snapshot(&self) -> AnyViewsSnapshot<Self::View>;
    fn watch(
        &self,
        range: (Bound<usize>, Bound<usize>),
        watcher: BoxAnyViewsWatcher<Self::View>,
    ) -> BoxWatcherGuard;
}

trait AnyViewSnapshotImpl {
    type View: View;

    fn range(&self) -> Range<usize>;
    fn get_id(&self, index: usize) -> Option<RawId>;
    fn get_view(&self, index: usize) -> Option<Self::View>;
}

struct IntoAnyViews<V>
where
    V: Views,
{
    contents: V,
    id: Mapping<V::Id>,
}

/// The erased snapshot counterpart of [`IntoAnyViews`]: an immutable source
/// snapshot plus a clone of the same [`Mapping`] handle, so ids this snapshot
/// reports are the ids every other snapshot and callback reports.
struct IntoAnyViewSnapshot<S>
where
    S: ViewSnapshot,
{
    source: S,
    id: Mapping<S::Id>,
}

impl<V> IntoAnyViews<V>
where
    V: Views + 'static,
{
    pub fn new(contents: V) -> Self {
        Self::new_with_ids(contents).0
    }

    /// Builds the erasure together with the [`Mapping`] it feeds: every id
    /// `get_id` returns is registered through it, so the same handle maps a
    /// `V::Id`-keyed binding to and from the erased ids in both directions.
    pub fn new_with_ids(contents: V) -> (Self, Mapping<V::Id>) {
        let id = Mapping::new();
        (
            Self {
                id: id.clone(),
                contents,
            },
            id,
        )
    }
}

impl<S> AnyViewSnapshotImpl for IntoAnyViewSnapshot<S>
where
    S: ViewSnapshot + 'static,
{
    type View = S::View;

    fn range(&self) -> Range<usize> {
        self.source.range()
    }

    fn get_id(&self, index: usize) -> Option<RawId> {
        self.source.get_id(index).map(|id| self.id.to_id(id))
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.source.get_view(index)
    }
}

impl<V> AnyViewsImpl for IntoAnyViews<V>
where
    V: Views + 'static,
{
    type View = V::View;

    fn len(&self) -> Computed<usize> {
        self.contents.len()
    }

    fn snapshot(&self) -> AnyViewsSnapshot<V::View> {
        AnyViewsSnapshot(Rc::new(IntoAnyViewSnapshot {
            source: self.contents.snapshot(),
            id: self.id.clone(),
        }))
    }

    fn watch(
        &self,
        range: (Bound<usize>, Bound<usize>),
        watcher: BoxAnyViewsWatcher<V::View>,
    ) -> BoxWatcherGuard {
        let id = self.id.clone();
        Box::new(self.contents.watch(range, move |ctx, change| {
            let snapshot = ctx.map(|source| {
                AnyViewsSnapshot(Rc::new(IntoAnyViewSnapshot {
                    source,
                    id: id.clone(),
                }))
            });
            watcher(snapshot, change);
        }))
    }
}

impl<V> AnyViews<V>
where
    V: View,
{
    /// Creates a new type-erased view collection from any type implementing the `Views` trait.
    ///
    /// This function wraps the provided collection in a type-erased container, allowing
    /// different view collection implementations to be used through a common interface.
    ///
    /// # Parameters
    /// * `contents` - Any collection implementing the `Views` trait with the appropriate item type
    ///
    /// # Returns
    /// A new `AnyViews` instance containing the provided collection
    pub fn new<C>(contents: C) -> Self
    where
        C: Views<View = V> + 'static,
    {
        Self::new_with_ids(contents).0
    }

    /// Creates a new `AnyViews` together with the id mapping that erases the
    /// collection's `C::Id` values into the ids `get_id` returns.
    ///
    /// The returned [`Mapping`] is the same generator the erased collection
    /// consults, so a selection keyed by `C::Id` can be mapped to and from
    /// the erased ids in both directions — the way `Mapping::binding` maps a
    /// `Picker` selection. Crate-internal: public only for `waterui` itself.
    #[doc(hidden)]
    #[must_use]
    pub fn new_with_ids<C>(contents: C) -> (Self, Mapping<C::Id>)
    where
        C: Views<View = V> + 'static,
    {
        let (contents, ids) = IntoAnyViews::new_with_ids(contents);
        (Self(Box::new(contents)), ids)
    }
}

impl<V> Views for AnyViews<V>
where
    V: View,
{
    type Id = RawId;
    type Guard = BoxWatcherGuard;
    type View = V;
    type Snapshot = AnyViewsSnapshot<V>;

    fn len(&self) -> Computed<usize> {
        self.0.len()
    }

    fn snapshot(&self) -> Self::Snapshot {
        self.0.snapshot()
    }

    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl Fn(Context<Self::Snapshot>, &CollectionChange) + 'static,
    ) -> Self::Guard {
        self.0.watch(
            (range.start_bound().cloned(), range.end_bound().cloned()),
            Box::new(watcher),
        )
    }
}

/// A utility for transforming elements of a collection with a mapping function.
///
/// `ForEach` applies a transformation function to each element of a source collection,
/// producing a new collection with the transformed elements. This is useful for
/// transforming data models into view representations.
///
/// Rows take their identity from the source collection's [`Collection::Id`]:
/// a `List` of [`Identifiable`](crate::id::Identifiable) items identifies a
/// row by its item's id, and an array or static slice such as `[&str; N]`
/// identifies a row by its index.
#[derive(Debug)]
pub struct ForEach<C, F, V>
where
    C: Collection,
    F: Fn(C::Item) -> V,
    V: View,
{
    data: C,
    generator: Rc<F>,
}

impl<C, F, V> Clone for ForEach<C, F, V>
where
    C: Collection + Clone,
    F: Fn(C::Item) -> V,
    V: View,
{
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            generator: self.generator.clone(),
        }
    }
}

impl<C, F, V> ForEach<C, F, V>
where
    C: Collection,
    F: Fn(C::Item) -> V,
    V: View,
{
    /// Creates a new `ForEach` transformation with the provided data collection and generator function.
    ///
    /// # Parameters
    /// * `data` - The source collection containing elements to be transformed
    /// * `generator` - A function that transforms elements from the source collection
    ///
    /// # Returns
    /// A new `ForEach` instance that will apply the transformation when accessed
    pub fn new(data: C, generator: F) -> Self {
        Self {
            data,
            generator: Rc::new(generator),
        }
    }

    /// Consumes the `ForEach` and returns the original data collection and
    /// the shared generator function.
    ///
    /// # Returns
    /// A tuple containing the original data collection and the shared generator
    pub fn into_inner(self) -> (C, Rc<F>) {
        (self.data, self.generator)
    }
}

/// The length of a collection as a signal: notifies only when a change
/// inserts or removes items.
#[derive(Clone)]
struct CollectionLenSignal<C>(C);

impl<C> Signal for CollectionLenSignal<C>
where
    C: Collection + Clone,
{
    type Output = usize;
    type Guard = C::Guard;

    fn snapshot(&self) -> Self::Output {
        self.0.count()
    }

    fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {
        self.0.watch(.., move |ctx, change| {
            if change.insertions().next().is_none() && change.removals().next().is_none() {
                return;
            }
            watcher(ctx.map(<[C::Item]>::len));
        })
    }
}

/// A `ForEach` is a collection of its source's items, with the source's ids.
impl<C, F, V> Collection for ForEach<C, F, V>
where
    C: Collection,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
    type Item = C::Item;
    type Id = C::Id;
    type Guard = C::Guard;

    fn get_item(&self, index: usize) -> Option<Self::Item> {
        self.data.get_item(index)
    }

    fn count(&self) -> usize {
        self.data.count()
    }

    fn with_range<R>(
        &self,
        range: impl RangeBounds<usize>,
        f: impl FnOnce(&[Self::Item]) -> R,
    ) -> R {
        self.data.with_range(range, f)
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.data.get_id(index)
    }

    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl Fn(Context<&[Self::Item]>, &CollectionChange) + 'static,
    ) -> Self::Guard {
        self.data.watch(range, watcher)
    }
}

/// The immutable snapshot a [`ForEach`] captures: cloned row data and the
/// rows' ids, read from one state of the source, plus a shared handle to the
/// generator.
///
/// Cloning the snapshot is O(1) — the row data, the ids and the generator are
/// all reference-counted. `get_view` invokes the shared generator only for
/// the requested row.
pub struct ForEachSnapshot<C, F, V>
where
    C: Collection,
{
    items: Rc<[C::Item]>,
    ids: Rc<[C::Id]>,
    start: usize,
    generator: Rc<F>,
    _marker: PhantomData<fn() -> V>,
}

impl<C, F, V> ForEachSnapshot<C, F, V>
where
    C: Collection,
    C::Item: Clone,
{
    /// Captures `bounds`, resolved against the current length, from the
    /// live collection.
    fn capture(data: &C, bounds: (Bound<usize>, Bound<usize>), generator: Rc<F>) -> Self {
        let range = resolve_range(bounds.0, bounds.1, data.count());
        let items: Rc<[C::Item]> = data.with_range(range.clone(), |items| Rc::from(items));
        let ids: Rc<[C::Id]> = range
            .clone()
            .map(|index| {
                data.get_id(index)
                    .expect("a collection must report an id inside its reported length")
            })
            .collect();
        Self {
            items,
            ids,
            start: range.start,
            generator,
            _marker: PhantomData,
        }
    }
}

impl<C, F, V> Debug for ForEachSnapshot<C, F, V>
where
    C: Collection,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<C, F, V> Clone for ForEachSnapshot<C, F, V>
where
    C: Collection,
{
    fn clone(&self) -> Self {
        Self {
            items: self.items.clone(),
            ids: self.ids.clone(),
            start: self.start,
            generator: self.generator.clone(),
            _marker: PhantomData,
        }
    }
}

impl<C, F, V> ViewSnapshot for ForEachSnapshot<C, F, V>
where
    C: Collection,
    C::Item: Clone,
    C::Id: 'static,
    F: Fn(C::Item) -> V,
    V: View,
{
    type Id = C::Id;
    type View = V;

    fn range(&self) -> Range<usize> {
        self.start..self.start + self.items.len()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.ids.get(index.checked_sub(self.start)?).cloned()
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.items
            .get(index.checked_sub(self.start)?)
            .map(|item| (self.generator)(item.clone()))
    }
}

impl<C, F, V> Views for ForEach<C, F, V>
where
    C: Collection + Clone,
    C::Item: Clone,
    C::Id: 'static,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
    type Id = C::Id;
    type View = V;
    type Guard = C::Guard;
    type Snapshot = ForEachSnapshot<C, F, V>;

    fn len(&self) -> Computed<usize> {
        Computed::new(CollectionLenSignal(self.data.clone()))
    }

    fn snapshot(&self) -> Self::Snapshot {
        ForEachSnapshot::capture(
            &self.data,
            (Bound::Unbounded, Bound::Unbounded),
            self.generator.clone(),
        )
    }

    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl Fn(Context<Self::Snapshot>, &CollectionChange) + 'static,
    ) -> Self::Guard {
        let bounds = (range.start_bound().cloned(), range.end_bound().cloned());
        let data = self.data.clone();
        let generator = self.generator.clone();
        // Rows and ids are read together from the live collection, so a
        // snapshot always describes one state of it. The notification's own
        // slice cannot provide the ids: a `Collection` derives an id from a
        // position, not from an item. Without a reentrant mutation the live
        // state is the notified one; with one, the snapshot is the newer
        // state and the queued notification for it follows.
        self.data.watch(.., move |ctx, change| {
            let snapshot = ForEachSnapshot::capture(&data, bounds, generator.clone());
            watcher(ctx.map(|_| snapshot), change);
        })
    }
}

/// A statically sized collection that never changes, removes, or adds items.
///
/// `Constant` captures a view collection's complete state once, at
/// construction: the stored snapshot is immutable, so `len` and every later
/// snapshot report the same captured membership even if the source moves on.
/// Elements are identified by their index position. Capturing clones row
/// data only — it never runs a generator, and `get_view` still materializes
/// the row lazily.
pub struct Constant<C>
where
    C: Views,
{
    captured: C::Snapshot,
}

impl<C> Clone for Constant<C>
where
    C: Views,
{
    fn clone(&self) -> Self {
        Self {
            captured: self.captured.clone(),
        }
    }
}

impl<C> Debug for Constant<C>
where
    C: Views,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<C> Constant<C>
where
    C: Views,
{
    /// Creates a new `Constant` collection by capturing the complete current
    /// state of `value` as an immutable snapshot.
    ///
    /// # Parameters
    /// * `value` - The view collection whose state is frozen at this instant
    ///
    /// # Returns
    /// A new `Constant` instance holding the captured state
    pub fn new(value: &C) -> Self {
        Self {
            captured: value.snapshot(),
        }
    }
}

/// A `Constant` is a collection of its captured positions: each item is its
/// own index, and so is its id.
impl<C> Collection for Constant<C>
where
    C: Views + 'static,
{
    type Item = usize;
    type Id = usize;
    type Guard = ();

    fn get_item(&self, index: usize) -> Option<Self::Item> {
        (index < self.captured.len()).then_some(index)
    }

    fn count(&self) -> usize {
        self.captured.len()
    }

    fn with_range<R>(
        &self,
        range: impl RangeBounds<usize>,
        f: impl FnOnce(&[Self::Item]) -> R,
    ) -> R {
        let indices: Vec<usize> = clamp_range(&range, self.captured.len()).collect();
        f(&indices)
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.get_item(index)
    }

    fn watch(
        &self,
        _range: impl RangeBounds<usize>,
        _watcher: impl Fn(Context<&[Self::Item]>, &CollectionChange) + 'static,
    ) -> Self::Guard {
        // Captured once: nothing ever changes.
    }
}

/// The immutable snapshot a [`Constant`] hands out: a clone of the source
/// snapshot it captured at construction — never a live read — so `get_view`
/// stays as lazy as the source's.
pub struct ConstantSnapshot<S> {
    source: S,
}

impl<S: Clone> Clone for ConstantSnapshot<S> {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
        }
    }
}

impl<S> Debug for ConstantSnapshot<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<S> ViewSnapshot for ConstantSnapshot<S>
where
    S: ViewSnapshot,
{
    type Id = usize;
    type View = S::View;

    fn range(&self) -> Range<usize> {
        self.source.range()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.source.range().contains(&index).then_some(index)
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.source.get_view(index)
    }
}

impl<C> Views for Constant<C>
where
    C: Views,
{
    type Id = usize;
    type Guard = ();
    type View = C::View;
    type Snapshot = ConstantSnapshot<C::Snapshot>;

    fn len(&self) -> Computed<usize> {
        Computed::constant(self.captured.len())
    }

    fn snapshot(&self) -> Self::Snapshot {
        ConstantSnapshot {
            source: self.captured.clone(),
        }
    }

    fn watch(
        &self,
        _range: impl RangeBounds<usize>,
        _watcher: impl Fn(Context<Self::Snapshot>, &CollectionChange) + 'static,
    ) -> Self::Guard {
        // No-op for Constant
    }
}

/// The immutable snapshot a statically owned view collection captures: the
/// views cloned once into shared storage plus the collection-wide offset the
/// captured window starts at.
pub struct SliceSnapshot<V> {
    data: Rc<[V]>,
    start: usize,
}

impl<V> Debug for SliceSnapshot<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<V> Clone for SliceSnapshot<V> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            start: self.start,
        }
    }
}

impl<V: View + Clone> ViewSnapshot for SliceSnapshot<V> {
    type Id = usize;
    type View = V;

    fn range(&self) -> Range<usize> {
        self.start..self.start + self.data.len()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.range().contains(&index).then_some(index)
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.data
            .as_ref()
            .get(index.checked_sub(self.start)?)
            .cloned()
    }
}

impl<V: View + Clone> Views for Vec<V> {
    type Id = usize;
    type Guard = ();
    type View = V;
    type Snapshot = SliceSnapshot<V>;

    fn len(&self) -> Computed<usize> {
        Computed::constant(self.as_slice().len())
    }

    fn snapshot(&self) -> Self::Snapshot {
        SliceSnapshot {
            data: Rc::from(self.as_slice()),
            start: 0,
        }
    }

    fn watch(
        &self,
        _range: impl RangeBounds<usize>,
        _watcher: impl Fn(Context<Self::Snapshot>, &CollectionChange) + 'static,
    ) -> Self::Guard {
        // No-op for Vec
    }
}

impl<V: View + Clone, const N: usize> Views for [V; N] {
    type Id = usize;
    type Guard = ();
    type View = V;
    type Snapshot = SliceSnapshot<V>;

    fn len(&self) -> Computed<usize> {
        Computed::constant(self.as_ref().len())
    }

    fn snapshot(&self) -> Self::Snapshot {
        SliceSnapshot {
            data: Rc::from(self.as_slice()),
            start: 0,
        }
    }

    fn watch(
        &self,
        _range: impl RangeBounds<usize>,
        _watcher: impl Fn(Context<Self::Snapshot>, &CollectionChange) + 'static,
    ) -> Self::Guard {
        // No-op for arrays
    }
}

/// A view collection that transforms views from a source collection using a mapping function.
///
/// `Map` wraps an existing view collection and applies a transformation function to each
/// view when it is accessed, allowing for lazy transformation of views.
#[derive(Debug)]
pub struct Map<C, F> {
    source: C,
    f: Rc<F>,
}

impl<C: Clone, F> Clone for Map<C, F> {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            f: self.f.clone(),
        }
    }
}

impl<C, F, V> Map<C, F>
where
    C: Views,
    F: Fn(C::View) -> V,
    V: View,
{
    /// Creates a new `Map` that transforms views from the source collection.
    ///
    /// # Parameters
    /// * `source` - The source view collection to map over
    /// * `f` - The transformation function to apply to each view
    ///
    /// # Returns
    /// A new `Map` instance that will apply the transformation to views
    #[must_use]
    pub fn new(source: C, f: F) -> Self {
        Self {
            source,
            f: Rc::new(f),
        }
    }
}

/// The immutable snapshot a [`Map`] captures: the source snapshot plus a
/// shared handle to the mapping closure, so identity and generation both
/// refer to the same captured source state.
pub struct MapSnapshot<S, F, V> {
    source: S,
    f: Rc<F>,
    _marker: PhantomData<fn() -> V>,
}

impl<S, F, V> Debug for MapSnapshot<S, F, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<S: Clone, F, V> Clone for MapSnapshot<S, F, V> {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            f: self.f.clone(),
            _marker: PhantomData,
        }
    }
}

impl<S, F, V> ViewSnapshot for MapSnapshot<S, F, V>
where
    S: ViewSnapshot,
    F: Fn(S::View) -> V,
    V: View,
{
    type Id = S::Id;
    type View = V;

    fn range(&self) -> Range<usize> {
        self.source.range()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.source.get_id(index)
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.source.get_view(index).map(|view| (self.f)(view))
    }
}

impl<C, F, V> Views for Map<C, F>
where
    C: Views,
    F: 'static + Fn(C::View) -> V,
    V: View,
{
    type Id = C::Id;
    type Guard = C::Guard;
    type View = V;
    type Snapshot = MapSnapshot<C::Snapshot, F, V>;

    fn len(&self) -> Computed<usize> {
        self.source.len()
    }

    fn snapshot(&self) -> Self::Snapshot {
        MapSnapshot {
            source: self.source.snapshot(),
            f: self.f.clone(),
            _marker: PhantomData,
        }
    }

    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl Fn(Context<Self::Snapshot>, &CollectionChange) + 'static,
    ) -> Self::Guard {
        let f = self.f.clone();
        self.source.watch(range, move |ctx, change| {
            let snapshot = ctx.map(|source| MapSnapshot {
                source,
                f: f.clone(),
                _marker: PhantomData,
            });
            watcher(snapshot, change);
        })
    }
}

/// Extension trait providing additional utilities for types implementing `Views`.
///
/// This trait provides convenient methods for transforming and manipulating view collections,
/// such as mapping views to different types.
pub trait ViewsExt: Views {
    /// Transforms each view in the collection using the provided mapping function.
    ///
    /// # Parameters
    /// * `f` - A function that transforms each view from the source type to a new view type
    ///
    /// # Returns
    /// A new `Map` view collection that applies the transformation to each element
    fn map<F, V>(self, f: F) -> Map<Self, F>
    where
        Self: Sized,
        F: Fn(Self::View) -> V,
        V: View,
    {
        Map::new(self, f)
    }

    /// Erases the specific type of the view collection, returning a type-erased `AnyViews`.
    fn erase(self) -> AnyViews<AnyView>
    where
        Self: 'static + Sized,
    {
        AnyViews::new(self.map(AnyView::new))
    }
}

impl<T: Views> ViewsExt for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Identifiable;
    use alloc::{rc::Rc, vec, vec::Vec};
    use core::cell::{Cell, RefCell};
    use nami::{Binding, Signal, SignalExt, binding, collection::List};

    #[derive(Clone, Debug)]
    struct TestItem {
        id: i32,
    }

    impl Identifiable for TestItem {
        type Id = i32;

        fn id(&self) -> Self::Id {
            self.id
        }
    }

    fn items(ids: &[i32]) -> List<TestItem> {
        List::from(
            ids.iter()
                .map(|id| TestItem { id: *id })
                .collect::<Vec<_>>(),
        )
    }

    fn ids_in(snapshot: &(impl ViewSnapshot<Id = i32> + ?Sized), range: Range<usize>) -> Vec<i32> {
        range.filter_map(|index| snapshot.get_id(index)).collect()
    }

    /// Captured `(captured range, ids)` pairs a watch emission produced.
    type SnapshotLog = Rc<RefCell<Vec<(Range<usize>, Vec<i32>)>>>;

    /// A snapshot retained across scopes for the drop-lifetime test.
    type KeptSnapshot = Rc<RefCell<Option<Box<dyn ViewSnapshot<Id = i32, View = ()>>>>>;

    #[derive(Clone)]
    struct ReactiveLenViews {
        len_signal: nami::Binding<usize>,
    }

    impl Views for ReactiveLenViews {
        type Id = usize;
        type Guard = ();
        type View = ();
        type Snapshot = SliceSnapshot<()>;

        fn len(&self) -> Computed<usize> {
            self.len_signal.computed()
        }

        fn snapshot(&self) -> Self::Snapshot {
            SliceSnapshot {
                data: Rc::from(vec![(); self.len_signal.snapshot()].as_slice()),
                start: 0,
            }
        }

        fn watch(
            &self,
            _range: impl RangeBounds<usize>,
            _watcher: impl Fn(Context<Self::Snapshot>, &CollectionChange) + 'static,
        ) -> Self::Guard {
        }
    }

    #[test]
    fn len_tracks_reactive_len_signal() {
        let len_signal = binding(2usize);
        let views = ReactiveLenViews {
            len_signal: len_signal.clone(),
        };

        assert_eq!(views.len().snapshot(), 2);
        len_signal.set(5);
        assert_eq!(views.len().snapshot(), 5);
    }

    #[test]
    fn map_preserves_reactive_len() {
        let len_signal = binding(1usize);
        let views = ReactiveLenViews {
            len_signal: len_signal.clone(),
        };
        let mapped = views.map(|view| view);

        assert_eq!(mapped.len().snapshot(), 1);
        len_signal.set(4);
        assert_eq!(mapped.len().snapshot(), 4);
    }

    #[test]
    fn for_each_watch_uses_requested_range() {
        let mut list = items(&[1, 2, 3, 4]);
        let views = ForEach::new(list.clone(), |_item| ());

        let snapshots: SnapshotLog = Rc::new(RefCell::new(Vec::new()));
        let snapshots_ref = snapshots.clone();

        let _guard = Views::watch(&views, 1..3, move |ctx, _change| {
            let snapshot = ctx.into_value();
            let range = snapshot.range();
            snapshots_ref
                .borrow_mut()
                .push((range.clone(), ids_in(&snapshot, range)));
        });

        // Registering delivers nothing; the current contents are read with
        // `snapshot()`.
        assert!(snapshots.borrow().is_empty());

        list.insert(0, TestItem { id: 9 });
        assert_eq!(snapshots.borrow().as_slice(), &[(1..3, vec![1, 2])]);
    }

    #[test]
    fn for_each_over_an_array_identifies_rows_by_index() {
        let seen: Rc<RefCell<Vec<&str>>> = Rc::new(RefCell::new(Vec::new()));
        let views = ForEach::new(["first", "second", "third"], {
            let seen = seen.clone();
            move |label: &'static str| seen.borrow_mut().push(label)
        });

        let snapshot = views.snapshot();
        assert_eq!(snapshot.range(), 0..3);
        assert_eq!(
            (0..3)
                .filter_map(|index| snapshot.get_id(index))
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        snapshot.get_view(1).expect("row 1 exists");
        assert_eq!(seen.borrow().as_slice(), &["second"]);
    }

    #[test]
    fn watch_reports_a_reorder_as_moves() {
        let mut list = items(&[1, 2, 3]);
        let views = ForEach::new(list.clone(), |_item| ());
        let changes: Rc<RefCell<Vec<CollectionChange>>> = Rc::new(RefCell::new(Vec::new()));
        let _guard = Views::watch(&views, .., {
            let changes = changes.clone();
            move |_ctx, change| changes.borrow_mut().push(change.clone())
        });

        list.sort_by_key(|item| core::cmp::Reverse(item.id));

        let changes = changes.borrow();
        let [change] = changes.as_slice() else {
            panic!("one notification per sort, got {changes:?}");
        };
        assert!(change.insertions().next().is_none());
        assert!(change.removals().next().is_none());
        assert!(change.moves().next().is_some());
    }

    #[test]
    fn snapshot_retains_removed_and_reordered_rows() {
        let mut list = items(&[1, 2, 3]);
        let views = ForEach::new(list.clone(), |_item| ());

        let snapshot = views.snapshot();
        let _ = list.remove(0);
        list.insert(0, TestItem { id: 99 });

        // The retained snapshot still answers with the state it captured.
        assert_eq!(snapshot.range(), 0..3);
        assert_eq!(ids_in(&snapshot, 0..3), vec![1, 2, 3]);
    }

    #[test]
    fn capture_never_runs_the_generator() {
        let calls = Rc::new(Cell::new(0usize));
        let mut list = items(&[1, 2, 3]);
        let views = ForEach::new(list.clone(), {
            let calls = calls.clone();
            move |_item| calls.set(calls.get() + 1)
        });

        let snapshot = views.snapshot();
        let _guard = Views::watch(&views, .., |_ctx, _change| {});
        list.push(TestItem { id: 4 });

        // Neither `snapshot()` nor notification capture materialized a view.
        assert_eq!(calls.get(), 0);

        snapshot.get_view(1).expect("row 1 is inside the snapshot");
        assert_eq!(calls.get(), 1);

        // Reads outside the captured range produce no view at all.
        assert!(snapshot.get_view(3).is_none());
        assert_eq!(calls.get(), 1);
    }

    #[derive(Clone)]
    struct SignalItem {
        id: i32,
        value: Binding<i32>,
    }

    impl Identifiable for SignalItem {
        type Id = i32;
        fn id(&self) -> Self::Id {
            self.id
        }
    }

    #[test]
    fn snapshot_row_data_stays_live_through_its_signals() {
        let value = binding(10i32);
        let list = List::from(vec![SignalItem {
            id: 1,
            value: value.clone(),
        }]);
        let seen: Rc<RefCell<Vec<i32>>> = Rc::new(RefCell::new(Vec::new()));
        let views = ForEach::new(list, {
            let seen = seen.clone();
            move |item: SignalItem| seen.borrow_mut().push(item.value.snapshot())
        });

        let snapshot = views.snapshot();
        snapshot.get_view(0).expect("row 0 exists");
        value.set(42);
        snapshot.get_view(0).expect("row 0 still exists");

        // The captured row holds the same live `Binding`, not a frozen value.
        assert_eq!(seen.borrow().as_slice(), &[10, 42]);
    }

    #[test]
    fn reentrant_mutation_still_delivers_coherent_snapshots() {
        let mut list = items(&[1, 2]);
        let views = ForEach::new(list.clone(), |_item| ());

        // Watcher A mutates the source once, from inside the notification of
        // the insert below. The list queues that notification behind the one
        // in progress, so watcher B is still notified once per change, in
        // order, and every snapshot it receives holds the rows and ids of one
        // state of the list.
        let mutated = Rc::new(Cell::new(false));
        let _guard_a = Views::watch(&views, .., {
            let list = RefCell::new(list.clone());
            move |_ctx, _change| {
                if !mutated.replace(true) {
                    list.borrow_mut().push(TestItem { id: 1000 });
                }
            }
        });

        let seen: Rc<RefCell<Vec<Vec<i32>>>> = Rc::new(RefCell::new(Vec::new()));
        let _guard_b = Views::watch(&views, .., {
            let seen = seen.clone();
            move |ctx, _change| {
                let snapshot = ctx.into_value();
                let range = snapshot.range();
                seen.borrow_mut().push(ids_in(&snapshot, range));
            }
        });

        list.insert(0, TestItem { id: 9 });
        assert_eq!(
            seen.borrow().as_slice(),
            &[vec![9, 1, 2, 1000], vec![9, 1, 2, 1000]]
        );
    }

    #[test]
    fn erased_snapshots_share_one_id_mapping() {
        let mut list = items(&[1, 2, 3]);
        let views = AnyViews::new(ForEach::new(list.clone(), |_item| ()));

        let first = views.snapshot();
        let _ = list.remove(0);
        let second = views.snapshot();

        // Same source id, same erased id — the mapping is the collection's,
        // not the snapshot's.
        assert_eq!(first.get_id(1), second.get_id(0));
    }

    #[test]
    fn for_each_accepts_a_non_clone_generator() {
        struct NotClone;
        let views = ForEach::new(items(&[1]), {
            let marker = NotClone;
            move |_item| {
                let _ = &marker;
            }
        });
        let cloned = views.clone();
        let snapshot = cloned.snapshot();
        snapshot.get_view(0).expect("row 0 exists");
        assert_eq!(Views::len(&views).snapshot(), 1);
    }

    #[test]
    fn constant_freezes_membership_at_construction() {
        let mut list = items(&[1, 2, 3]);
        let seen: Rc<RefCell<Vec<i32>>> = Rc::new(RefCell::new(Vec::new()));
        let views = ForEach::new(list.clone(), {
            let seen = seen.clone();
            move |item: TestItem| {
                seen.borrow_mut().push(item.id);
            }
        });

        let constant = Constant::new(&views);

        // Construction captured row data only — the generator never ran.
        assert!(seen.borrow().is_empty());
        assert_eq!(Views::len(&constant).snapshot(), 3);

        let _ = list.remove(0);
        list.insert(0, TestItem { id: 99 });

        // The frozen membership ignores every later mutation of the source.
        let snapshot = constant.snapshot();
        assert_eq!(snapshot.range(), 0..3);
        assert_eq!(
            (0..3)
                .filter_map(|index| snapshot.get_id(index))
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );

        // Rows still materialize lazily — and they are the captured
        // `[1, 2, 3]`, not the mutated `[99, 2, 3]`.
        for index in 0..3 {
            snapshot
                .get_view(index)
                .expect("captured row must materialize");
        }
        assert_eq!(seen.borrow().as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn snapshot_survives_guard_and_source_drop() {
        let kept: KeptSnapshot = Rc::new(RefCell::new(None));
        {
            let mut list = items(&[1]);
            let views = ForEach::new(list.clone(), |_item| ());
            let guard = Views::watch(&views, .., {
                let kept = kept.clone();
                move |ctx, _change| {
                    *kept.borrow_mut() = Some(Box::new(ctx.into_value()));
                }
            });
            list.push(TestItem { id: 2 });
            drop(guard);
        }
        let borrowed = kept.borrow();
        let snapshot = borrowed.as_ref().expect("the push delivered a snapshot");
        let range = snapshot.range();
        assert_eq!(ids_in(&**snapshot, range), vec![1, 2]);
    }
}
