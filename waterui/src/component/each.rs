use crate::{utils::IdentifierMap, view::ViewExt};
use alloc::{boxed::Box, vec::Vec};
use waterui_reactive::Binding;
use waterui_view::{AnyView, View};

pub struct Each<T, Content> {
    data: Binding<Vec<T>>,
    content: Content,
    data_mapper: IdentifierMap<T>,
}

impl<T: Ord + Clone, Content> Each<T, Content> {
    pub fn new(data: &Binding<Vec<T>>, content: Content) -> Self {
        Self {
            data: data.clone(),
            content,
            data_mapper: IdentifierMap::new(),
        }
    }
}

impl<T, Content, V> View for Each<T, Content>
where
    T: Ord + Clone + 'static,
    Content: 'static + Fn(T) -> V,
    V: View + 'static,
{
    fn body(self, _env: waterui_view::Environment) -> impl View {
        RawEach {
            inner: Box::new(self),
        }
    }
}

raw_view!(RawEach);

trait PullView {
    fn len(&self) -> usize;
    fn data_id(&mut self, index: usize) -> Int;
    fn view(&self, data_id: Int) -> AnyView;
}

impl<T, Content, V> PullView for Each<T, Content>
where
    T: Ord + Clone,
    Content: Fn(T) -> V,
    V: View + 'static,
{
    fn len(&self) -> usize {
        self.data.get().len()
    }
    fn data_id(&mut self, index: usize) -> Int {
        self.data_mapper.id(self.data.get().get(index).cloned())
    }

    fn view(&self, id: Int) -> AnyView {
        let data = self.data_mapper.data(id).unwrap();
        (self.content)(data).anyview()
    }
}

pub struct RawEach {
    inner: Box<dyn PullView>,
}
