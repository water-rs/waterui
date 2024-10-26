use crate::LazyList;
use alloc::{boxed::Box, rc::Rc, vec::Vec};
use core::ops::Deref;
use futures_lite::{stream::iter, Stream};

impl<T> LazyList for [T]
where
    T: Clone,
{
    type Item = T;

    async fn get(&self, index: usize) -> Option<Self::Item> {
        <[T]>::get(self, index).cloned()
    }

    fn len(&self) -> Option<usize> {
        Some(<[T]>::len(self))
    }

    fn iter(&self) -> impl Stream<Item = Self::Item> {
        iter(<[T]>::iter(self).cloned())
    }

    fn rev_iter(&self) -> impl Stream<Item = Self::Item> {
        iter(<[T]>::iter(self).rev().cloned())
    }
}

impl<T> LazyList for &[T]
where
    T: Clone,
{
    type Item = T;

    async fn get(&self, index: usize) -> Option<Self::Item> {
        <[T] as LazyList>::get(*self, index).await
    }

    fn len(&self) -> Option<usize> {
        <[T] as LazyList>::len(*self)
    }

    fn iter(&self) -> impl Stream<Item = Self::Item> {
        <[T] as LazyList>::iter(*self)
    }

    fn rev_iter(&self) -> impl Stream<Item = Self::Item> {
        <[T] as LazyList>::rev_iter(*self)
    }
}

impl<T> LazyList for &mut [T]
where
    T: Clone,
{
    type Item = T;

    async fn get(&self, index: usize) -> Option<Self::Item> {
        <[T] as LazyList>::get(*self, index).await
    }

    fn len(&self) -> Option<usize> {
        <[T] as LazyList>::len(*self)
    }

    fn iter(&self) -> impl Stream<Item = Self::Item> {
        <[T] as LazyList>::iter(*self)
    }

    fn rev_iter(&self) -> impl Stream<Item = Self::Item> {
        <[T] as LazyList>::rev_iter(*self)
    }
}

impl<T: Clone> LazyList for Vec<T> {
    type Item = T;

    async fn get(&self, index: usize) -> Option<Self::Item> {
        <[T] as LazyList>::get(self.deref(), index).await
    }

    fn len(&self) -> Option<usize> {
        <[T] as LazyList>::len(self.deref())
    }

    fn iter(&self) -> impl Stream<Item = Self::Item> {
        <[T] as LazyList>::iter(self.deref())
    }

    fn rev_iter(&self) -> impl Stream<Item = Self::Item> {
        <[T] as LazyList>::rev_iter(self.deref())
    }
}

impl<L: LazyList> LazyList for &L {
    type Item = L::Item;

    async fn get(&self, index: usize) -> Option<Self::Item> {
        LazyList::get(*self, index).await
    }

    fn len(&self) -> Option<usize> {
        LazyList::len(*self)
    }

    fn iter(&self) -> impl Stream<Item = Self::Item> {
        LazyList::iter(*self)
    }

    fn rev_iter(&self) -> impl Stream<Item = Self::Item> {
        LazyList::rev_iter(*self)
    }
}

impl<L: LazyList> LazyList for &mut L {
    type Item = L::Item;

    async fn get(&self, index: usize) -> Option<Self::Item> {
        LazyList::get(*self, index).await
    }

    fn len(&self) -> Option<usize> {
        LazyList::len(*self)
    }

    fn iter(&self) -> impl Stream<Item = Self::Item> {
        LazyList::iter(*self)
    }

    fn rev_iter(&self) -> impl Stream<Item = Self::Item> {
        LazyList::rev_iter(*self)
    }
}

impl<L: LazyList> LazyList for Box<L> {
    type Item = L::Item;

    async fn get(&self, index: usize) -> Option<Self::Item> {
        LazyList::get(self.deref(), index).await
    }

    fn len(&self) -> Option<usize> {
        LazyList::len(self.deref())
    }

    fn iter(&self) -> impl Stream<Item = Self::Item> {
        LazyList::iter(self.deref())
    }

    fn rev_iter(&self) -> impl Stream<Item = Self::Item> {
        LazyList::rev_iter(self.deref())
    }
}

impl<L: LazyList> LazyList for Rc<L> {
    type Item = L::Item;

    async fn get(&self, index: usize) -> Option<Self::Item> {
        LazyList::get(self.deref(), index).await
    }

    fn len(&self) -> Option<usize> {
        LazyList::len(self.deref())
    }

    fn iter(&self) -> impl Stream<Item = Self::Item> {
        LazyList::iter(self.deref())
    }

    fn rev_iter(&self) -> impl Stream<Item = Self::Item> {
        LazyList::rev_iter(self.deref())
    }
}
