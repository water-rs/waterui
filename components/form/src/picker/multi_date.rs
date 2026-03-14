//! Multi-date picker component.

use alloc::{collections::BTreeSet, vec::Vec};
use core::ops::RangeInclusive;

use jiff::civil::Date;
use nami::{Binding, Computed, SignalExt, signal::IntoComputed};
use waterui_controls::IntoLabel;
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};
use waterui_core::{AnyView, Environment, View};

use crate::calendar::{
    CalendarBody, MultiDayCellView, VisibleMonth, calendar_rows, initial_visible_month,
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
    pub fn range(mut self, range: RangeInclusive<Date>) -> Self {
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
    visible_month: Binding<VisibleMonth>,
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
        let initial_month =
            initial_visible_month(value.get().iter().next().copied(), &config.range);
        Self {
            label: config.label,
            value,
            range: config.range.clone(),
            visible_month: Binding::container(initial_month),
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
        let visible_month = self.visible_month;
        let range = self.range;
        let decorated = self.decorated;
        let locale = resolve_locale(env);

        waterui_layout::stack::vstack((
            label,
            waterui_core::dynamic::Dynamic::watch(visible_month.clone(), move |month| {
                MultiCalendarMonthView::new(
                    locale.clone(),
                    month,
                    range.clone(),
                    visible_month.clone(),
                    selection.clone(),
                    decorated.clone(),
                )
            }),
        ))
        .spacing(10.0)
    }
}

#[derive(Debug, Clone)]
struct MultiCalendarMonthView {
    locale: waterui_locale::Locale,
    month: VisibleMonth,
    range: RangeInclusive<Date>,
    visible_month: Binding<VisibleMonth>,
    selection: Binding<BTreeSet<Date>>,
    decorated: Computed<BTreeSet<Date>>,
}

impl MultiCalendarMonthView {
    fn new(
        locale: waterui_locale::Locale,
        month: VisibleMonth,
        range: RangeInclusive<Date>,
        visible_month: Binding<VisibleMonth>,
        selection: Binding<BTreeSet<Date>>,
        decorated: Computed<BTreeSet<Date>>,
    ) -> Self {
        Self {
            locale,
            month,
            range,
            visible_month,
            selection,
            decorated,
        }
    }
}

impl View for MultiCalendarMonthView {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        CalendarBody::new(
            self.locale,
            self.month,
            self.range.clone(),
            self.visible_month,
            calendar_rows(self.month, move |cell| {
                MultiDayCellView::new(
                    cell,
                    self.selection.clone(),
                    self.range.clone(),
                    self.decorated.clone(),
                )
            }),
        )
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
