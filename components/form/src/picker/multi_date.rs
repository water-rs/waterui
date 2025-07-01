use alloc::collections::BTreeSet;

use time::Date;
use waterui_core::{AnyView, View, configurable};
use waterui_reactive::Binding;
#[derive(Debug)]
#[non_exhaustive]
pub struct MultiDatePickerConfig {
    pub label: AnyView,
    pub value: Binding<BTreeSet<Date>>,
}

configurable!(MultiDatePicker, MultiDatePickerConfig);

impl MultiDatePicker {
    pub fn new(date: &Binding<BTreeSet<Date>>) -> Self {
        Self(MultiDatePickerConfig {
            label: AnyView::default(),
            value: date.clone(),
        })
    }

    pub fn label(mut self, label: impl View) -> Self {
        self.0.label = AnyView::new(label);
        self
    }
}
