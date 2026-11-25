//! The `multi_date_picker` leaf: the `MultiDatePicker` composer claimed
//! before `body()` expands it, rendered through the shared `calendar` leaf
//! in multi-selection mode — `UICalendarView` with
//! `UICalendarSelectionMultiDate` on `UIKit`, cocoa-ui's month calendar on
//! `AppKit`.
//!
//! A pick toggles the day: a selected day leaves the selection, any other
//! joins it in date order. `Native<MultiDatePickerConfig>` is claimed too,
//! for the payload a hook emits.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use core::ops::RangeInclusive;

use cocoa_ui::date::CalendarSelectionMode;
use waterui::component::form::picker::date::Date;
use waterui::component::form::picker::multi_date::{MultiDatePickerConfig, clamp_dates_to_range};
use waterui::reactive::{Binding, Signal};

use super::calendar::{CalendarSelection, CalendarSpec, render_calendar};
use crate::contract::{Held, NativeLeaf, RenderContext};

/// The toggle a pick applies to the date-ordered selection: `day` leaves
/// it when present, else joins it in order.
fn toggle_selection(selected: &mut BTreeSet<Date>, day: Date) {
    if !selected.remove(&day) {
        selected.insert(day);
    }
}

impl CalendarSelection for Binding<BTreeSet<Date>> {
    const MODE: CalendarSelectionMode = CalendarSelectionMode::Multiple;

    fn selected(value: &BTreeSet<Date>) -> Vec<Date> {
        value.iter().copied().collect()
    }

    fn pick(&self, day: Date) {
        let mut selected = self.snapshot();
        toggle_selection(&mut selected, day);
        self.set(selected);
    }

    fn clamp_to_range(&self, range: &RangeInclusive<Date>) {
        clamp_dates_to_range(self, range);
    }
}

/// Renders the config through the shared calendar leaf.
#[expect(
    clippy::needless_pass_by_value,
    reason = "NativeBackend hands each payload by value"
)]
fn render(config: MultiDatePickerConfig<Held>, ctx: &RenderContext<'_>) -> NativeLeaf {
    render_calendar(
        CalendarSpec {
            label: config.label().clone(),
            selection: config.value().clone(),
            range: config.range().clone(),
            decorated: config.decorated().clone(),
            visible_month: config.visible_month().clone(),
        },
        ctx,
    )
}

/// The `multi_date_picker` realization: `Native<MultiDatePickerConfig>` maps
/// to the platform multi-date calendar.
pub fn realize_multi_date_picker_config(
    config: MultiDatePickerConfig<Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    render(config, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(year: i16, month: i8, day: i8) -> Date {
        Date::new(year, month, day).unwrap()
    }

    #[test]
    fn toggle_adds_in_order_and_removes() {
        let mut selected = BTreeSet::from([day(2025, 3, 1), day(2025, 3, 10)]);
        toggle_selection(&mut selected, day(2025, 3, 5));
        assert_eq!(
            selected,
            BTreeSet::from([day(2025, 3, 1), day(2025, 3, 5), day(2025, 3, 10)])
        );
        toggle_selection(&mut selected, day(2025, 3, 5));
        assert_eq!(
            selected,
            BTreeSet::from([day(2025, 3, 1), day(2025, 3, 10)])
        );
    }
}
