use crate::LazyList;
use futures_lite::{Stream, StreamExt};
pub struct Filter<L, F> {
    list: L,
    f: F,
}

impl<L, F> Filter<L, F> {
    pub fn new(list: L, f: F) -> Self {
        Self { list, f }
    }
}

impl<L, F> LazyList for Filter<L, F>
where
    L: LazyList,
    F: Fn(&L::Item) -> bool,
{
    type Item = L::Item;

    async fn get(&self, index: usize) -> Option<Self::Item> {
        self.list.get(index).await.filter(|v| (self.f)(v))
    }

    fn len(&self) -> Option<usize> {
        None
    }

    fn iter(&self) -> impl Stream<Item = Self::Item> {
        self.list.iter().filter(|v| (self.f)(v))
    }

    fn rev_iter(&self) -> impl Stream<Item = Self::Item> {
        self.list.rev_iter().filter(|v| (self.f)(v))
    }
}
