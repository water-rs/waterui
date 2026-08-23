pub mod color;
pub use color::ColorPicker;
pub mod date;
pub use date::{DatePicker, DatePickerType};
pub mod file;
pub mod multi_date;
pub use multi_date::{MultiDatePicker, MultiDatePickerConfig};

use alloc::vec::Vec;
use nami::SignalExt;
use nami::signal::IntoComputed;
use nami::{Binding, Computed};
use waterui_controls::label::Label;
use waterui_controls::{IntoLabel, impl_label_style_methods};
use waterui_core::{Environment, configurable};

use waterui_core::id::{Id, Mapping, TaggedView};
use waterui_locale::locale_binding;

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
    /// A segmented button style picker.
    /// Displays all options horizontally as mutually exclusive segments.
    Segmented,
}

#[non_exhaustive]
#[derive(Debug)]
/// Configuration for the `Picker` component.
pub struct PickerConfig {
    /// The label displayed for the picker.
    ///
    /// Always present: it is required at construction so assistive technology
    /// has a name to announce, even when
    /// [`LabelDisplayMode::Hidden`](waterui_controls::label::LabelDisplayMode::Hidden)
    /// removes the visible chrome.
    pub label: Label,
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
    /// The label is required at construction; hide it visually with
    /// [`Picker::hide_label`] when the surrounding chrome already names the
    /// control.
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
    PickerConfig,
    resolve |config, env| config.resolve(env)
);

/// A picker item that associates a value of type `T` with a text display.
pub type PickerItem<T> = TaggedView<T, Text>;

impl PickerConfig {
    #[must_use]
    fn resolve(mut self, env: &Environment) -> Self {
        self.label = self.label.resolve(env);
        let env = env.clone();
        let locale = locale_binding(&env);
        self.items = self
            .items
            .zip(&locale)
            .map(move |(items, _locale)| {
                items
                    .into_iter()
                    .map(|item| TaggedView::new(item.tag, Text::from(item.content.resolve(&env))))
                    .collect()
            })
            .computed();
        self
    }
}

impl Picker {
    /// Creates a new `Picker` with the given label, items, and selection
    /// binding.
    ///
    /// The label is mandatory — see
    /// [the label module](waterui_controls::label). Chain [`Self::hide_label`]
    /// when the surrounding chrome already names the picker; the label stays in
    /// the accessibility tree.
    pub fn new<T: Ord + Clone + 'static>(
        label: impl IntoLabel,
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
                        .map(|item| item.map(|value| mapping.to_id(value)))
                        .collect::<Vec<_>>()
                })
                .computed()
        };

        Self(PickerConfig {
            label: label.into_label(),
            items,
            selection: mapping.binding(selection),
            style: PickerStyle::default(),
        })
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

    /// Changes the picker to segmented button style.
    #[must_use]
    pub const fn segmented(self) -> Self {
        self.style(PickerStyle::Segmented)
    }
}

impl_label_style_methods!(Picker);

/// Creates a new `Picker` with the given label, items, and selection binding.
/// See [`Picker`] for more details.
pub fn picker<T: Ord + Clone + 'static>(
    label: impl IntoLabel,
    items: impl IntoComputed<Vec<PickerItem<T>>>,
    selection: &Binding<T>,
) -> Picker {
    Picker::new(label, items, selection)
}
