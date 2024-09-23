use core::num::NonZeroUsize;

use alloc::{boxed::Box, vec::Vec};
use waterui_core::{AnyView, Environment};
use waterui_reactive::{compute::ToComputed, Binding, Compute, ComputeExt, Computed};

use crate::{navigation::NavigationView, utils::Mapping, view::TaggedView};

pub struct Tab<T> {
    pub label: TaggedView<T, AnyView>,
    pub content: Box<dyn Fn(Environment) -> NavigationView>,
}

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

type Id = NonZeroUsize;

#[derive(Debug)]
pub struct TabsConfig {
    pub selection: Binding<Id>,
    pub tabs: Computed<Vec<Tab<Id>>>,
}

configurable!(Tabs, TabsConfig);

impl Tabs {
    pub fn new<T: Ord + Clone + 'static>(tabs: impl ToComputed<Vec<Tab<T>>>) -> Self {
        let tabs = tabs.to_compute();
        let tab = tabs
            .compute()
            .first()
            .expect("Tabs can not be empty")
            .label
            .tag
            .clone();
        let selection = Binding::container(tab);
        Self::with_selection(&selection, tabs)
    }

    pub fn with_selection<T: Ord + Clone + 'static>(
        selection: &Binding<T>,
        tabs: impl ToComputed<Vec<Tab<T>>>,
    ) -> Self {
        let mapping = Mapping::new();
        let selection = mapping.binding(selection.clone());

        let tabs = tabs
            .to_compute()
            .map(move |value| {
                value
                    .into_iter()
                    .map(|value| Tab {
                        label: value.label.map(|value| mapping.to_id(value)),
                        content: value.content,
                    })
                    .collect::<Vec<_>>()
            })
            .computed();

        Self(TabsConfig { selection, tabs })
    }
}
