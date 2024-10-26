use core::num::NonZeroI32;

use alloc::{boxed::Box, vec::Vec};
use waterui_core::{AnyView, Environment};
use waterui_reactive::Binding;

use super::NavigationView;
use crate::{utils::Mapping, view::TaggedView};
pub struct Tab<T> {
    pub label: TaggedView<T, AnyView>,
    pub content: Box<dyn Fn(Environment) -> NavigationView>,
}

impl_debug!(Tab<Id>);

impl<T> Tab<T> {
    pub fn new(
        label: TaggedView<T, AnyView>,
        content: impl 'static + Fn(Environment) -> NavigationView,
    ) -> Self {
        Self {
            label,
            content: Box::new(content),
        }
    }
}

type Id = NonZeroI32;

#[derive(Debug)]
#[non_exhaustive]
pub struct TabsConfig {
    pub selection: Binding<Id>,
    pub tabs: Vec<Tab<Id>>,
}

configurable!(Tabs, TabsConfig);

impl TabsConfig {
    pub fn new(selection: Binding<Id>, tabs: Vec<Tab<Id>>) -> Self {
        Self { selection, tabs }
    }
}

impl Tabs {
    pub fn new<T: Ord + Clone + 'static>(tabs: impl IntoIterator<Item = Tab<T>>) -> Self {
        let mut tabs = tabs.into_iter().peekable();
        let tab = tabs
            .peek()
            .expect("Tabs can not be empty")
            .label
            .tag
            .clone();
        let selection = Binding::container(tab);
        Self::with_selection(&selection, tabs)
    }

    pub fn with_selection<T: Ord + Clone + 'static>(
        selection: &Binding<T>,
        tabs: impl IntoIterator<Item = Tab<T>>,
    ) -> Self {
        let mapping = Mapping::new();
        let selection = mapping.binding(selection.clone());

        let tabs = tabs
            .into_iter()
            .map(|value| Tab {
                label: value.label.map(|value| mapping.to_id(value)),
                content: value.content,
            })
            .collect::<Vec<_>>();

        Self(TabsConfig { selection, tabs })
    }
}
