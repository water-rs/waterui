use core::ops::RangeInclusive;

use alloc::format;
use waterui_core::{AnyView, View};
use waterui_reactive::Binding;

use crate::ViewExt;

#[derive(Debug)]
#[non_exhaustive]
pub struct SliderConfig {
    pub label: AnyView,
    pub min_value_label: AnyView,
    pub max_value_label: AnyView,
    pub range: RangeInclusive<f64>,
    pub value: Binding<f64>,
}

configurable!(Slider, SliderConfig);

impl Slider {
    pub fn new(range: RangeInclusive<f64>, value: &Binding<f64>) -> Self {
        Self(SliderConfig {
            label: text!("{:.2}", value).anyview(),
            min_value_label: AnyView::default(),
            max_value_label: AnyView::default(),
            range,
            value: value.clone(),
        })
    }
}

macro_rules! labels {
    ($($name:ident),*) => {
        $(
            pub fn $name(mut self, $name: impl View) -> Self {
                self.0.$name = $name.anyview();
                self
            }
        )*
    };
}

impl Slider {
    labels!(label, min_value_label, max_value_label);
}

pub fn new(range: RangeInclusive<f64>, value: &Binding<f64>) -> Slider {
    Slider::new(range, value)
}
