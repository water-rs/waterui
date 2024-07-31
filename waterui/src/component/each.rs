use core::ops::Deref;
use core::ops::DerefMut;

use crate::View;
use alloc::{boxed::Box, collections::BTreeMap, vec::Vec};
use waterui_core::raw_view;
use waterui_core::AnyView;
use waterui_reactive::Binding;

pub trait EachImpl: 'static {
    fn id(&mut self, index: usize) -> usize; // return id
    fn pull(&mut self, index: usize) -> AnyView;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub struct Each(Box<dyn EachImpl>);

impl_debug!(Each);

impl Deref for Each {
    type Target = dyn EachImpl;
    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl DerefMut for Each {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.deref_mut()
    }
}

raw_view!(Each);

impl Each {
    pub fn new<T, Content, V>(data: &Binding<Vec<T>>, content: Content) -> Self
    where
        T: Ord + Clone + 'static,
        Content: 'static + Fn(&T) -> V,
        V: View,
    {
        Self::from_impl(EachInner {
            data: data.clone(),
            content,
            id_counter: 0,
            id_map: BTreeMap::new(),
        })
    }

    pub fn from_impl(each: impl EachImpl) -> Self {
        Self(Box::new(each))
    }
}

struct EachInner<T: 'static, Content> {
    data: Binding<Vec<T>>,
    content: Content,
    id_counter: usize,
    id_map: BTreeMap<T, usize>,
}

impl<T, Content> Reactive for EachInner<T, Content> {
    fn register_subscriber(
        &self,
        subscriber: waterui_reactive::watcher::Subscriber,
    ) -> Option<SubscriberId> {
        self.data.register_subscriber(subscriber)
    }
    fn cancel_subscriber(&self, id: SubscriberId) {
        self.data.cancel_subscriber(id);
    }
}

impl<T, Content, V> EachImpl for EachInner<T, Content>
where
    T: Ord + Clone + 'static,
    Content: 'static + Fn(&T) -> V,
    V: View,
{
    fn id(&mut self, index: usize) -> usize {
        let data = &self.data.get()[index];
        self.id_map.get(data).cloned().unwrap_or_else(|| {
            let id = self.id_counter;
            self.id_counter += 1;
            id
        })
    }

    fn pull(&mut self, index: usize) -> AnyView {
        let data = &self.data.get()[index];
        AnyView::new((self.content)(data))
    }

    fn len(&self) -> usize {
        self.data.get().len()
    }
}
