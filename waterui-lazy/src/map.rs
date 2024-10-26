use core::marker::PhantomData;

use crate::LazyList;

use futures_lite::{Stream, StreamExt};
pub struct Map<L, F, Output> {
    list: L,
    f: F,
    _marker: PhantomData<Output>,
}

impl<L, F, Output> Map<L, F, Output> {
    pub fn new(list: L, f: F) -> Self {
        Self {
            list,
            f,
            _marker: PhantomData,
        }
    }
}

impl<L, F, Output> LazyList for Map<L, F, Output>
where
    L: LazyList,
    F: Fn(L::Item) -> Output,
{
    type Item = Output;

    async fn get(&self, index: usize) -> Option<Self::Item> {
        self.list.get(index).await.map(|v| (self.f)(v))
    }

    fn len(&self) -> Option<usize> {
        self.list.len()
    }

    fn iter(&self) -> impl Stream<Item = Self::Item> {
        self.list.iter().map(|v| (self.f)(v))
    }

    fn rev_iter(&self) -> impl Stream<Item = Self::Item> {
        self.list.rev_iter().map(|v| (self.f)(v))
    }
}
