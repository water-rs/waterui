//! Collection of view-related utilities for managing and transforming UI components.
//!
//! This module provides types and traits for working with collections of views in a type-safe
//! and efficient manner. It includes utilities for type erasure, transformation, and identity
//! tracking of view collections.

use crate::id::Id as RawId;
use crate::{AnyView, View};
use alloc::fmt::Debug;
use alloc::{boxed::Box, collections::BTreeMap, rc::Rc, vec::Vec};
use core::any::type_name;
use core::num::NonZeroI32;
use core::ops::{Bound, RangeBounds};
use core::{
    cell::{Cell, RefCell},
    hash::Hash,
};
use nami::collection::Collection;
use nami::watcher::{BoxWatcherGuard, Context, WatcherGuard};

use crate::id::{Identifable, SelfId};

/// A trait for collections that can provide unique identifiers for their elements.
///
/// `Views` extends the `Collection` trait by adding identity tracking capabilities.
/// This allows for efficient diffing and reconciliation of UI elements during updates.
/// Tip: the `get` method of `Collection` should return a unique identifier for each item.
pub trait Views {
    /// The type of unique identifier for items in the collection.
    /// Must implement `Hash` and `Ord` to ensure uniqueness and ordering.
    type Id: 'static + Hash + Ord + Clone;
    /// The type of guard returned when registering a watcher.
    type Guard: WatcherGuard;
    /// The view type that this collection produces for each element.
    type View: View;
    /// Returns the unique identifier for the item at the specified index, or `None` if out of bounds.
    fn get_id(&self, index: usize) -> Option<Self::Id>;
    /// Returns the number of items in the collection.
    fn len(&self) -> usize;

    /// Returns `true` if the collection contains no elements.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Registers a watcher for changes in the specified range of the collection.
    ///
    /// Returns a guard that will unregister the watcher when dropped.
    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl for<'a> Fn(Context<&'a [Self::Id]>) + 'static, // watcher will receive a slice of items, its range is decided by the range parameter
    ) -> Self::Guard;

    /// Returns the view at the specified index, or `None` if the index is out of bounds.
    fn get_view(&self, id: usize) -> Option<Self::View>;
}

/// A type-erased container for `Views` collections.
///
/// `AnyViews` provides a uniform interface to different views collections
/// by wrapping them in a type-erased container. This enables working with
/// heterogeneous view collections through a common interface.
pub struct AnyViews<V>(Box<dyn AnyViewsImpl<View = V>>);

impl<V> Debug for AnyViews<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
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
}

impl<V> From<AnyViews<V>> for SharedAnyViews<V> {
    fn from(value: AnyViews<V>) -> Self {
        Self(Rc::from(value.0))
    }
}

impl<V> Debug for SharedAnyViews<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<V: View> Views for SharedAnyViews<V> {
    type Id = SelfId<RawId>;
    type Guard = BoxWatcherGuard;
    type View = V;
    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.0.get_id(index).map(SelfId::new)
    }
    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.0.get_view(index)
    }
    fn len(&self) -> usize {
        self.0.len()
    }

    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl for<'a> Fn(Context<&'a [Self::Id]>) + 'static, // watcher will receive a slice of items, its range is decided by the range parameter
    ) -> Self::Guard {
        self.0.watch(
            (range.start_bound().cloned(), range.end_bound().cloned()),
            Box::new(move |ctx| {
                let ctx =
                    ctx.map(|value| value.iter().copied().map(SelfId::new).collect::<Vec<_>>());
                watcher(ctx.as_deref());
            }),
        )
    }
}

trait AnyViewsImpl {
    type View;

    fn get_view(&self, index: usize) -> Option<Self::View>;
    fn get_id(&self, index: usize) -> Option<RawId>;
    fn len(&self) -> usize;
    #[allow(clippy::type_complexity)]
    fn watch(
        &self,
        range: (Bound<usize>, Bound<usize>),
        watcher: Box<dyn for<'a> Fn(Context<&'a [RawId]>) + 'static>,
    ) -> BoxWatcherGuard;
}

#[derive(Debug)]
struct IdGenerator<Id> {
    map: RefCell<BTreeMap<Id, i32>>,
    counter: Cell<i32>,
}

impl<Id: Hash + Ord> Default for IdGenerator<Id> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Id: Hash + Ord> IdGenerator<Id> {
    pub const fn new() -> Self {
        Self {
            map: RefCell::new(BTreeMap::new()),
            counter: Cell::new(i32::MIN),
        }
    }
    pub fn to_id(&self, value: Id) -> RawId {
        let mut this = self.map.borrow_mut();
        if let Some(&id) = this.get(&value) {
            return RawId::from(unsafe { NonZeroI32::new_unchecked(id) });
        }
        let id = self.counter.get();
        self.counter
            .set(id.checked_add(1).expect("id counter should not overflow"));
        this.insert(value, id);
        RawId::from(unsafe { NonZeroI32::new_unchecked(id) })
    }
}

struct IntoAnyViews<V>
where
    V: Views,
{
    contents: V,
    id: Rc<IdGenerator<V::Id>>,
}

impl<V> IntoAnyViews<V>
where
    V: Views + 'static,
{
    pub fn new(contents: V) -> Self {
        Self {
            contents,
            id: Rc::default(),
        }
    }
}

impl<V> AnyViewsImpl for IntoAnyViews<V>
where
    V: Views + 'static,
{
    type View = V::View;

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.contents.get_view(index)
    }

    fn get_id(&self, index: usize) -> Option<RawId> {
        self.contents.get_id(index).map(|item| self.id.to_id(item))
    }

    fn len(&self) -> usize {
        self.contents.len()
    }

    fn watch(
        &self,
        range: (Bound<usize>, Bound<usize>),
        watcher: Box<dyn for<'a> Fn(Context<&'a [RawId]>) + 'static>,
    ) -> BoxWatcherGuard {
        let id = self.id.clone();
        Box::new(self.contents.watch(range, move |ctx| {
            let ctx = ctx.map(|value| {
                value
                    .iter()
                    .map(|data| id.to_id(data.clone()))
                    .collect::<Vec<_>>()
            });
            watcher(ctx.as_deref());
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
        Self(Box::new(IntoAnyViews {
            id: Rc::new(IdGenerator::<C::Id>::new()),
            contents,
        }))
    }
}

impl<V> Views for AnyViews<V>
where
    V: View,
{
    type Id = SelfId<RawId>;
    type Guard = BoxWatcherGuard;
    type View = V;
    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.0.get_id(index).map(SelfId::new)
    }
    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.0.get_view(index)
    }
    fn len(&self) -> usize {
        self.0.len()
    }

    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl for<'a> Fn(Context<&'a [Self::Id]>) + 'static, // watcher will receive a slice of items, its range is decided by the range parameter
    ) -> Self::Guard {
        self.0.watch(
            (range.start_bound().cloned(), range.end_bound().cloned()),
            Box::new(move |ctx| {
                let ctx =
                    ctx.map(|value| value.iter().copied().map(SelfId::new).collect::<Vec<_>>());
                watcher(ctx.as_deref());
            }),
        )
    }
}
/// A utility for transforming elements of a collection with a mapping function.
///
/// `ForEach` applies a transformation function to each element of a source collection,
/// producing a new collection with the transformed elements. This is useful for
/// transforming data models into view representations.
#[derive(Debug, Clone)]
pub struct ForEach<C, F, V>
where
    C: Collection,
    C::Item: Identifable,
    F: Fn(C::Item) -> V,
    V: View,
{
    data: C,
    generator: F,
}

impl<C, F, V> ForEach<C, F, V>
where
    C: Collection,
    C::Item: Identifable,
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
    pub const fn new(data: C, generator: F) -> Self {
        Self { data, generator }
    }

    /// Consumes the `ForEach` and returns the original data collection and generator function.
    ///
    /// # Returns
    /// A tuple containing the original data collection and generator function
    pub fn into_inner(self) -> (C, F) {
        (self.data, self.generator)
    }
}

/// Represents a single transformed item, pairing data with a generator function to produce a view.
#[derive(Debug)]
pub struct ForEachItem<T, F, V>
where
    F: Fn(T) -> V,
    V: View,
{
    data: T,

    generator: Rc<F>,
}

impl<C, F, V> Collection for ForEach<C, F, V>
where
    C: Collection,
    C::Item: Identifable,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
    type Item = <C::Item as Identifable>::Id;
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
        watcher: impl for<'a> Fn(Context<&'a [Self::Item]>) + 'static, // watcher will receive a slice of items, its range is decided by the range parameter
    ) -> Self::Guard {
        self.data.watch(range, move |ctx| {
            let ctx = ctx.map(|value| {
                value
                    .iter()
                    .map(super::id::Identifable::id)
                    .collect::<Vec<_>>()
            });

            watcher(ctx.as_deref());
        })
    }
}

impl<C, F, V> Views for ForEach<C, F, V>
where
    C: Collection,
    C::Item: Identifable,
    F: 'static + Fn(C::Item) -> V,
    V: View,
{
    type Id = <C::Item as Identifable>::Id;
    type View = V;
    type Guard = C::Guard;
    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.data.get(index).map(|item| item.id())
    }
    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.data.get(index).map(|item| (self.generator)(item))
    }
    fn len(&self) -> usize {
        self.data.len()
    }
    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl for<'a> Fn(Context<&'a [Self::Id]>) + 'static, // watcher will receive a slice of items, its range is decided by the range parameter
    ) -> Self::Guard {
        self.data.watch(range, move |ctx| {
            let ctx = ctx.map(|value| {
                value
                    .iter()
                    .map(super::id::Identifable::id)
                    .collect::<Vec<_>>()
            });

            watcher(ctx.as_deref());
        })
    }
}

/// A statically sized collection that never changes, removes, or adds items.
///
/// `Constant` wraps a collection and provides a stable view collection where
/// elements are identified by their index position. This is useful for static
/// lists of views that don't need to be updated or reordered.
#[derive(Debug, Clone)]
pub struct Constant<C>
where
    C: Collection,
    C::Item: View,
{
    value: C,
}

impl<C> Constant<C>
where
    C: Collection,
    C::Item: View,
{
    /// Creates a new `Constant` collection from the provided collection.
    ///
    /// # Parameters
    /// * `value` - The underlying collection to be wrapped as a constant view collection
    ///
    /// # Returns
    /// A new `Constant` instance containing the provided collection
    pub const fn new(value: C) -> Self {
        Self { value }
    }
}

impl<C> Collection for Constant<C>
where
    C: Collection + Clone,
    C::Item: View,
{
    type Item = SelfId<usize>;
    type Guard = ();

    fn get(&self, index: usize) -> Option<Self::Item> {
        if index < self.value.len() {
            Some(SelfId::new(index))
        } else {
            None
        }
    }

    fn len(&self) -> usize {
        self.value.len()
    }

    fn watch(
        &self,
        _range: impl RangeBounds<usize>,
        _watcher: impl for<'a> Fn(Context<&'a [Self::Item]>) + 'static, // watcher will receive a slice of items, its range is decided by the range parameter
    ) -> Self::Guard {
    }
}

impl<V> Views for Constant<V>
where
    V: Collection + Clone,
    V::Item: View,
{
    type Id = SelfId<usize>;
    type Guard = ();
    type View = V::Item;

    fn len(&self) -> usize {
        self.value.len()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        if index < self.value.len() {
            Some(SelfId::new(index))
        } else {
            None
        }
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.value.get(index)
    }

    fn watch(
        &self,
        _range: impl RangeBounds<usize>,
        _watcher: impl for<'a> Fn(Context<&'a [Self::Id]>) + 'static, // watcher will receive a slice of items, its range is decided by the range parameter
    ) -> Self::Guard {
        // No-op for Constant
    }
}

impl<V: View + Clone> Views for Vec<V> {
    type Id = SelfId<usize>;
    type Guard = ();
    type View = V;

    fn len(&self) -> usize {
        self.len()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        if index < self.len() {
            Some(SelfId::new(index))
        } else {
            None
        }
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.get(index)
    }

    fn watch(
        &self,
        _range: impl RangeBounds<usize>,
        _watcher: impl for<'a> Fn(Context<&'a [Self::Id]>) + 'static, // watcher will receive a slice of items, its range is decided by the range parameter
    ) -> Self::Guard {
        // No-op for Vec
    }
}

impl<V: View + Clone, const N: usize> Views for [V; N] {
    type Id = SelfId<usize>;
    type Guard = ();
    type View = V;

    fn len(&self) -> usize {
        self.as_ref().len()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        if index < self.as_ref().len() {
            Some(SelfId::new(index))
        } else {
            None
        }
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.get(index)
    }

    fn watch(
        &self,
        _range: impl RangeBounds<usize>,
        _watcher: impl for<'a> Fn(Context<&'a [Self::Id]>) + 'static, // watcher will receive a slice of items, its range is decided by the range parameter
    ) -> Self::Guard {
        // No-op for arrays
    }
}

/// A view collection that transforms views from a source collection using a mapping function.
///
/// `Map` wraps an existing view collection and applies a transformation function to each
/// view when it is accessed, allowing for lazy transformation of views.
#[derive(Debug, Clone)]
pub struct Map<C, F> {
    source: C,
    f: F,
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
    pub const fn new(source: C, f: F) -> Self {
        Self { source, f }
    }
}

impl<C, F, V> Views for Map<C, F>
where
    C: Views,
    F: Clone + Fn(C::View) -> V,
    V: View,
{
    type Id = C::Id;
    type Guard = C::Guard;
    type View = V;

    fn len(&self) -> usize {
        self.source.len()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.source.get_id(index)
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.source.get_view(index).map(&self.f)
    }

    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl for<'a> Fn(Context<&'a [Self::Id]>) + 'static, // watcher will receive a slice of items, its range is decided by the range parameter
    ) -> Self::Guard {
        self.source.watch(range, watcher)
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
