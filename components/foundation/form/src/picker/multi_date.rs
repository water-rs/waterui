//! Multi-date picker component.

use alloc::collections::BTreeSet;
use core::ops::RangeInclusive;

use jiff::civil::Date;
use nami::watcher::BoxWatcherGuard;
use nami::{Binding, Computed, Signal, SignalExt, signal::IntoComputed};
use waterui_controls::label::{ControlLabel, Label, LabelKind};
use waterui_controls::{IntoLabel, impl_label_style_methods};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::{AnyView, Environment, Metadata, Native, NativeView, Retain, View};
use waterui_locale::locale_binding;

use crate::calendar::{
    CalendarBody, calendar_rows, ensure_visible_month_in_range, initial_visible_month,
    map_visible_month_binding, multi_day_cell_content, sanitize_date_range,
};

/// Removes any selected dates outside `range` in place.
pub fn clamp_dates_to_range(value: &Binding<BTreeSet<Date>>, range: &RangeInclusive<Date>) {
    let range = sanitize_date_range(range.clone());
    let current = value.snapshot();
    if current.iter().any(|date| !range.contains(date)) {
        let clamped: BTreeSet<Date> = current
            .into_iter()
            .filter(|date| range.contains(date))
            .collect();
        value.set(clamped);
    }
}

/// Configuration for the `MultiDatePicker` component.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct MultiDatePickerConfig<S> {
    /// The label displayed for the picker.
    label: ControlLabel<S>,
    /// The selected dates.
    value: Binding<BTreeSet<Date>>,
    /// The valid date range.
    range: Computed<RangeInclusive<Date>>,
    /// Passively decorated dates.
    decorated: Computed<BTreeSet<Date>>,
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

    /// The selected dates.
    #[must_use]
    pub const fn value(&self) -> &Binding<BTreeSet<Date>> {
        &self.value
    }

    /// The valid date range.
    #[must_use]
    pub const fn range(&self) -> &Computed<RangeInclusive<Date>> {
        &self.range
    }

    /// Watches `range` and removes out-of-range dates and adjusts
    /// `visible_month` in place whenever the range updates, returning the
    /// subscription guard.
    pub fn watch_range_clamp(&self) -> BoxWatcherGuard {
        let value = self.value.clone();
        let visible_month = self.visible_month.clone();
        let apply = move |range: &RangeInclusive<Date>| {
            let range = sanitize_date_range(range.clone());
            clamp_dates_to_range(&value, &range);
            ensure_visible_month_in_range(
                &visible_month,
                value.snapshot().into_iter().next(),
                &range,
            );
        };
        apply(&self.range.snapshot());
        self.range.watch(move |ctx| apply(ctx.value()))
    }

    /// Passively decorated dates.
    #[must_use]
    pub const fn decorated(&self) -> &Computed<BTreeSet<Date>> {
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
    value: Binding<BTreeSet<Date>>,
    range: Computed<RangeInclusive<Date>>,
    decorated: Computed<BTreeSet<Date>>,
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
    /// Creates a new `MultiDatePicker` with the given semantic label and
    /// selected dates binding.
    ///
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    pub fn new<I: IntoLabel<Kind = L>>(label: I, date: &Binding<BTreeSet<Date>>) -> Self {
        let initial_range = Date::MIN..=Date::MAX;
        let visible_month = Binding::new(
            initial_visible_month(date.snapshot().into_iter().next(), &initial_range).first_day(),
        );
        Self {
            label: label.into_label(),
            value: date.clone(),
            range: Computed::constant(initial_range),
            decorated: Computed::constant(BTreeSet::new()),
            visible_month,
        }
    }

    pub(crate) const fn label_mut(&mut self) -> &mut Label<L> {
        &mut self.label
    }

    /// Sets the valid date range for the picker.
    ///
    /// # Contract violation
    ///
    /// A reversed range panics in debug builds and logs and clamps in release
    /// builds.
    pub fn range(mut self, range: impl IntoComputed<RangeInclusive<Date>>) -> Self {
        let mapped = range.into_computed().map(sanitize_date_range).computed();
        let initial = mapped.snapshot();
        clamp_dates_to_range(&self.value, &initial);
        ensure_visible_month_in_range(
            &self.visible_month,
            self.value.snapshot().into_iter().next(),
            &initial,
        );
        self.range = mapped;
        self
    }

    /// Binds external control over the currently displayed month.
    pub fn visible_month(mut self, month: &Binding<Date>) -> Self {
        let visible_month = month.clone();
        ensure_visible_month_in_range(
            &visible_month,
            self.value.snapshot().into_iter().next(),
            &self.range.snapshot(),
        );
        self.visible_month = visible_month;
        self
    }

    /// Marks calendar days with a passive decoration dot.
    pub fn decorated(mut self, decorated: impl IntoComputed<BTreeSet<Date>>) -> Self {
        self.decorated = decorated.into_computed();
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
pub struct MultiDatePickerGrid<S>(MultiDatePickerConfig<S>);

impl<S> MultiDatePickerGrid<S> {
    /// The grid for `config`.
    #[must_use]
    pub const fn new(config: MultiDatePickerConfig<S>) -> Self {
        Self(config)
    }

    /// Splits this grid into its resolved label and its composed calendar body.
    pub fn into_parts(self, env: &Environment) -> (ControlLabel<S>, impl View + use<S>) {
        let range_guard = self.0.watch_range_clamp();
        let MultiDatePickerConfig {
            label,
            value: selection,
            range,
            decorated,
            visible_month,
        } = self.0;
        let visible_month = map_visible_month_binding(&visible_month);
        let locale = locale_binding(env).computed();

        let cell_range = range.clone();
        let cell_selection = selection;
        let cell_decorated = decorated;
        let rows = calendar_rows(&visible_month, move |cell| {
            multi_day_cell_content(cell, &cell_range, cell_selection.clone(), &cell_decorated)
        });
        let calendar = Metadata::new(
            CalendarBody::new(locale, range, visible_month, rows),
            Retain::new(range_guard),
        );
        (label, calendar)
    }
}

impl<S: View> View for MultiDatePickerGrid<S> {
    fn body(self, env: &Environment) -> impl View {
        let (label, calendar) = self.into_parts(env);
        waterui_layout::stack::vstack((label, calendar)).spacing(10.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nami::Signal;

    #[test]
    fn owned_visible_month_defaults_to_first_selected_and_supports_opt_in() {
        let mut selected = BTreeSet::new();
        selected.insert(Date::new(2025, 4, 20).unwrap());
        let dates = Binding::new(selected);

        let default_cfg = MultiDatePicker::new("Available", &dates).config();
        assert_eq!(
            default_cfg.visible_month().snapshot(),
            Date::new(2025, 4, 1).unwrap()
        );

        let shown = Binding::new(Date::new(2025, 7, 1).unwrap());
        let opt_in_cfg = MultiDatePicker::new("Available", &dates)
            .visible_month(&shown)
            .range(Date::new(2025, 1, 1).unwrap()..=Date::new(2025, 6, 30).unwrap())
            .config();
        assert_eq!(
            opt_in_cfg.visible_month().snapshot(),
            Date::new(2025, 4, 1).unwrap()
        );
        assert_eq!(shown.snapshot(), Date::new(2025, 4, 1).unwrap());
    }
}
