use crate::view::ViewExt;
use crate::{AnyView, Environment, View};
use alloc::{boxed::Box, collections::BTreeMap, vec::Vec};
use waterui_reactive::{Binding, Reactive};

pub struct Each<T, Content> {
    data: Binding<Vec<T>>,
    content: Content,
}

impl<T: Ord + Clone, Content> Each<T, Content> {
    pub fn new(data: &Binding<Vec<T>>, content: Content) -> Self {
        Self {
            data: data.clone(),
            content,
        }
    }
}

impl<T, Content, V> View for Each<T, Content>
where
    T: Ord + Clone + 'static,
    Content: 'static + Fn(&T) -> V,
    V: View + 'static,
{
    fn body(self, _env: Environment) -> impl View {
        let raw: RawEach = Box::new(EachWithCache {
            each: self,
            cache_counter: 0,
            cache: BTreeMap::new(),
        });
        raw
    }
}

struct EachWithCache<T, Content> {
    each: Each<T, Content>,
    cache_counter: usize,
    cache: BTreeMap<T, usize>,
}

#[allow(clippy::len_without_is_empty)]
pub trait PullView: Reactive {
    fn ready(&mut self, index: usize) -> usize; // return id
    fn pull(&self, index: usize) -> AnyView;
    fn len(&self) -> usize;
}

impl<T, Content> Reactive for EachWithCache<T, Content> {
    fn register_subscriber(
        &self,
        subscriber: waterui_reactive::subscriber::BoxSubscriber,
    ) -> Option<core::num::NonZeroUsize> {
        self.each.data.register_subscriber(subscriber)
    }
    fn cancel_subscriber(&self, id: core::num::NonZeroUsize) {
        self.each.data.cancel_subscriber(id);
    }
    fn notify(&self) {
        self.each.data.notify();
    }
}

impl<T, Content, V> PullView for EachWithCache<T, Content>
where
    T: Ord + Clone,
    Content: Fn(&T) -> V,
    V: View + 'static,
{
    fn ready(&mut self, index: usize) -> usize {
        let data = &self.each.data.get()[index];
        self.cache.get(data).cloned().unwrap_or_else(|| {
            let id = self.cache_counter;
            self.cache_counter += 1;
            id
        })
    }

    fn pull(&self, index: usize) -> AnyView {
        let data = &self.each.data.get()[index];
        (self.each.content)(data).anyview()
    }

    fn len(&self) -> usize {
        self.each.data.get().len()
    }
}

pub type RawEach = Box<dyn PullView>;
raw_view!(RawEach);
