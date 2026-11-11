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
use waterui_core::{Environment, configurable, layout::StretchAxis};

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
    /// On iOS this renders as a menu button; on macOS as a popup button.
    #[default]
    Automatic,
    /// A dropdown menu style picker.
    /// Displays as a button that opens a menu when tapped.
    Menu,
    /// A radio button group style picker.
    /// Displays all options vertically with radio button indicators.
    ///
    /// Platforms without a radio group render their inline picker instead:
    /// iOS has no radio group in `UIKit` or `SwiftUI`, so the Apple backend
    /// renders this style as the inline wheel picker there, exactly as
    /// `SwiftUI`'s `.inline` style does in the same place, while macOS keeps
    /// its native radio buttons. The asymmetry is documented rather than
    /// faked with a self-drawn list.
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
    label: Label,
    /// The items to display in the picker.
    items: Computed<Vec<PickerItem<Id>>>,
    /// The binding to the currently selected item.
    selection: Binding<Id>,
    /// The visual style of the picker.
    style: PickerStyle,
}

impl PickerConfig {
    /// The label displayed for the picker.
    ///
    /// Always present: it is required at construction so assistive technology
    /// has a name to announce, even when
    /// [`LabelDisplayMode::Hidden`](waterui_controls::label::LabelDisplayMode::Hidden)
    /// removes the visible chrome.
    #[must_use]
    pub const fn label(&self) -> &Label {
        &self.label
    }

    /// Sets the `label`.
    pub fn set_label(&mut self, label: Label) {
        self.label = label;
    }

    /// The label displayed for the control, for editing it in place.
    pub const fn label_mut(&mut self) -> &mut Label {
        &mut self.label
    }

    /// The items to display in the picker.
    #[must_use]
    pub const fn items(&self) -> &Computed<Vec<PickerItem<Id>>> {
        &self.items
    }

    /// The binding to the currently selected item.
    #[must_use]
    pub const fn selection(&self) -> &Binding<Id> {
        &self.selection
    }

    /// The visual style of the picker.
    #[must_use]
    pub const fn style(&self) -> PickerStyle {
        self.style
    }
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
    /// A segmented picker takes the width it is offered: it answers the
    /// proposed width and its intrinsic height, dividing the width into equal
    /// segments, and answers its intrinsic width when the width is unbounded.
    /// Every other style (automatic, menu, radio) answers the platform
    /// control's intrinsic size and never stretches to fill extra space
    /// (`docs/layout-spec.md` §6).
    //
    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: Layout Contract for Backend Implementers
    // ═══════════════════════════════════════════════════════════════════════════
    //
    // - Segmented: stretchAxis `.horizontal`; sizeThatFits answers the
    //   proposed width (never below the row's intrinsic width) and the
    //   intrinsic height; the row is divided into equal segments.
    // - Every other style: stretchAxis `.none`; sizeThatFits answers the
    //   control's intrinsic size.
    // - Read the axis from `NativeView::stretch_axis` on the payload; do not
    //   restate the rule in the backend.
    //
    // ═══════════════════════════════════════════════════════════════════════════
    //
    Picker,
    PickerConfig,
    |config| config.layout_stretch_axis(),
    resolve | config,
    env | config.resolve(env)
);

/// A picker item that associates a value of type `T` with a text display.
pub type PickerItem<T> = TaggedView<T, Text>;

impl PickerConfig {
    /// `docs/layout-spec.md` §6: a picker is `Horizontal` in the segmented
    /// style and `None` in every other.
    const fn layout_stretch_axis(&self) -> StretchAxis {
        match self.style {
            PickerStyle::Segmented => StretchAxis::Horizontal,
            _ => StretchAxis::None,
        }
    }

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
    /// [the label module](mod@waterui_controls::label). Chain [`Self::hide_label`]
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

#[cfg(test)]
mod tests {
    use nami::Binding;
    use waterui_core::layout::StretchAxis;
    use waterui_core::{NativeView, View};
    use waterui_text::Text;

    use super::{Picker, PickerStyle, TaggedView};

    /// `docs/layout-spec.md` §3: only the segmented style is `Horizontal`,
    /// both as the static answer a container reads before `body` and as the
    /// payload a backend receives.
    #[test]
    fn only_the_segmented_style_stretches() {
        let selection = Binding::i32(0);
        let cases = [
            (PickerStyle::Automatic, StretchAxis::None),
            (PickerStyle::Menu, StretchAxis::None),
            (PickerStyle::Radio, StretchAxis::None),
            (PickerStyle::Segmented, StretchAxis::Horizontal),
        ];
        for (style, expected) in cases {
            let picker = Picker::new(
                "Mode",
                alloc::vec![TaggedView::new(0, Text::new("One"))],
                &selection,
            )
            .style(style);
            assert_eq!(View::stretch_axis(&picker), expected, "{style:?} (static)");
            assert_eq!(
                NativeView::stretch_axis(&picker.0),
                expected,
                "{style:?} (payload)"
            );
        }
    }
}
