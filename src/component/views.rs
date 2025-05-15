//! Collection of view-related utilities for managing and transforming UI components.
//!
//! This module provides types and traits for working with collections of views in a type-safe
//! and efficient manner. It includes utilities for type erasure, transformation, and identity
//! tracking of view collections.

use alloc::fmt::Debug;
use alloc::{collections::BTreeMap, rc::Rc};
use core::any::type_name;
use core::{
    cell::{Cell, RefCell},
    hash::Hash,
    marker::PhantomData,
    num::NonZeroUsize,
};
use waterui_reactive::watcher::BoxWatcher;

use waterui_core::id::{Identifable, IdentifableExt, SelfId};

/// A trait for collections that can provide unique identifiers for their elements.
///
/// `Views` extends the `Collection` trait by adding identity tracking capabilities.
/// This allows for efficient diffing and reconciliation of UI elements during updates.
pub trait Views: Collection {
    /// The type of identifier used for elements in the collection.
    /// Must implement `Hash` and `Ord` to enable efficient lookups.
    type Id: Hash + Ord;

    /// Returns the unique identifier for the element at the specified index.
    ///
    /// # Parameters
    /// * `index` - The position in the collection to retrieve the ID for
    ///
    /// # Returns
    /// * `Some(Id)` if the element exists at the specified index
    /// * `None` if the index is out of bounds
    fn get_id(&self, index: usize) -> Option<Self::Id>;
}

/// A type-erased container for `Views` collections.
///
/// `AnyViews` provides a uniform interface to different views collections
/// by wrapping them in a type-erased container. This enables working with
/// heterogeneous view collections through a common interface.
pub struct AnyViews<V>(Rc<dyn Views<Item = V, Id = SelfId<NonZeroUsize>>>);

impl<V> Debug for AnyViews<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(type_name::<Self>())
    }
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

struct IntoAnyViews<V, Id> {
    contents: V,
    id: IdGenerator<Id>,
}

impl<V, Id> Collection for IntoAnyViews<V, Id>
where
    V: Views<Id = Id>,
    Id: Ord + Hash,
{
    type Item = V::Item;
    fn get(&self, index: usize) -> Option<Self::Item> {
        self.contents.get(index)
    }
    fn remove(&self, index: usize) {
        self.contents.remove(index);
    }
    fn len(&self) -> usize {
        self.contents.len()
    }
    fn add_watcher(&self, watcher: BoxWatcher<()>) -> waterui_reactive::watcher::WatcherGuard {
        self.contents.watch(watcher)
    }
}

impl<V, Id> Views for IntoAnyViews<V, Id>
where
    V: Views<Id = Id>,
    Id: Ord + Hash,
{
    type Id = SelfId<NonZeroUsize>;
    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.contents
            .get_id(index)
            .map(|value| self.id.to_id(value).self_id())
    }
}

impl<V> AnyViews<V> {
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
    pub fn new(contents: impl Views<Item = V> + 'static) -> Self {
        Self(Rc::new(IntoAnyViews {
            id: IdGenerator::new(),
            contents,
        }))
    }
}

impl<V> Clone for AnyViews<V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<V> Collection for AnyViews<V> {
    type Item = V;
    fn get(&self, index: usize) -> Option<Self::Item> {
        self.0.get(index)
    }
    fn remove(&self, index: usize) {
        self.0.remove(index);
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn watch(
        &self,
        watcher: waterui_reactive::watcher::Watcher<()>,
    ) -> waterui_reactive::watcher::WatcherGuard {
        self.0.watch(watcher)
    }
}

impl<V> Views for AnyViews<V> {
    type Id = SelfId<NonZeroUsize>;
    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.0.get_id(index)
    }
}

/// A utility for transforming elements of a collection with a mapping function.
///
/// `ForEach` applies a transformation function to each element of a source collection,
/// producing a new collection with the transformed elements. This is useful for
/// transforming data models into view representations.
#[derive(Debug)]
pub struct ForEach<C, F, V, Output> {
    data: C,
    generator: F,
    _marker: PhantomData<(V, Output)>,
}

impl<C, F, V, Output> ForEach<C, F, V, Output> {
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
            generator,
            _marker: PhantomData,
        }
    }
}

impl<C, Id, F, V, Output> Collection for ForEach<C, F, V, Output>
where
    C: Collection,
    C::Item: Identifable<Id = Id>,
    F: Fn(C::Item) -> V,
    V: Into<Output>,
{
    type Item = Output;
    fn get(&self, index: usize) -> Option<Self::Item> {
        self.data
            .get(index)
            .map(|value| (self.generator)(value).into())
    }

    fn len(&self) -> usize {
        self.data.len()
    }
    fn remove(&self, index: usize) {
        self.data.remove(index);
    }
    fn watch(
        &self,
        watcher: waterui_reactive::watcher::Watcher<()>,
    ) -> waterui_reactive::watcher::WatcherGuard {
        self.data.watch(watcher)
    }
}

impl<C, F, V, Output> Views for ForEach<C, F, V, Output>
where
    C: Collection,
    C::Item: Identifable,
    F: Fn(C::Item) -> V,
    V: Into<Output>,
{
    type Id = <C::Item as Identifable>::Id;
    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.data.get(index).map(|data| data.id())
    }
}
