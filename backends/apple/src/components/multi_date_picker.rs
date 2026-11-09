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

use cocoa_ui::date::CalendarSelectionMode;
use waterui::component::form::picker::date::Date;
use waterui::component::form::picker::multi_date::{MultiDatePicker, MultiDatePickerConfig};
use waterui::reactive::{Binding, Signal, SignalExt};
use waterui::view::{ConfigurableView, Hook};

use super::calendar::{CalendarSelection, CalendarSpec, render_calendar};
use crate::contract::{NativeLeaf, RenderContext};
use crate::dispatch::Dispatcher;

/// The toggle a pick applies to the date-ordered selection: `day` leaves
/// it when present, else joins it in order.
fn toggle_selection(selected: &mut Vec<Date>, day: Date) {
    match selected.binary_search(&day) {
        Ok(index) => {
            selected.remove(index);
        }
        Err(index) => selected.insert(index, day),
    }
}

impl CalendarSelection for Binding<Vec<Date>> {
    const MODE: CalendarSelectionMode = CalendarSelectionMode::Multiple;

    fn selected(value: &Vec<Date>) -> Vec<Date> {
        value.clone()
    }

    fn pick(&self, day: Date) {
        let mut selected = self.snapshot();
        toggle_selection(&mut selected, day);
        self.set(selected);
    }
}

/// Renders the config through the shared calendar leaf.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the dispatcher hands the config by value"
)]
fn render(config: MultiDatePickerConfig, ctx: &RenderContext<'_>) -> NativeLeaf {
    render_calendar(
        CalendarSpec {
            label: config.label().clone(),
            selection: config.value().clone(),
            range: config.range().clone(),
            decorated: config
                .decorated()
                .clone()
                .map(|days: Vec<Date>| days.into_iter().collect::<BTreeSet<_>>())
                .computed(),
            visible_month: config.visible_month().clone(),
        },
        ctx,
    )
}

/// Installs the `multi_date_picker` handler: the composer claim plus the
/// `Native<MultiDatePickerConfig>` claim for hook-produced payloads.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<MultiDatePickerConfig>(render);
    dispatcher.register_view::<MultiDatePicker>(|picker, ctx| {
        let mut config = picker.config();
        // `body()`: an environment hook wins; otherwise the label resolves.
        if let Some(hook) = ctx.env().get::<Hook<MultiDatePickerConfig>>() {
            return ctx.render(hook.apply(ctx.env(), config));
        }
        config.label_mut().resolve_in_place(ctx.env());
        render(config, ctx)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(year: i16, month: i8, day: i8) -> Date {
        Date::new(year, month, day).unwrap()
    }

    #[test]
    fn toggle_adds_in_order_and_removes() {
        let mut selected = vec![day(2025, 3, 1), day(2025, 3, 10)];
        toggle_selection(&mut selected, day(2025, 3, 5));
        assert_eq!(
            selected,
            vec![day(2025, 3, 1), day(2025, 3, 5), day(2025, 3, 10)]
        );
        toggle_selection(&mut selected, day(2025, 3, 5));
        assert_eq!(selected, vec![day(2025, 3, 1), day(2025, 3, 10)]);
    }
}
