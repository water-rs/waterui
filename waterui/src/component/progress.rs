use crate::component::text;
use crate::AnyView;
use crate::ViewExt;
use alloc::format;
use waterui_core::View;
use waterui_reactive::compute::IntoComputed;
use waterui_reactive::zip::FlattenMap;
use waterui_reactive::{ComputeExt, Computed};

#[non_exhaustive]
#[derive(Debug)]
pub struct ProgressConfig {
    pub label: AnyView,
    pub value_label: AnyView,
    pub value: Computed<f64>,
    pub style: ProgressStyle,
}

#[non_exhaustive]
#[derive(Debug)]
pub enum ProgressStyle {
    Circular,
    Linear,
}

configurable!(Progress, ProgressConfig);

#[derive(Debug)]
pub struct ProgressWithTotal(Progress);

impl ProgressWithTotal {
    pub fn label(self, label: impl View) -> Self {
        Self(self.0.label(label))
    }

    pub fn circular(self) -> Self {
        Self(self.0.circular())
    }

    pub fn linear(self) -> Self {
        Self(self.0.linear())
    }
}

impl View for ProgressWithTotal {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        self.0
    }
}

impl Progress {
    pub fn new(value: impl IntoComputed<f64>) -> Self {
        let value = value.into_computed();
        Self(ProgressConfig {
            label: text("Please wait...").anyview(),
            value_label: text(value.clone().map(|v| format!("{v:.2} %"))).anyview(),
            value,
            style: ProgressStyle::Linear,
        })
    }

    pub fn total(mut self, total: impl IntoComputed<f64>) -> ProgressWithTotal {
        self.0.value = total
            .into_compute()
            .zip(self.0.value)
            .flatten_map(|total, value| value / total)
            .computed();

        ProgressWithTotal(self)
    }

    pub fn infinity() -> Self {
        Self::new(f64::NAN)
    }

    pub fn label(mut self, label: impl View) -> Self {
        self.0.label = label.anyview();
        self
    }

    fn style(mut self, style: ProgressStyle) -> Self {
        self.0.style = style;
        self
    }

    pub fn circular(self) -> Self {
        self.style(ProgressStyle::Circular)
    }

    pub fn linear(self) -> Self {
        self.style(ProgressStyle::Linear)
    }
}

pub fn progress(value: impl IntoComputed<f64>) -> Progress {
    Progress::new(value)
}

pub fn loading() -> Progress {
    Progress::infinity().circular()
}
