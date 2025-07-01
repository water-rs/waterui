use core::ops::RangeInclusive;

use alloc::format;
use waterui_core::{AnyView, View, configurable};
use waterui_reactive::Binding;
use waterui_text::text;

/// Configuration for the [`Slider`] widget.
#[derive(Debug)]
#[non_exhaustive]
pub struct SliderConfig {
    /// The label to display for the slider.
    pub label: AnyView,
    /// The label for the minimum value of the slider.
    pub min_value_label: AnyView,
    /// The label for the maximum value of the slider.
    pub max_value_label: AnyView,
    /// The range of values the slider can take.
    pub range: RangeInclusive<f64>,
    /// The binding to the current value of the slider.
    pub value: Binding<f64>,
}

configurable!(Slider, SliderConfig);

impl Slider {
    /// Creates a new [`Slider`] widget.
    #[must_use]
    pub fn new(range: RangeInclusive<f64>, value: &Binding<f64>) -> Self {
        Self(SliderConfig {
            label: AnyView::new(text!("{:.2}", value)),
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
            #[must_use]
            /// Sets the label for the slider.
            pub fn $name(mut self, $name: impl View) -> Self {
                self.0.$name = AnyView::new($name);
                self
            }
        )*
    };
}

impl Slider {
    labels!(label, min_value_label, max_value_label);
}

/// Creates a new [`Slider`] with the specified range and value binding.
#[must_use]
pub fn slider(range: RangeInclusive<f64>, value: &Binding<f64>) -> Slider {
    Slider::new(range, value)
}
