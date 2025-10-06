pub mod color;
pub use color::ColorPicker;
pub mod date;
pub use date::DatePicker;
pub mod multi_date;

use alloc::vec::Vec;
use nami::SignalExt;
use nami::signal::IntoComputed;
use nami::{Binding, Computed};
use waterui_core::configurable;

use waterui_core::id::{Id, Mapping, TaggedView};

use waterui_text::Text;

#[non_exhaustive]
#[derive(Debug)]
/// Configuration for the `Picker` component.
pub struct PickerConfig {
    /// The items to display in the picker.
    pub items: Computed<Vec<PickerItem<Id>>>,
    /// The binding to the currently selected item.
    pub selection: Binding<Id>,
}

configurable!(
    #[doc = "A generic selection picker that binds choices to a reactive identifier."]
    Picker,
    PickerConfig
);

/// A picker item that associates a value of type `T` with a text display.
pub type PickerItem<T> = TaggedView<T, Text>;

impl Picker {
    /// Creates a new `Picker` with the given items and selection binding.
    pub fn new<T: Ord + Clone + 'static>(
        items: impl IntoComputed<Vec<PickerItem<T>>>,
        selection: &Binding<T>,
    ) -> Self {
        let mapping: Mapping<T> = Mapping::new();
        let items = items.into_signal();
        let items = {
            let mapping = mapping.clone();
            items
                .map(move |items| {
                    items
                        .into_iter()
                        .map(|item| item.mapping(&mapping))
                        .collect::<Vec<_>>()
                })
                .computed()
        };

        Self(PickerConfig {
            items,
            selection: mapping.binding(selection),
        })
    }
}

/// Creates a new `Picker` with the given items and selection binding.
/// See [`Picker`] for more details.
pub fn picker<T: Ord + Clone + 'static>(
    items: impl IntoComputed<Vec<PickerItem<T>>>,
    selection: &Binding<T>,
) -> Picker {
    Picker::new(items, selection)
}
