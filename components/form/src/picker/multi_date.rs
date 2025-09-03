//! A data structure that represents a sorted set of dates.
//!
//! It uses a `BTreeSet` internally to store the dates, which guarantees that the dates are always sorted.
//! This is useful for scenarios where you need to iterate over the dates in a specific order or perform range queries.
//!
//! The `MultiDate` struct provides methods for adding, removing, and checking the existence of dates in the set.
//! It also supports operations like finding the minimum and maximum dates, as well as iterating over the dates in the set.
use alloc::collections::BTreeSet;

use nami::Binding;
use time::Date;
use waterui_core::{AnyView, View, configurable};
#[derive(Debug)]
#[non_exhaustive]
/// Configuration for the `MultiDatePicker` component.
pub struct MultiDatePickerConfig {
    /// The label to display for the multi-date picker.
    pub label: AnyView,
    /// The binding to the set of selected dates.
    pub value: Binding<BTreeSet<Date>>,
}

configurable!(MultiDatePicker, MultiDatePickerConfig);

impl MultiDatePicker {
    /// Creates a new `MultiDatePicker` with the given binding for selected dates.
    #[must_use]
    pub fn new(date: &Binding<BTreeSet<Date>>) -> Self {
        Self(MultiDatePickerConfig {
            label: AnyView::default(),
            value: date.clone(),
        })
    }

    /// Sets the label for the multi-date picker.
    #[must_use]
    pub fn label(mut self, label: impl View) -> Self {
        self.0.label = AnyView::new(label);
        self
    }
}
