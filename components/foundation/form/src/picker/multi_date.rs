//! Multi-date picker component.

use alloc::{collections::BTreeSet, vec::Vec};
use core::ops::RangeInclusive;

use jiff::civil::Date;
use nami::{Binding, Computed, SignalExt, signal::IntoComputed};
use waterui_controls::label::Label;
use waterui_controls::{IntoLabel, impl_label_style_methods};
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};
use waterui_core::{AnyView, Environment, View};

use crate::calendar::{
    CalendarBody, VisibleMonth, calendar_rows, initial_visible_month, map_visible_month_binding,
    multi_day_cell_content, resolve_locale, signal_driven_view, visible_month_in_range,
};

/// Configuration for the `MultiDatePicker` component.
#[derive(Debug)]
#[non_exhaustive]
pub struct MultiDatePickerConfig {
    /// The label displayed for the picker.
    pub label: Label,
    /// The selected dates as an ordered vector for backend/FFI transport.
    pub value: Binding<Vec<Date>>,
    /// The valid date range.
    pub range: RangeInclusive<Date>,
    /// Passively decorated dates.
    pub decorated: Computed<Vec<Date>>,
    visible_month: Binding<VisibleMonth>,
}

/// A control for selecting multiple dates.
#[derive(Debug)]
pub struct MultiDatePicker(MultiDatePickerConfig);

impl ConfigurableView for MultiDatePicker {
    type Config = MultiDatePickerConfig;

    fn config(self) -> Self::Config {
        self.0
    }
}

impl ViewConfiguration for MultiDatePickerConfig {
    type View = MultiDatePicker;

    fn render(self) -> Self::View {
        MultiDatePicker(self)
    }
}

impl From<MultiDatePickerConfig> for MultiDatePicker {
    fn from(value: MultiDatePickerConfig) -> Self {
        Self(value)
    }
}

impl waterui_core::NativeView for MultiDatePickerConfig {
    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::None
    }
}

impl MultiDatePickerConfig {
    #[must_use]
    fn resolve(mut self, env: &Environment) -> Self {
        self.label = self.label.resolve(env);
        self
    }
}

impl MultiDatePicker {
    /// Creates a new `MultiDatePicker` with the given semantic label, selected
    /// dates binding, and visible month binding.
    ///
    /// The label is required so screen readers always have meaningful text to
    /// announce. Use [`hide_label`](Self::hide_label) to omit it visually
    /// while keeping it in the accessibility tree.
    #[must_use]
    pub fn new(
        label: impl IntoLabel,
        date: &Binding<BTreeSet<Date>>,
        visible_month: &Binding<Date>,
    ) -> Self {
        let range = Date::MIN..=Date::MAX;
        Self(MultiDatePickerConfig {
            label: label.into_label(),
            value: map_multi_date_binding(date),
            range,
            decorated: Computed::constant(Vec::new()),
            visible_month: map_visible_month_binding(visible_month),
        })
    }

    /// Sets the valid date range for the picker.
    #[must_use]
    pub fn range(mut self, range: RangeInclusive<Date>) -> Self {
        if !visible_month_in_range(self.0.visible_month.get(), &range) {
            self.0.visible_month.set(initial_visible_month(
                self.0.value.get().into_iter().next(),
                &range,
            ));
        }
        self.0.range = range;
        self
    }

    /// Marks calendar days with a passive decoration dot.
    #[must_use]
    pub fn decorated(mut self, decorated: impl IntoComputed<BTreeSet<Date>>) -> Self {
        self.0.decorated = decorated
            .into_computed()
            .map(|dates| dates.into_iter().collect())
            .computed();
        self
    }
}

impl_label_style_methods!(MultiDatePicker);

impl View for MultiDatePicker {
    fn body(self, env: &Environment) -> impl View {
        let config = self.0;
        if let Some(hook) = env.get::<Hook<MultiDatePickerConfig>>() {
            AnyView::new(hook.apply(env, config))
        } else {
            AnyView::new(MultiDatePickerFallback::from_config(config.resolve(env)))
        }
    }

    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::None
    }
}

#[derive(Debug)]
struct MultiDatePickerFallback {
    label: Label,
    value: Binding<BTreeSet<Date>>,
    range: RangeInclusive<Date>,
    decorated: Computed<BTreeSet<Date>>,
    visible_month: Binding<VisibleMonth>,
}

impl MultiDatePickerFallback {
    fn from_config(config: MultiDatePickerConfig) -> Self {
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
            range: config.range.clone(),
            decorated: config
                .decorated
                .map(|dates: Vec<Date>| dates.into_iter().collect::<BTreeSet<_>>())
                .computed(),
            visible_month: config.visible_month,
        }
    }
}

impl View for MultiDatePickerFallback {
    fn body(self, env: &Environment) -> impl View {
        let label = self.label;
        let selection = self.value;
        let range = self.range;
        let decorated = self.decorated;
        let visible_month = self.visible_month;
        let locale = resolve_locale(env);

        let calendar_state = visible_month
            .clone()
            .zip(&selection.zip(&decorated))
            .computed();
        let calendar = signal_driven_view(
            calendar_state,
            move |(month, (selected_dates, decorated_dates))| {
                let cell_range = range.clone();
                let cell_selection = selection.clone();
                CalendarBody::new(
                    locale.clone(),
                    month,
                    range.clone(),
                    visible_month.clone(),
                    calendar_rows(month, move |cell| {
                        multi_day_cell_content(
                            cell,
                            &selected_dates,
                            &cell_range,
                            cell_selection.clone(),
                            &decorated_dates,
                        )
                    }),
                )
            },
        );

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
