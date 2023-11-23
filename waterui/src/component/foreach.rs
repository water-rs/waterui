use crate::{
    binding::BoxSubscriber,
    view::{Alignment, BoxView},
    Binding, View,
};

use super::{stack::DisplayMode, Stack};
pub struct ForEach {
    builder: Box<dyn ViewBuilder>,
    mode: DisplayMode,
    alignment: Alignment,
}

struct IntoView<F, Iter: 'static> {
    f: F,
    iter: Binding<Iter>,
}

trait ViewBuilder: 'static {
    fn build(&mut self) -> Vec<BoxView>;
    fn subscribe(&self, subscriber: BoxSubscriber);
}

impl<F, Iter, V> ViewBuilder for IntoView<F, Iter>
where
    F: 'static + Fn(<&Iter as IntoIterator>::Item) -> V,
    for<'a> &'a Iter: IntoIterator,
    V: View,
{
    fn build(&mut self) -> Vec<BoxView> {
        let mut vec = Vec::new();
        for item in self.iter.get().into_iter() {
            let view: BoxView = Box::new((self.f)(item));
            vec.push(view);
        }
        vec
    }

    fn subscribe(&self, subscriber: BoxSubscriber) {
        self.iter.add_boxed_subscriber(subscriber);
    }
}

impl ViewBuilder for Option<Vec<BoxView>> {
    fn build(&mut self) -> Vec<BoxView> {
        self.take().unwrap()
    }

    fn subscribe(&self, _subscriber: BoxSubscriber) {}
}

impl ForEach {
    pub fn new<Iter, V, F>(iter: Iter, f: F) -> Self
    where
        Iter: IntoIterator + 'static,
        V: View,
        F: Fn(Iter::Item) -> V,
    {
        let mut content = Vec::new();
        for item in iter.into_iter() {
            let view: BoxView = Box::new((f)(item));
            content.push(view);
        }
        Self {
            builder: Box::new(Some(content)),
            mode: DisplayMode::Vertical,
            alignment: Alignment::Default,
        }
    }

    pub fn binding<Iter: 'static, V: View>(
        iter: Binding<Iter>,
        f: impl 'static + Fn(<&Iter as IntoIterator>::Item) -> V,
    ) -> Self
    where
        for<'a> &'a Iter: IntoIterator,
    {
        let iter: Binding<Iter> = iter.into();
        Self {
            builder: Box::new(IntoView { iter, f }),
            mode: DisplayMode::Vertical,
            alignment: Alignment::Default,
        }
    }

    pub fn vertical(mut self) -> Self {
        self.mode = DisplayMode::Vertical;
        self
    }

    pub fn horizontal(mut self) -> Self {
        self.mode = DisplayMode::Horizontal;
        self
    }

    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }
}

impl View for ForEach {
    fn view(&mut self) -> Box<dyn View> {
        Box::new(Stack::new(self.builder.build()).mode(self.mode.clone()))
    }

    fn subscribe(&self, subscriber: fn() -> BoxSubscriber) {
        self.builder.subscribe(subscriber());
    }
}
