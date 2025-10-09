//! An accordion is a vertically stacked set of collapsible items.

use crate::{ViewExt, ext::SignalExt, prelude::*, widget::condition::when};
use alloc::vec::Vec;
use waterui_core::view::IntoView;
use waterui_layout::stack::{VStack, vstack};

/// Represents a single item to be displayed in an `Accordion`.
///
/// Each item consists of a header view and a content view.
#[derive(Debug)]
pub struct AccordionItem<H: IntoView, C: IntoView> {
    /// The view to display as the item's header. Always visible.
    pub header: H,
    /// The view to display as the item's content. Visible only when the item is expanded.
    pub content: C,
}

/// A widget that displays a list of vertically-stacked, collapsible items.
///
/// Only one item can be expanded at a time. The state of which item is
/// currently selected is managed by the `selection` binding.
#[derive(Debug)]
pub struct Accordion<H: IntoView, C: IntoView> {
    items: Vec<AccordionItem<H, C>>,
    selection: Binding<Option<usize>>,
}

impl<H: IntoView, C: IntoView> Accordion<H, C> {
    /// Creates a new Accordion with the given items and selection binding.
    ///
    /// # Arguments
    ///
    /// * `items` - A vector of `AccordionItem`s to display.
    /// * `selection` - A `Binding` that holds the index of the currently selected item, or `None`.
    pub fn new(items: Vec<AccordionItem<H, C>>, selection: Binding<Option<usize>>) -> Self {
        Self { items, selection }
    }
}

impl<H, C> View for Accordion<H, C>
where
    // The `Clone` and `'static` bounds are necessary to move the views into the closure for rendering.
    H: IntoView + Clone + 'static,
    C: IntoView + Clone + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let items_with_indices = self.items.into_iter().enumerate();

        let selection = self.selection;

        // Use `VStack::from_iter` to build a stack from a dynamic collection of views.
        VStack::from_iter(items_with_indices.map(move |(index, item)| {
            let env = _env.clone();
            let header = item.header.clone().into_view(&env);
            let selection_for_action = selection.clone();

            // The header acts as a button to toggle the selection.
            let header_button = button(header).action(move || {
                if selection_for_action.get() == Some(index) {
                    // If clicking the currently open item, close it.
                    selection_for_action.set(None);
                } else {
                    // Otherwise, open the clicked item.
                    selection_for_action.set(Some(index));
                }
            });

            // The content is only rendered `when` this item is selected.
            let is_open = selection.clone().map(move |s| s == Some(index));

            // This closure captures the view "source" and environment, creating the view
            // on demand. This makes it an `Fn` closure, satisfying `when`'s requirement.
            let content_source = item.content.clone();
            let content_closure = move || content_source.clone().into_view(&env);

            vstack((header_button, when(is_open, content_closure))).anyview() // Erase the complex type for the iterator
        }))
    }
}

/// Convenience function to create a new `Accordion`.
pub fn accordion<H: IntoView, C: IntoView>(
    items: Vec<AccordionItem<H, C>>,
    selection: Binding<Option<usize>>,
) -> Accordion<H, C> {
    Accordion::new(items, selection)
}
