//! A multi-date picker rendered entirely with WaterUI primitives.

use alloc::collections::BTreeSet;
use core::ops::RangeInclusive;

use jiff::civil::Date;
use nami::{Binding, Computed, SignalExt, signal::IntoComputed};
use waterui_core::View;
use waterui_core::dynamic::Dynamic;

use crate::calendar::{
    VisibleMonth, build_calendar_body, calendar_rows, initial_visible_month, multi_day_cell_view,
    resolve_locale,
};

#[derive(Debug)]
/// A calendar-style control for selecting multiple dates.
pub struct MultiDatePicker {
    label: waterui_core::AnyView,
    value: Binding<BTreeSet<Date>>,
    range: RangeInclusive<Date>,
    visible_month: Binding<VisibleMonth>,
    decorated: Computed<BTreeSet<Date>>,
}

impl MultiDatePicker {
    /// Creates a new `MultiDatePicker` with the given binding for selected dates.
    #[must_use]
    pub fn new(date: &Binding<BTreeSet<Date>>) -> Self {
        let range = Date::MIN..=Date::MAX;
        let visible_month = Binding::container(initial_visible_month(
            date.get().iter().next().copied(),
            &range,
        ));
        Self {
            label: waterui_core::AnyView::default(),
            value: date.clone(),
            range,
            visible_month,
            decorated: Computed::constant(BTreeSet::new()),
        }
    }

    /// Sets the label for the multi-date picker.
    #[must_use]
    pub fn label(mut self, label: impl View) -> Self {
        self.label = waterui_core::AnyView::new(label);
        self
    }

    /// Sets the valid date range for the picker.
    #[must_use]
    pub fn range(mut self, range: RangeInclusive<Date>) -> Self {
        let initial_month = initial_visible_month(self.value.get().iter().next().copied(), &range);
        self.visible_month.set(initial_month);
        self.range = range;
        self
    }

    /// Marks calendar days with a passive decoration dot.
    #[must_use]
    pub fn decorated(mut self, decorated: impl IntoComputed<BTreeSet<Date>>) -> Self {
        self.decorated = decorated.into_computed();
        self
    }
}

impl View for MultiDatePicker {
    fn body(self, env: &waterui_core::Environment) -> impl View {
        let label = self.label;
        let selection = self.value;
        let visible_month = self.visible_month;
        let range = self.range;
        let decorated = self.decorated;
        let locale = resolve_locale(env);

        waterui_layout::stack::vstack((
            label,
            Dynamic::watch(
                visible_month.zip(&selection).zip(&decorated),
                move |((month, selected_dates), decorated_dates)| {
                    waterui_core::AnyView::new(build_calendar_body(
                        &locale,
                        month,
                        &range,
                        visible_month.clone(),
                        calendar_rows(month, |cell| {
                            multi_day_cell_view(
                                cell,
                                &selected_dates,
                                &range,
                                selection.clone(),
                                &decorated_dates,
                            )
                        }),
                    ))
                },
            ),
        ))
        .spacing(10.0)
    }
}
