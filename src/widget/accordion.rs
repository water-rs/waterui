//! An accordion is a vertically stacked set of collapsible items.

use crate::{ViewExt, ext::SignalExt, prelude::*, widget::condition::when};
use alloc::vec::Vec;
use core::hash::Hash;
use waterui_core::{id::Identifable, view::IntoView};
use waterui_layout::stack::{VStack, vstack};

/// Represents a single item to be displayed in an `Accordion`.
#[derive(Debug, Clone)]
pub struct AccordionItem<H, C, ID> {
    /// The unique identifier for this item.
    pub id: ID,
    /// The view to display as the item's header. Always visible.
    pub header: H,
    /// The view to display as the item's content. Visible only when the item is expanded.
    pub content: C,
}

impl<H, C, ID: Clone + Hash + Ord> Identifable for AccordionItem<H, C, ID> {
    type Id = ID;
    fn id(&self) -> Self::Id {
        self.id.clone()
    }
}

/// A widget that displays a list of vertically-stacked, collapsible items.
#[derive(Debug)]
pub struct Accordion<H, C, ID>
where
    H: IntoView + Clone + 'static,
    C: IntoView + Clone + 'static,
    ID: Clone + Hash + Eq + Ord + 'static,
{
    items: Vec<AccordionItem<H, C, ID>>,
    selection: Binding<Option<ID>>,
}

impl<H, C, ID> Accordion<H, C, ID>
where
    H: IntoView + Clone + 'static,
    C: IntoView + Clone + 'static,
    ID: Clone + Hash + Eq + Ord + 'static,
{
    /// Creates a new Accordion with the given items and selection binding.
    pub fn new(items: Vec<AccordionItem<H, C, ID>>, selection: Binding<Option<ID>>) -> Self {
        Self { items, selection }
    }
}

impl<H, C, ID> View for Accordion<H, C, ID>
where
    H: IntoView + Clone + 'static,
    C: IntoView + Clone + 'static,
    ID: Clone + Hash + Eq + Ord + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let selection = self.selection;

        VStack::from_iter(self.items.into_iter().map(move |item| {
            let env = _env.clone();
            let header = item.header.clone().into_view(&env);
            let id = item.id();
            let selection_for_action = selection.clone();

            let header_button = button(header).action({
                let id = id.clone();
                move || {
                    if selection_for_action.get().as_ref() == Some(&id) {
                        selection_for_action.set(None);
                    } else {
                        selection_for_action.set(Some(id.clone()));
                    }
                }
            });

            let is_open = selection.clone().map(move |s| s.as_ref() == Some(&id));

            let content_source = item.content.clone();
            let content_closure = move || content_source.clone().into_view(&env);

            vstack((header_button, when(is_open, content_closure))).anyview()
        }))
    }
}

/// Convenience function to create a new `Accordion`.
pub fn accordion<H, C, ID>(
    items: Vec<AccordionItem<H, C, ID>>,
    selection: Binding<Option<ID>>,
) -> Accordion<H, C, ID>
where
    H: IntoView + Clone + 'static,
    C: IntoView + Clone + 'static,
    ID: Clone + Hash + Eq + Ord + 'static,
{
    Accordion::new(items, selection)
}
