use crate::{Environment, View};
use alloc::{boxed::Box, collections::BTreeMap, vec::Vec};
use waterui_core::AnyView;
use waterui_reactive::{Binding, Reactive};

pub struct Each<T, Content> {
    data: Binding<Vec<T>>,
    content: Content,
}

trait EachImpl: Reactive {
    fn id(&mut self, index: usize) -> usize; // return id
    fn pull(&mut self, index: usize) -> AnyView;
    fn len(&self) -> usize;
}

pub struct RawEach(Box<dyn EachImpl>);

impl RawEach {
    pub fn id(&mut self, index: usize) -> usize {
        self.0.id(index)
    }
    pub fn pull(&mut self, index: usize) -> AnyView {
        self.0.pull(index)
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.len() == 0
    }
}

raw_view!(RawEach);

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
        let raw: Box<dyn EachImpl> = Box::new(EachInner {
            each: self,
            id_counter: 0,
            id_map: BTreeMap::new(),
        });
        RawEach(raw)
    }
}

struct EachInner<T, Content> {
    each: Each<T, Content>,
    id_counter: usize,
    id_map: BTreeMap<T, usize>,
}

impl<T, Content> Reactive for EachInner<T, Content> {
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

impl<T, Content, V> EachImpl for EachInner<T, Content>
where
    T: Ord + Clone,
    Content: Fn(&T) -> V,
    V: View + 'static,
{
    fn id(&mut self, index: usize) -> usize {
        let data = &self.each.data.get()[index];
        self.id_map.get(data).cloned().unwrap_or_else(|| {
            let id = self.id_counter;
            self.id_counter += 1;
            id
        })
    }

    fn pull(&mut self, index: usize) -> AnyView {
        let data = &self.each.data.get()[index];
        AnyView::new((self.each.content)(data))
    }

    fn len(&self) -> usize {
        self.each.data.get().len()
    }
}
