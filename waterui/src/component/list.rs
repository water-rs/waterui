use alloc::boxed::Box;

use crate::{
    component::views::{AnyViews, Views},
    view::ViewExt,
};
use waterui_core::{AnyView, Environment, View};
impl_compute_result!(AnyViews<ListItem>);

#[derive(Debug)]
pub struct ListConfig {
    pub contents: AnyViews<ListItem>,
}

configurable!(List, ListConfig);

impl List {
    pub fn new(contents: impl Views<Item = ListItem> + 'static) -> Self {
        Self(ListConfig {
            contents: AnyViews::new(contents),
        })
    }
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
