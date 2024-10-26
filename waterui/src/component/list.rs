use crate::view::ViewExt;
use waterui_core::{AnyView, View};
use waterui_lazy::{AnyLazyList, LazyList, LazyListExt};

#[derive(Debug)]
#[must_use]
pub struct List {
    pub contents: AnyLazyList<AnyView>,
}

impl List {
    pub fn new<T, V: View>(
        data: impl 'static + LazyList<Item = T>,
        generator: impl 'static + Fn(T) -> V,
    ) -> Self {
        Self {
            contents: AnyLazyList::new(data.map(move |item| generator(item).anyview())),
        }
    }
}
