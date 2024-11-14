use alloc::collections::BTreeSet;

use crate::view::ViewExt;
use time::Date;
use waterui_core::{AnyView, View};
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
        self.0.label = label.anyview();
        self
    }
}
