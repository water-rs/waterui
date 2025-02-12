use alloc::boxed::Box;
use waterui_reactive::collection::Collection;

use crate::{
    component::views::{AnyViews, ForEach, Views},
    view::ViewExt,
};
use waterui_core::{AnyView, Environment, View};
impl_compute_result!(AnyViews<ListItem>);

#[derive(Debug)]
pub struct ListConfig {
    pub contents: AnyViews<ListItem>,
}

#[derive(Debug)]
pub struct List<C, F, V>(ForEach<C, F, V, V>);

impl<C, F, V> List<C, F, V>
where
    C: Collection,
    F: Fn(C::Item) -> V,
    V: View,
{
    pub fn new(data: C, generator: F) -> Self {
        Self(ForEach::new(data, generator))
    }

    pub fn searchable() {}
}

pub struct ListItem {
    pub content: AnyView,
    pub on_delete: Option<OnDelete>,
}

type OnDelete = Box<dyn Fn(&Environment, usize)>;

impl_debug!(ListItem);

impl ListItem {
    pub fn on_delete(mut self, on_delete: impl Fn(&Environment, usize) + 'static) -> Self {
        self.on_delete = Some(Box::new(on_delete));
        self
    }

    pub fn disable_delete(mut self) -> Self {
        self.on_delete = None;
        self
    }
}

impl<V> From<V> for ListItem
where
    V: View,
{
    fn from(value: V) -> Self {
        Self {
            content: value.anyview(),
            on_delete: None,
        }
    }
}
