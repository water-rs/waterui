use crate::{
    reactive::{IntoRef, Ref},
    view::{Alignment, BoxView},
    View,
};

use super::{stack::DisplayMode, ReactiveView, Stack};
use std::{collections::HashMap, hash::Hash};
pub struct ForEach {
    reactive: Ref<()>,
    builder: Box<dyn ViewBuilder>,
    mode: DisplayMode,
    alignment: Alignment,
}

struct IntoView<F, Iter: 'static> {
    f: F,
    iter: Ref<Iter>,
}

trait ViewBuilder: 'static {
    fn build(&self) -> Vec<BoxView>;
}

impl<F, Iter, V> ViewBuilder for IntoView<F, Iter>
where
    F: 'static + Fn(Iter::Item) -> V,
    Iter: 'static + IntoIterator,
    V: View,
{
    fn build(&self) -> Vec<BoxView> {
        let mut vec = Vec::new();
        for item in self.iter.into_inner().into_iter() {
            let view: BoxView = Box::new((self.f)(item));
            vec.push(view);
        }
        vec
    }
}

impl ForEach {
    pub fn new<Iter: IntoIterator + 'static, V: View>(
        iter: impl IntoRef<Iter>,
        f: impl 'static + Fn(Iter::Item) -> V,
    ) -> Self
    where
        Iter::Item: Hash + Eq,
    {
        let reactive = Ref::new_with_updater(|| {});
        let iter: Ref<Iter> = iter.into_ref();
        iter.subcribe(reactive.clone());
        Self {
            reactive,
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
    fn view(&self) -> Box<dyn View> {
        Box::new(ReactiveView::new(
            self.reactive.clone(),
            Stack::new(self.builder.build(), self.mode.clone()),
        ))
    }
}
