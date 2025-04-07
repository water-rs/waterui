pub mod color;
pub mod date;
pub mod multi_date;
use core::num::NonZeroI32;

use crate::ComputeExt;
use alloc::vec::Vec;
use waterui_reactive::compute::IntoComputed;
use waterui_reactive::{Binding, Computed};

use crate::id::TaggedView;
use crate::utils::Mapping;

use super::Text;

pub type ItemId = NonZeroI32;

#[non_exhaustive]
#[derive(Debug)]
pub struct PickerConfig {
    pub items: Computed<Vec<PickerItem<ItemId>>>,
    pub selection: Binding<ItemId>,
}

configurable!(Picker, PickerConfig);

pub type PickerItem<T> = TaggedView<T, Text>;

impl Picker {
    pub fn new<T: Ord + Clone + 'static>(
        items: impl IntoComputed<Vec<PickerItem<T>>>,
        selection: &Binding<T>,
    ) -> Self {
        let mapping: Mapping<T> = Mapping::new();
        let items = {
            let mapping = mapping.clone();

            items.into_compute().map(move |items| {
                items
                    .into_iter()
                    .map(|value| value.map(|value| mapping.to_id(value)))
                    .collect()
            })
        }
        .computed();

        let selection = mapping.binding(selection.clone());

        Self(PickerConfig { items, selection })
    }
}

pub fn picker<T: Ord + Clone + 'static>(
    items: impl IntoComputed<Vec<PickerItem<T>>>,
    selection: &Binding<T>,
) -> Picker {
    Picker::new(items, selection)
}
