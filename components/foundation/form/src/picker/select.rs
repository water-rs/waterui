use alloc::vec::Vec;
use nami::SignalExt;
use nami::signal::IntoComputed;
use nami::{Binding, Computed};
use waterui_controls::label::{ControlLabel, Label, LabelKind};
use waterui_controls::{ControlSize, IntoLabel, impl_label_style_methods};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::layout::StretchAxis;
use waterui_core::plugin::Plugin;
use waterui_core::{AnyView, Environment, Native, NativeView, View};

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
    /// Displays all options vertically with radio button indicators on every platform.
    Radio,
    /// A segmented button style picker.
    /// Displays all options horizontally as mutually exclusive segments.
    Segmented,
    /// A wheel style picker.
    /// Displays options in a scrollable vertical drum with a centered selection band.
    Wheel,
}

/// A picker style installed on a subtree becomes the default for pickers
/// that did not specify an explicit `.style(...)`.
impl Plugin for PickerStyle {}

#[non_exhaustive]
#[derive(Debug, Clone)]
/// Configuration for the `Picker` component.
pub struct PickerConfig<S> {
    /// The label displayed for the picker.
    ///
    /// Always present: it is required at construction so assistive technology
    /// has a name to announce, even when
    /// [`LabelDisplayMode::Hidden`](waterui_controls::label::LabelDisplayMode::Hidden)
    /// removes the visible chrome.
    label: ControlLabel<S>,
    /// The items to display in the picker.
    items: Computed<Vec<PickerItem<Id>>>,
    /// The binding to the currently selected item.
    selection: Binding<Id>,
    /// The visual style of the picker.
    style: PickerStyle,
    /// How large the picker is drawn.
    size: ControlSize,
}

impl<S> PickerConfig<S> {
    /// The label displayed for the picker.
    #[must_use]
    pub const fn label(&self) -> &ControlLabel<S> {
        &self.label
    }

    /// Sets the `label`.
    pub fn set_label(&mut self, label: ControlLabel<S>) {
        self.label = label;
    }

    /// The label displayed for the control, for editing it in place.
    pub const fn label_mut(&mut self) -> &mut ControlLabel<S> {
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

    /// How large the picker is drawn.
    #[must_use]
    pub const fn size(&self) -> ControlSize {
        self.size
    }

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

impl<S: 'static> NativeView for PickerConfig<S> {
    fn stretch_axis(&self) -> StretchAxis {
        self.layout_stretch_axis()
    }
}

impl<S: 'static> MapSlots<S> for PickerConfig<S> {
    type With<T: 'static> = PickerConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> PickerConfig<T> {
        PickerConfig {
            label: self.label.map_slots(f),
            items: self.items,
            selection: self.selection,
            style: self.style,
            size: self.size,
        }
    }
}

/// A picker item that associates a value of type `T` with a text display.
pub type PickerItem<T> = TaggedView<T, Text>;

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
/// Every other style (automatic, menu, radio, wheel) answers the platform
/// control's intrinsic size and never stretches to fill extra space
/// (`docs/layout-spec.md` §6).
#[derive(Debug, Clone)]
#[must_use]
pub struct Picker<L> {
    label: Label<L>,
    items: Computed<Vec<PickerItem<Id>>>,
    selection: Binding<Id>,
    style: Option<PickerStyle>,
    size: ControlSize,
}

impl<L> Picker<L> {
    /// Creates a new `Picker` with the given label, items, and selection
    /// binding.
    ///
    /// The label is mandatory — see
    /// [the label module](mod@waterui_controls::label). Chain [`Self::hide_label`]
    /// when the surrounding chrome already names the picker; the label stays in
    /// the accessibility tree.
    pub fn new<T: Ord + Clone + 'static, I: IntoLabel<Kind = L>>(
        label: I,
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

        Self {
            label: label.into_label(),
            items,
            selection: mapping.binding(selection),
            style: None,
            size: ControlSize::Small,
        }
    }

    pub(crate) const fn label_mut(&mut self) -> &mut Label<L> {
        &mut self.label
    }

    /// Sets the visual style of the picker.
    pub const fn style(mut self, style: PickerStyle) -> Self {
        self.style = Some(style);
        self
    }

    /// Sets how large the picker is drawn. A picker is
    /// [`ControlSize::Small`], like a
    /// [`Button`](waterui_controls::button::Button), when none is chosen; a
    /// theme decides what each size changes.
    pub const fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Changes the picker to dropdown menu style.
    pub const fn menu(self) -> Self {
        self.style(PickerStyle::Menu)
    }

    /// Changes the picker to radio button group style.
    pub const fn radio(self) -> Self {
        self.style(PickerStyle::Radio)
    }

    /// Changes the picker to segmented button style.
    pub const fn segmented(self) -> Self {
        self.style(PickerStyle::Segmented)
    }

    /// Changes the picker to wheel drum style.
    pub const fn wheel(self) -> Self {
        self.style(PickerStyle::Wheel)
    }
}

impl<L: LabelKind> Picker<L> {
    fn into_config<S: ChildSlot>(self, env: &Environment) -> PickerConfig<S> {
        let style = self.style.unwrap_or_else(|| {
            env.get::<PickerStyle>()
                .copied()
                .unwrap_or(PickerStyle::Automatic)
        });
        PickerConfig {
            label: self.label.into_control_label::<S>(env),
            items: self.items,
            selection: self.selection,
            style,
            size: self.size,
        }
        .resolve(env)
    }
}

impl<L: LabelKind> Picker<L> {
    /// Returns the configuration for this picker.
    #[must_use]
    pub fn config(self) -> PickerConfig<Label<L>> {
        PickerConfig {
            label: self.label.into_unscoped_control_label(),
            items: self.items,
            selection: self.selection,
            style: self.style.unwrap_or(PickerStyle::Automatic),
            size: self.size,
        }
    }
}

impl<S: View> From<PickerConfig<S>> for Picker<ControlLabel<S>> {
    fn from(value: PickerConfig<S>) -> Self {
        let mode = value.label.display_mode_preference();
        Self {
            label: Label::from_control_label(value.label, mode),
            items: value.items,
            selection: value.selection,
            style: Some(value.style),
            size: value.size,
        }
    }
}

impl<L: LabelKind> View for Picker<L> {
    fn body(self, env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>(env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(env), env, cx)
    }
}

impl_label_style_methods!(Picker<L>);

/// Creates a new `Picker` with the given label, items, and selection binding.
/// See [`Picker`] for more details.
pub fn picker<T: Ord + Clone + 'static, L: IntoLabel>(
    label: L,
    items: impl IntoComputed<Vec<PickerItem<T>>>,
    selection: &Binding<T>,
) -> Picker<L::Kind> {
    Picker::new(label, items, selection)
}

#[cfg(test)]
mod tests {
    use nami::Binding;
    use waterui_core::layout::StretchAxis;
    use waterui_core::plugin::Plugin;
    use waterui_core::{AnyView, Environment, NativeView};
    use waterui_text::Text;

    use super::{Picker, PickerStyle, TaggedView};

    /// `docs/layout-spec.md` §3: only the segmented style is `Horizontal` on
    /// the payload a backend receives.
    #[test]
    fn only_the_segmented_style_stretches() {
        let selection = Binding::new(0_i32);
        let cases = [
            (PickerStyle::Automatic, StretchAxis::None),
            (PickerStyle::Menu, StretchAxis::None),
            (PickerStyle::Radio, StretchAxis::None),
            (PickerStyle::Segmented, StretchAxis::Horizontal),
            (PickerStyle::Wheel, StretchAxis::None),
        ];
        for (style, expected) in cases {
            let picker = Picker::new(
                "Mode",
                alloc::vec![TaggedView::new(0, Text::new("One"))],
                &selection,
            )
            .style(style);
            assert_eq!(
                NativeView::stretch_axis(&picker.config()),
                expected,
                "{style:?} (payload)"
            );
        }
    }

    #[test]
    fn style_shorthands_select_their_variant() {
        let selection = Binding::new(0_i32);
        let items = || alloc::vec![TaggedView::new(0, Text::new("One"))];
        assert_eq!(
            Picker::new("Mode", items(), &selection)
                .menu()
                .config()
                .style(),
            PickerStyle::Menu
        );
        assert_eq!(
            Picker::new("Mode", items(), &selection)
                .radio()
                .config()
                .style(),
            PickerStyle::Radio
        );
        assert_eq!(
            Picker::new("Mode", items(), &selection)
                .segmented()
                .config()
                .style(),
            PickerStyle::Segmented
        );
        assert_eq!(
            Picker::new("Mode", items(), &selection)
                .wheel()
                .config()
                .style(),
            PickerStyle::Wheel
        );
    }

    #[test]
    fn ambient_picker_style_applies_unless_overridden() {
        let selection = Binding::new(0_i32);
        let items = || alloc::vec![TaggedView::new(0, Text::new("One"))];
        let mut env = Environment::new();
        PickerStyle::Wheel.install(&mut env);

        let inherited = Picker::new("Mode", items(), &selection).into_config::<AnyView>(&env);
        assert_eq!(inherited.style(), PickerStyle::Wheel);

        let overridden = Picker::new("Mode", items(), &selection)
            .radio()
            .into_config::<AnyView>(&env);
        assert_eq!(overridden.style(), PickerStyle::Radio);

        let explicit_auto = Picker::new("Mode", items(), &selection)
            .style(PickerStyle::Automatic)
            .into_config::<AnyView>(&env);
        assert_eq!(explicit_auto.style(), PickerStyle::Automatic);
    }
}
