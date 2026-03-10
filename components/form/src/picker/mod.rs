pub mod color;
pub use color::ColorPicker;
pub mod date;
pub use date::DatePicker;
pub mod file;
pub mod multi_date;

use alloc::vec::Vec;
use nami::SignalExt;
use nami::signal::IntoComputed;
use nami::{Binding, Computed};
use waterui_core::configurable;
use waterui_core::{AnyView, View};

use waterui_core::id::{Id, Mapping, TaggedView};

use waterui_text::Text;

/// Visual style options for pickers.
///
/// Different picker styles provide different visual presentation for selecting from options.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PickerStyle {
    /// The default picker style, determined by the platform and context.
    /// On iOS, this typically renders as a segmented control.
    /// On macOS, this typically renders as a popup button.
    #[default]
    Automatic,
    /// A dropdown menu style picker.
    /// Displays as a button that opens a menu when tapped.
    Menu,
    /// A radio button group style picker.
    /// Displays all options vertically with radio button indicators.
    Radio,
}

#[non_exhaustive]
#[derive(Debug)]
/// Configuration for the `Picker` component.
pub struct PickerConfig {
    /// The label to display for the picker.
    pub label: AnyView,
    /// The items to display in the picker.
    pub items: Computed<Vec<PickerItem<Id>>>,
    /// The binding to the currently selected item.
    pub selection: Binding<Id>,
    /// The visual style of the picker.
    pub style: PickerStyle,
}

configurable!(
    /// A control for selecting from a list of options.
    ///
    /// Picker displays a selection UI (menu, wheel, or segmented style depending on context).
    ///
    /// # Layout Behavior
    ///
    /// Picker sizes itself to fit its content and never stretches to fill extra space.
    /// In a stack, it takes only the space it needs.
    //
    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: Layout Contract for Backend Implementers
    // ═══════════════════════════════════════════════════════════════════════════
    //

    // Size: Determined by content and picker style (platform-determined)
    //
    // Note: Segmented picker style may use `Horizontal` stretch axis.
    //
    // ═══════════════════════════════════════════════════════════════════════════
    //
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
            label: AnyView::default(),
            items,
            selection: mapping.binding(selection),
            style: PickerStyle::default(),
        })
    }

    /// Sets the label for the picker.
    #[must_use]
    pub fn label(mut self, label: impl View) -> Self {
        self.0.label = AnyView::new(label);
        self
    }

    /// Sets the visual style of the picker.
    ///
    /// # Arguments
    ///
    /// * `style` - The picker style to apply
    ///
    /// # Returns
    ///
    /// The modified picker with the style set
    #[must_use]
    pub const fn style(mut self, style: PickerStyle) -> Self {
        self.0.style = style;
        self
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
