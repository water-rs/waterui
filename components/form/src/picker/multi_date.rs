//! Multi-date picker component.

use alloc::{collections::BTreeSet, vec::Vec};
use core::ops::RangeInclusive;

use jiff::civil::Date;
use nami::{Binding, Computed, SignalExt, signal::IntoComputed};
use waterui_controls::IntoLabel;
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};
use waterui_core::{AnyView, Environment, View};

use crate::calendar::{
    CalendarBody, calendar_rows, initial_visible_month, local_binding, multi_day_cell_content,
    resolve_locale,
};

/// Configuration for the `MultiDatePicker` component.
#[derive(Debug)]
#[non_exhaustive]
pub struct MultiDatePickerConfig {
    /// The label displayed for the picker.
    pub label: AnyView,
    /// The selected dates as an ordered vector for backend/FFI transport.
    pub value: Binding<Vec<Date>>,
    /// The valid date range.
    pub range: RangeInclusive<Date>,
    /// Passively decorated dates.
    pub decorated: Computed<Vec<Date>>,
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

impl MultiDatePicker {
    /// Creates a new `MultiDatePicker` with the given binding for selected dates.
    #[must_use]
    pub fn new(date: &Binding<BTreeSet<Date>>) -> Self {
        Self(MultiDatePickerConfig {
            label: AnyView::default(),
            value: map_multi_date_binding(date),
            range: Date::MIN..=Date::MAX,
            decorated: Computed::constant(Vec::new()),
        })
    }

    /// Sets the label for the multi-date picker.
    #[must_use]
    pub fn label(mut self, label: impl IntoLabel) -> Self {
        self.0.label = AnyView::new(label.into_label());
        self
    }

    /// Sets the valid date range for the picker.
    #[must_use]
    pub const fn range(mut self, range: RangeInclusive<Date>) -> Self {
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

impl View for MultiDatePicker {
    fn body(self, env: &Environment) -> impl View {
        let config = self.0;
        if let Some(hook) = env.get::<Hook<MultiDatePickerConfig>>() {
            AnyView::new(hook.apply(env, config))
        } else {
            AnyView::new(MultiDatePickerFallback::from_config(config))
        }
    }

    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::None
    }
}

#[derive(Debug)]
struct MultiDatePickerFallback {
    label: AnyView,
    value: Binding<BTreeSet<Date>>,
    range: RangeInclusive<Date>,
    decorated: Computed<BTreeSet<Date>>,
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
        }
    }
}

impl View for MultiDatePickerFallback {
    fn body(self, env: &Environment) -> impl View {
        let label = self.label;
        let selection = self.value;
        let range = self.range;
        let decorated = self.decorated;
        let locale = resolve_locale(env);
        let visible_month = local_binding(env, {
            let selection = selection.clone();
            let range = range.clone();
            move || initial_visible_month(selection.get().iter().next().copied(), &range)
        });

        let selection_and_decorated = selection.zip(&decorated);
        let calendar = visible_month
            .clone()
            .zip(&selection_and_decorated)
            .map(move |(month, (selected_dates, decorated_dates))| {
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
            })
            .computed();

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
