use core::ops::{RangeBounds, RangeInclusive};

use crate::view::ViewExt;
use time::Date;
use waterui_core::{AnyView, View};
use waterui_reactive::Binding;
#[derive(Debug)]
pub struct DatePickerConfig {
    pub label: AnyView,
    pub value: Binding<Date>,
    pub range: RangeInclusive<Date>,
    pub ty: DatePickerType,
}

#[derive(Debug, Default)]
pub enum DatePickerType {
    Date,
    HourAndMinute,
    HourMinuteAndSecond,
    #[default]
    DateHourAndMinute,
    DateHourMinuteAndSecond,
}

configurable!(DatePicker, DatePickerConfig);

impl DatePicker {
    pub fn new(date: &Binding<Date>) -> Self {
        Self(DatePickerConfig {
            label: AnyView::default(),
            value: date.clone(),
            range: Date::MIN..=Date::MAX,
            ty: DatePickerType::default(),
        })
    }

    pub fn range(mut self, range: impl RangeBounds<Date> + 'static) -> Self {
        self.0.value = self.0.value.range(range);
        self
    }

    pub fn label(mut self, label: impl View) -> Self {
        self.0.label = label.anyview();
        self
    }

    pub fn ty(mut self, ty: DatePickerType) -> Self {
        self.0.ty = ty;
        self
    }
}
