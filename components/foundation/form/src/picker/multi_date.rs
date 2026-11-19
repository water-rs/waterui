//! Multi-date picker component.

use alloc::{collections::BTreeSet, vec::Vec};
use core::ops::RangeInclusive;

use jiff::civil::Date;
use nami::{Binding, Computed, Signal, SignalExt, signal::IntoComputed};
use waterui_controls::label::{ControlLabel, Label, LabelKind};
use waterui_controls::{IntoLabel, impl_label_style_methods};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::{AnyView, Environment, Native, NativeView, View};
use waterui_locale::locale_binding;

use crate::calendar::{
    CalendarBody, VisibleMonth, calendar_rows, initial_visible_month, map_visible_month_binding,
    multi_day_cell_content, visible_month_in_range,
};

/// Configuration for the `MultiDatePicker` component.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MultiDatePickerConfig<S> {
    /// The label displayed for the picker.
    label: ControlLabel<S>,
    /// The selected dates as an ordered vector for backend/FFI transport.
    value: Binding<Vec<Date>>,
    /// The valid date range.
    range: RangeInclusive<Date>,
    /// Passively decorated dates.
    decorated: Computed<Vec<Date>>,
    /// The month the picker shows.
    visible_month: Binding<Date>,
}

impl<S> MultiDatePickerConfig<S> {
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

    /// The selected dates as an ordered vector for backend/FFI transport.
    #[must_use]
    pub const fn value(&self) -> &Binding<Vec<Date>> {
        &self.value
    }

    /// The valid date range.
    #[must_use]
    pub const fn range(&self) -> &RangeInclusive<Date> {
        &self.range
    }

    /// Passively decorated dates.
    #[must_use]
    pub const fn decorated(&self) -> &Computed<Vec<Date>> {
        &self.decorated
    }

    /// The month the picker shows: any day of it reads as that month, and
    /// the picker writes the month's first day when the user navigates.
    #[must_use]
    pub const fn visible_month(&self) -> &Binding<Date> {
        &self.visible_month
    }
}

/// A control for selecting multiple dates.
#[derive(Debug, Clone)]
#[must_use]
pub struct MultiDatePicker<L> {
    label: Label<L>,
    value: Binding<Vec<Date>>,
    range: RangeInclusive<Date>,
    decorated: Computed<Vec<Date>>,
    visible_month: Binding<Date>,
}

impl<L: LabelKind> MultiDatePicker<L> {
    /// Returns the configuration for this multi-date picker.
    #[must_use]
    pub fn config(self) -> MultiDatePickerConfig<Label<L>> {
        MultiDatePickerConfig {
            label: self.label.into_unscoped_control_label(),
            value: self.value,
            range: self.range,
            decorated: self.decorated,
            visible_month: self.visible_month,
        }
    }
}

impl<S: View> From<MultiDatePickerConfig<S>> for MultiDatePicker<ControlLabel<S>> {
    fn from(value: MultiDatePickerConfig<S>) -> Self {
        let mode = value.label.display_mode_preference();
        Self {
            label: Label::from_control_label(value.label, mode),
            value: value.value,
            range: value.range,
            decorated: value.decorated,
            visible_month: value.visible_month,
        }
    }
}

impl<S: 'static> NativeView for MultiDatePickerConfig<S> {
    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::None
    }
}

impl<S: 'static> MapSlots<S> for MultiDatePickerConfig<S> {
    type With<T: 'static> = MultiDatePickerConfig<T>;

    fn map_slots<T: 'static>(
        self,
        f: impl Fn(S) -> T + Clone + 'static,
    ) -> MultiDatePickerConfig<T> {
        MultiDatePickerConfig {
            label: self.label.map_slots(f),
            value: self.value,
            range: self.range,
            decorated: self.decorated,
            visible_month: self.visible_month,
        }
    }
}

impl<L> MultiDatePicker<L> {
    /// Creates a new `MultiDatePicker` with the given semantic label, selected
    /// dates binding, and visible month binding.
    ///
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    pub fn new<I: IntoLabel<Kind = L>>(
        label: I,
        date: &Binding<BTreeSet<Date>>,
        visible_month: &Binding<Date>,
    ) -> Self {
        let range = Date::MIN..=Date::MAX;
        Self {
            label: label.into_label(),
            value: map_multi_date_binding(date),
            range,
            decorated: Computed::constant(Vec::new()),
            visible_month: visible_month.clone(),
        }
    }

    pub(crate) const fn label_mut(&mut self) -> &mut Label<L> {
        &mut self.label
    }

    /// Sets the valid date range for the picker.
    pub fn range(mut self, range: RangeInclusive<Date>) -> Self {
        let visible = VisibleMonth::from_date(self.visible_month.snapshot());
        if !visible_month_in_range(visible, &range) {
            self.visible_month.set(
                initial_visible_month(self.value.snapshot().into_iter().next(), &range).first_day(),
            );
        }
        self.range = range;
        self
    }

    /// Marks calendar days with a passive decoration dot.
    pub fn decorated(mut self, decorated: impl IntoComputed<BTreeSet<Date>>) -> Self {
        self.decorated = decorated
            .into_computed()
            .map(|dates| dates.into_iter().collect())
            .computed();
        self
    }
}

impl_label_style_methods!(MultiDatePicker<L>);

impl<L: LabelKind> MultiDatePicker<L> {
    fn into_config<S: ChildSlot>(self, env: &Environment) -> MultiDatePickerConfig<S> {
        MultiDatePickerConfig {
            label: self.label.into_control_label::<S>(env),
            value: self.value,
            range: self.range,
            decorated: self.decorated,
            visible_month: self.visible_month,
        }
    }
}

impl<L: LabelKind> View for MultiDatePicker<L> {
    fn body(self, env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>(env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(env), env, cx)
    }
}

/// The multi-date picker composed from `WaterUI` views: how a self-drawn
/// backend realizes [`MultiDatePickerConfig`].
#[derive(Debug)]
pub struct MultiDatePickerGrid<S> {
    label: ControlLabel<S>,
    value: Binding<BTreeSet<Date>>,
    range: RangeInclusive<Date>,
    decorated: Computed<BTreeSet<Date>>,
    visible_month: Binding<VisibleMonth>,
}

impl<S> MultiDatePickerGrid<S> {
    /// The grid for `config`.
    #[must_use]
    pub fn new(config: MultiDatePickerConfig<S>, _env: &Environment) -> Self {
        let value = Binding::mapping(
            &config.value,
            |dates: Vec<Date>| dates.into_iter().collect::<BTreeSet<_>>(),
            |binding: &Binding<Vec<Date>>, dates: BTreeSet<Date>| {
                binding.set(dates.into_iter().collect());
            },
        );
        Self {
            label: config.label,
            value,
            range: config.range,
            decorated: config
                .decorated
                .map(|dates: Vec<Date>| dates.into_iter().collect::<BTreeSet<_>>())
                .computed(),
            visible_month: map_visible_month_binding(&config.visible_month),
        }
    }

    /// Splits this grid into its resolved label and its composed calendar body.
    pub fn into_parts(self, env: &Environment) -> (ControlLabel<S>, impl View + use<S>) {
        let label = self.label;
        let selection = self.value;
        let range = self.range;
        let decorated = self.decorated;
        let visible_month = self.visible_month;
        let locale = locale_binding(env).computed();

        let cell_range = range.clone();
        let cell_selection = selection;
        let cell_decorated = decorated;
        let rows = calendar_rows(&visible_month, move |cell| {
            multi_day_cell_content(cell, &cell_range, cell_selection.clone(), &cell_decorated)
        });
        let calendar = CalendarBody::new(locale, range, visible_month, rows);
        (label, calendar)
    }
}

impl<S: View> View for MultiDatePickerGrid<S> {
    fn body(self, env: &Environment) -> impl View {
        let (label, calendar) = self.into_parts(env);
        waterui_layout::stack::vstack((label, calendar)).spacing(10.0)
    }
}

fn map_multi_date_binding(dates: &Binding<BTreeSet<Date>>) -> Binding<Vec<Date>> {
    Binding::mapping(
        dates,
        |dates| dates.into_iter().collect(),
        |binding, dates: Vec<Date>| {
            binding.set(dates.into_iter().collect());
        },
    )
}
