#![no_std]
extern crate alloc;

mod filter;
mod impls;
mod map;
use alloc::boxed::Box;
use core::{any::type_name, fmt::Debug, future::Future};
use filter::Filter;
use futures_lite::Stream;
use map::Map;

use core::pin::Pin;

#[allow(clippy::len_without_is_empty)]
pub trait LazyList {
    type Item;
    fn get(&self, index: usize) -> impl Future<Output = Option<Self::Item>>;
    fn len(&self) -> Option<usize>;
    fn iter(&self) -> impl Stream<Item = Self::Item>;
    fn rev_iter(&self) -> impl Stream<Item = Self::Item>;
}

pub trait LazyListExt: LazyList + Sized {
    fn map<F, Output>(self, f: F) -> Map<Self, F, Output>
    where
        F: Fn(Self::Item) -> Output;

    fn filter<F>(self, f: F) -> Filter<Self, F>
    where
        F: Fn(&Self::Item) -> bool;
}

impl<L: LazyList> LazyListExt for L {
    fn map<F, Output>(self, f: F) -> Map<Self, F, Output>
    where
        F: Fn(Self::Item) -> Output,
    {
        Map::new(self, f)
    }
    fn filter<F>(self, f: F) -> Filter<Self, F>
    where
        F: Fn(&Self::Item) -> bool,
    {
        Filter::new(self, f)
    }
}

trait LazyListImpl {
    type Item;
    fn get<'a>(&'a self, index: usize) -> Pin<Box<dyn 'a + Future<Output = Option<Self::Item>>>>
    where
        Self: 'a;
    fn len(&self) -> Option<usize>;
    fn iter<'a>(&'a self) -> Pin<Box<dyn 'a + Stream<Item = Self::Item>>>
    where
        Self: 'a;
    fn rev_iter<'a>(&'a self) -> Pin<Box<dyn 'a + Stream<Item = Self::Item>>>
    where
        Self: 'a;
}

impl<T: LazyList> LazyListImpl for T {
    type Item = T::Item;
    fn get<'a>(&'a self, index: usize) -> Pin<Box<dyn 'a + Future<Output = Option<Self::Item>>>>
    where
        Self: 'a,
    {
        Box::pin(LazyList::get(self, index))
    }
    fn len(&self) -> Option<usize> {
        LazyList::len(self)
    }
    fn iter<'a>(&'a self) -> Pin<Box<dyn 'a + Stream<Item = Self::Item>>>
    where
        Self: 'a,
    {
        Box::pin(LazyList::iter(self))
    }
    fn rev_iter<'a>(&'a self) -> Pin<Box<dyn 'a + Stream<Item = Self::Item>>>
    where
        Self: 'a,
    {
        Box::pin(LazyList::rev_iter(self))
    }
}

pub struct AnyLazyList<T>(Box<dyn LazyListImpl<Item = T>>);

impl<T> PartialEq for AnyLazyList<T> {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

impl<T> Debug for AnyLazyList<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(type_name::<Self>())
    }
}

impl<T> AnyLazyList<T> {
    pub fn new(list: impl LazyList<Item = T> + 'static) -> Self {
        Self(Box::new(list))
    }
}

impl<T> LazyList for AnyLazyList<T> {
    type Item = T;

    fn get(&self, index: usize) -> impl Future<Output = Option<Self::Item>> {
        self.0.get(index)
    }

    fn len(&self) -> Option<usize> {
        self.0.len()
    }

    fn iter(&self) -> impl Stream<Item = Self::Item> {
        self.0.iter()
    }

    fn rev_iter(&self) -> impl Stream<Item = Self::Item> {
        self.0.rev_iter()
    }
}
