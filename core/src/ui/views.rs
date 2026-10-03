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
use core::hash::Hash;
use core::marker::PhantomData;
use core::ops::{Bound, Range, RangeBounds};
use nami::collection::{Collection, CollectionChange};
use nami::watcher::{BoxWatcherGuard, Context, WatcherGuard};
use nami::{Computed, Signal};

use crate::id::{Identifiable, SelfId};

/// Resolves `bounds` against a captured length, clamping to valid indices.
///
/// The result is in collection-wide index space: bounds that extend past the
/// captured data are clamped to `len`, never expanded. Use this to apply a
/// watcher-requested range to the immutable data a single notification
/// carried — never to live data, which may already have moved on.
#[must_use]
pub fn resolve_range(bounds: (Bound<usize>, Bound<usize>), len: usize) -> Range<usize> {
    let mut start = match bounds.0 {
        Bound::Included(n) => n,
        Bound::Excluded(n) => n.saturating_add(1),
        Bound::Unbounded => 0,
    };
    let mut end = match bounds.1 {
        Bound::Included(n) => n.saturating_add(1),
        Bound::Excluded(n) => n,
        Bound::Unbounded => len,
    };
    start = start.min(len);
    end = end.min(len);
    if start > end {
        start = end;
    }
    start..end
}

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
    type Id: 'static + Hash + Ord + Clone;
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
/// owning snapshot built from the exact data the notification carried, so a
/// retained snapshot stays coherent with the [`CollectionChange`] it arrived
/// with even when another observer mutates the source first.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a WaterUI view collection",
    label = "expected a collection of views",
    note = "`Views` is implemented by `ForEach`, `Constant`, `Vec` and arrays of views, and `AnyViews`. To turn a data collection into views use `ForEach::new(data, f)` — exposed as `Lazy::for_each` / `List::for_each` — whose items must implement `Identifiable`."
)]
pub trait Views {
    /// The type of unique identifier for items in the collection.
    /// Must implement `Hash` and `Ord` to ensure uniqueness and ordering.
    type Id: 'static + Hash + Ord + Clone;
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
    /// requested range resolved against that notification's captured data,
    /// plus a [`CollectionChange`] naming which positions the notification
    /// touched — both in collection-wide index space. The watcher may retain
    /// the snapshot beyond the callback.
    ///
    /// Returns a guard that will unregister the watcher when dropped.
    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl Fn(Context<Self::Snapshot>, CollectionChange) + 'static,
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
    type Id = SelfId<RawId>;
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
        watcher: impl Fn(Context<Self::Snapshot>, CollectionChange) + 'static,
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
    type Id = SelfId<RawId>;
    type View = V;

    fn range(&self) -> Range<usize> {
        self.0.range()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.0.get_id(index).map(SelfId::new)
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.0.get_view(index)
    }
}

/// The boxed watcher callback an erased collection registers — it receives
/// an owning erased snapshot plus the notification's change report.
type BoxAnyViewsWatcher<V> =
    Box<dyn Fn(Context<AnyViewsSnapshot<V>>, CollectionChange) + 'static>;

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
    type Id = SelfId<RawId>;
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
        watcher: impl Fn(Context<Self::Snapshot>, CollectionChange) + 'static,
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
#[derive(Debug)]
pub struct ForEach<C, F, V>
where
    C: Collection,
    C::Item: Identifiable,
    F: Fn(C::Item) -> V,
    V: View,
{
    data: C,
    generator: Rc<F>,
}

impl<C, F, V> Clone for ForEach<C, F, V>
where
    C: Collection + Clone,
    C::Item: Identifiable,
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
    C::Item: Identifiable,
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

#[derive(Clone)]
struct CollectionLenSignal<C>(C);

impl<C> Signal for CollectionLenSignal<C>
where
    C: Collection + Clone,
{
    type Output = usize;
    type Guard = C::Guard;

    fn snapshot(&self) -> Self::Output {
        self.0.len()
    }

    fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {
        self.0.watch(.., move |ctx, _change| {
            let len = ctx.value().len();
            watcher(ctx.map(move |_| len));
        })
    }
}

impl<C, F, V> Collection for ForEach<C, F, V>
where
    C: Collection,
    C::Item: Identifiable,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
    type Item = <C::Item as Identifiable>::Id;
    type Guard = C::Guard;
    fn get(&self, index: usize) -> Option<Self::Item> {
        self.data.get(index).map(|item| item.id())
    }

    fn len(&self) -> usize {
        self.data.len()
    }

    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl for<'a> Fn(Context<&'a [Self::Item]>, CollectionChange) + 'static, // watcher will receive a slice of items, its range is decided by the range parameter
    ) -> Self::Guard {
        self.data.watch(range, move |ctx, change| {
            let ctx = ctx.map(|value| value.iter().map(Identifiable::id).collect::<Vec<_>>());

            watcher(ctx.as_deref(), change);
        })
    }
}

/// The immutable snapshot a [`ForEach`] captures: cloned row data in an
/// `Rc<[C::Item]>` plus a shared handle to the generator.
///
/// Cloning the snapshot is O(1) — the row data and the generator are both
/// reference-counted. `get_view` invokes the shared generator only for the
/// requested row.
pub struct ForEachSnapshot<C, F, V>
where
    C: Collection,
    C::Item: Identifiable,
{
    data: Rc<[C::Item]>,
    start: usize,
    generator: Rc<F>,
    _marker: PhantomData<fn() -> V>,
}

impl<C, F, V> Debug for ForEachSnapshot<C, F, V>
where
    C: Collection,
    C::Item: Identifiable,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<C, F, V> Clone for ForEachSnapshot<C, F, V>
where
    C: Collection,
    C::Item: Identifiable,
{
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            start: self.start,
            generator: self.generator.clone(),
            _marker: PhantomData,
        }
    }
}

impl<C, F, V> ViewSnapshot for ForEachSnapshot<C, F, V>
where
    C: Collection,
    C::Item: Identifiable + Clone,
    F: Fn(C::Item) -> V,
    V: View,
{
    type Id = <C::Item as Identifiable>::Id;
    type View = V;

    fn range(&self) -> Range<usize> {
        self.start..self.start + self.data.len()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.data
            .as_ref()
            .get(index.checked_sub(self.start)?)
            .map(Identifiable::id)
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.data
            .as_ref()
            .get(index.checked_sub(self.start)?)
            .map(|item| (self.generator)(item.clone()))
    }
}

impl<C, F, V> Views for ForEach<C, F, V>
where
    C: Collection + Clone,
    C::Item: Identifiable + Clone,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
    type Id = <C::Item as Identifiable>::Id;
    type View = V;
    type Guard = C::Guard;
    type Snapshot = ForEachSnapshot<C, F, V>;

    fn len(&self) -> Computed<usize> {
        Computed::new(CollectionLenSignal(self.data.clone()))
    }

    fn snapshot(&self) -> Self::Snapshot {
        let len = self.data.len();
        let data: Rc<[C::Item]> = (0..len)
            .map(|index| {
                self.data
                    .get(index)
                    .expect("a collection must honor `get` inside its reported length")
            })
            .collect();
        ForEachSnapshot {
            data,
            start: 0,
            generator: self.generator.clone(),
            _marker: PhantomData,
        }
    }

    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl Fn(Context<Self::Snapshot>, CollectionChange) + 'static,
    ) -> Self::Guard {
        let bounds = (range.start_bound().cloned(), range.end_bound().cloned());
        let generator = self.generator.clone();
        // Subscribe to the whole collection and slice each event's immutable
        // data ourselves: the requested bounds resolve against the length the
        // notification carried, not the live length — another observer may
        // already have mutated the source by the time this watcher runs.
        self.data.watch(.., move |ctx, change| {
            let items: &[C::Item] = ctx.value();
            let range = resolve_range(bounds, items.len());
            let snapshot = ForEachSnapshot {
                data: Rc::from(&items[range.clone()]),
                start: range.start,
                generator: generator.clone(),
                _marker: PhantomData,
            };
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

impl<C> Collection for Constant<C>
where
    C: Views + 'static,
{
    type Item = SelfId<usize>;
    type Guard = ();

    fn get(&self, index: usize) -> Option<Self::Item> {
        if index < self.captured.len() {
            Some(SelfId::new(index))
        } else {
            None
        }
    }

    fn len(&self) -> usize {
        self.captured.len()
    }

    fn watch(
        &self,
        _range: impl RangeBounds<usize>,
        _watcher: impl for<'a> Fn(Context<&'a [Self::Item]>, CollectionChange) + 'static, // watcher will receive a slice of items, its range is decided by the range parameter
    ) -> Self::Guard {
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
    type Id = SelfId<usize>;
    type View = S::View;

    fn range(&self) -> Range<usize> {
        self.source.range()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.source
            .range()
            .contains(&index)
            .then(|| SelfId::new(index))
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.source.get_view(index)
    }
}

impl<C> Views for Constant<C>
where
    C: Views,
{
    type Id = SelfId<usize>;
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
        _watcher: impl Fn(Context<Self::Snapshot>, CollectionChange) + 'static,
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
    type Id = SelfId<usize>;
    type View = V;

    fn range(&self) -> Range<usize> {
        self.start..self.start + self.data.len()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.data
            .as_ref()
            .get(index.checked_sub(self.start)?)
            .map(|_| SelfId::new(index))
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.data
            .as_ref()
            .get(index.checked_sub(self.start)?)
            .cloned()
    }
}

impl<V: View + Clone> Views for Vec<V> {
    type Id = SelfId<usize>;
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
        _watcher: impl Fn(Context<Self::Snapshot>, CollectionChange) + 'static,
    ) -> Self::Guard {
        // No-op for Vec
    }
}

impl<V: View + Clone, const N: usize> Views for [V; N] {
    type Id = SelfId<usize>;
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
        _watcher: impl Fn(Context<Self::Snapshot>, CollectionChange) + 'static,
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
        watcher: impl Fn(Context<Self::Snapshot>, CollectionChange) + 'static,
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
        type Id = SelfId<usize>;
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
            _watcher: impl Fn(Context<Self::Snapshot>, CollectionChange) + 'static,
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
        let list = items(&[1, 2, 3, 4]);
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

        // The initial emission covers the requested window; the change report
        // stays in collection-wide space (`populated` over the whole event).
        assert_eq!(snapshots.borrow().as_slice(), &[(1..3, vec![2, 3])]);

        list.insert(0, TestItem { id: 9 });
        let borrowed = snapshots.borrow();
        let latest_snapshot = borrowed.last().expect("watch should emit after insert");
        assert_eq!(latest_snapshot, &(1..3, vec![1, 2]));
    }

    #[test]
    fn snapshot_retains_removed_and_reordered_rows() {
        let list = items(&[1, 2, 3]);
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
        let list = items(&[1, 2, 3]);
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
    fn reentrant_producer_mutation_cannot_rewrite_a_delivered_snapshot() {
        let list = items(&[1, 2]);
        let views = ForEach::new(list.clone(), |_item| ());

        // Watcher A mutates the source once, inside the notification triggered
        // by the second mutation below. `List` snapshots the event before
        // dispatching, so watcher B's snapshot must still hold that event's
        // data, not the reentrant result.
        let mutations = Rc::new(Cell::new(0usize));
        let _guard_a = Views::watch(&views, .., {
            let list = list.clone();
            move |_ctx, _change| {
                if mutations.get() == 1 {
                    mutations.set(2);
                    list.push(TestItem { id: 1000 });
                } else {
                    mutations.set(mutations.get() + 1);
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

        assert_eq!(seen.borrow().as_slice(), &[vec![1, 2]]);

        // This insert notifies A first; A pushes 1000 reentrantly, which
        // dispatches the nested notification `[9, 1, 2, 1000]` to B before the
        // outer notification reaches it. B's snapshot for the outer event is
        // still the event data `[9, 1, 2]` — the concurrent mutation cannot
        // rewrite what was already captured.
        list.insert(0, TestItem { id: 9 });
        assert_eq!(
            seen.borrow().as_slice(),
            &[vec![1, 2], vec![9, 1, 2, 1000], vec![9, 1, 2]]
        );
    }

    #[test]
    fn erased_snapshots_share_one_id_mapping() {
        let list = items(&[1, 2, 3]);
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
        let list = items(&[1, 2, 3]);
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
                .filter_map(|index| snapshot.get_id(index).map(SelfId::into_inner))
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
            let list = items(&[1, 2]);
            let views = ForEach::new(list, |_item| ());
            let guard = Views::watch(&views, .., {
                let kept = kept.clone();
                move |ctx, _change| {
                    *kept.borrow_mut() = Some(Box::new(ctx.into_value()));
                }
            });
            drop(guard);
        }
        let borrowed = kept.borrow();
        let snapshot = borrowed
            .as_ref()
            .expect("the initial emission delivered a snapshot");
        let range = snapshot.range();
        assert_eq!(ids_in(&**snapshot, range), vec![1, 2]);
    }
}