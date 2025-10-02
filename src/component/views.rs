//! Collection of view-related utilities for managing and transforming UI components.
//!
//! This module provides types and traits for working with collections of views in a type-safe
//! and efficient manner. It includes utilities for type erasure, transformation, and identity
//! tracking of view collections.

use alloc::fmt::Debug;
use alloc::{collections::BTreeMap, rc::Rc};
use core::any::type_name;
use core::ops::{Bound, RangeBounds};
use core::{
    cell::{Cell, RefCell},
    hash::Hash,
    num::NonZeroUsize,
};
use nami::collection::Collection;
use nami::watcher::{BoxWatcher, BoxWatcherGuard, Context};
use waterui_core::View;

use waterui_core::id::Identifable;

/// A trait for collections that can provide unique identifiers for their elements.
///
/// `Views` extends the `Collection` trait by adding identity tracking capabilities.
/// This allows for efficient diffing and reconciliation of UI elements during updates.
pub trait Views: Collection<Item: Hash + Ord> + Clone {
    /// The view type that this collection produces for each element.
    type View: View;
    /// Returns the view at the specified index, or `None` if the index is out of bounds.
    fn get_view(&self, index: usize) -> Option<Self::View>;
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

trait AnyViewsImpl {
    type View;

    fn get_view(&self, index: usize) -> Option<Self::View>;
    fn get_id(&self, index: usize) -> Option<NonZeroUsize>;
    fn len(&self) -> usize;
    fn watch(
        &self,
        range: (Bound<usize>, Bound<usize>),
        watcher: BoxWatcher<Vec<NonZeroUsize>>,
    ) -> BoxWatcherGuard;
    fn clone(&self) -> Box<dyn AnyViewsImpl<View = Self::View>>;
}

#[derive(Debug)]
struct IdGenerator<Id> {
    map: RefCell<BTreeMap<Id, NonZeroUsize>>,
    counter: Cell<NonZeroUsize>,
}

impl<Id: Hash + Ord> IdGenerator<Id> {
    pub fn new() -> Self {
        Self {
            map: RefCell::default(),
            counter: Cell::new(NonZeroUsize::MIN),
        }
    }
    pub fn to_id(&self, value: Id) -> NonZeroUsize {
        let mut this = self.map.borrow_mut();
        if let Some(id) = this.get(&value) {
            *id
        } else {
            let id = self.counter.get();
            self.counter.set(id.checked_add(1).unwrap());
            this.insert(value, id);
            id
        }
    }
}

struct IntoAnyViews<V>
where
    V: Views,
{
    contents: V,
    id: Rc<IdGenerator<V::Item>>,
}

impl<V> Clone for IntoAnyViews<V>
where
    V: Views,
{
    fn clone(&self) -> Self {
        Self {
            contents: self.contents.clone(),
            id: self.id.clone(),
        }
    }
}
impl<V> AnyViewsImpl for IntoAnyViews<V>
where
    V: Views,
{
    type View = V::View;

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.contents.get_view(index)
    }

    fn get_id(&self, index: usize) -> Option<NonZeroUsize> {
        self.contents.get(index).map(|item| self.id.to_id(item))
    }

    fn len(&self) -> usize {
        self.contents.len()
    }

    fn watch(
        &self,
        range: (Bound<usize>, Bound<usize>),
        watcher: BoxWatcher<Vec<NonZeroUsize>>,
    ) -> BoxWatcherGuard {
        let id = self.id.clone();
        Box::new(self.contents.watch(range, move |ctx| {
            let metadata = ctx.metadata;
            let values: Vec<_> = ctx.value.into_iter().map(|data| id.to_id(data)).collect();
            watcher(Context::new(values, metadata));
        }))
    }

    fn clone(&self) -> Box<dyn AnyViewsImpl<View = Self::View>> {
        Box::new(Clone::clone(self))
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
            id: Rc::new(IdGenerator::<C::Item>::new()),
            contents,
        }))
    }
}

impl<V> Clone for AnyViews<V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<V: 'static> Collection for AnyViews<V> {
    type Item = NonZeroUsize;
    type Guard = BoxWatcherGuard;
    fn get(&self, index: usize) -> Option<Self::Item> {
        self.0.get_id(index)
    }

    fn len(&self) -> usize {
        self.0.len()
    }
    fn watch(
        &self,
        range: impl RangeBounds<usize>,
        watcher: impl Fn(nami::watcher::Context<Vec<Self::Item>>) + 'static,
    ) -> Self::Guard {
        self.0.watch(
            (range.start_bound().cloned(), range.end_bound().cloned()),
            Box::new(watcher),
        )
    }
}

impl<V> Views for AnyViews<V>
where
    V: View,
{
    type View = V;
    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.0.get_view(index)
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
    C::Item: Identifable,
    F: Fn(C::Item) -> V,
    V: View,
{
    data: C,
    generator: Rc<F>,
}

impl<C, F, V> Clone for ForEach<C, F, V>
where
    C: Collection,
    C::Item: Identifable,
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
    pub fn new(data: C, generator: F) -> Self {
        Self {
            data,
            generator: Rc::new(generator),
        }
    }
}

#[derive(Debug)]
pub struct ForEachItem<T, F, V>
where
    F: Fn(T) -> V,
    V: View,
{
    #[allow(dead_code)]
    data: T,
    #[allow(dead_code)]
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
        watcher: impl Fn(nami::watcher::Context<Vec<Self::Item>>) + 'static,
    ) -> Self::Guard {
        self.data.watch(range, move |ctx| {
            let metadata = ctx.metadata;
            let values: Vec<_> = ctx.value.into_iter().map(|data| data.id()).collect();
            watcher(Context::new(values, metadata));
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
    type View = V;
    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.data.get(index).map(|item| (self.generator)(item))
    }
}
