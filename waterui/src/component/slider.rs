use core::ops::RangeInclusive;

use alloc::format;
use waterui_core::{AnyView, View};
use waterui_reactive::Binding;

use crate::ViewExt;

#[derive(Debug)]
pub struct SliderConfig {
    pub label: AnyView,
    pub min_value_label: AnyView,
    pub max_value_label: AnyView,
    pub current_value_label: AnyView,
    pub range: RangeInclusive<i32>,
    pub value: Binding<i32>,
}

configurable!(Slider, SliderConfig);

impl Slider {
    pub fn new(range: RangeInclusive<i32>, value: &Binding<i32>) -> Self {
        Self(SliderConfig {
            label: AnyView::default(),
            min_value_label: AnyView::default(),
            max_value_label: AnyView::default(),
            current_value_label: text!("{}", value).anyview(),
            range,
            value: value.clone(),
        })
    }

    pub fn label(mut self, label: impl View) -> Self {
        self.0.label = label.anyview();
        self
    }
}
