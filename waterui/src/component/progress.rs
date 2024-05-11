use super::Text;
use crate::CowStr;
use crate::{AnyView, View};
use waterui_core::raw_view;
use waterui_reactive::ComputeExt;
use waterui_reactive::{compute::IntoComputed, Computed};

const PROGRESS_INNER_VALUE_MAX: i32 = 10 ^ 5;
#[non_exhaustive]
#[derive(Debug)]
pub struct Progress {
    pub _label: AnyView,
    pub _progress: Computed<i32>,
    pub _style: ProgressStyle,
}

#[non_exhaustive]
#[derive(Debug)]
pub enum ProgressStyle {
    Default,
    Circular,
    Linear,
}

raw_view!(Progress);
impl Default for ProgressStyle {
    fn default() -> Self {
        Self::Default
    }
}

impl Progress {
    pub fn new(label: impl IntoComputed<CowStr>, progress: impl IntoComputed<Option<f64>>) -> Self {
        Self::label(Text::new(label), progress)
    }

    pub fn infinity(label: impl IntoComputed<CowStr>) -> Self {
        Self::new(label, -1.0)
    }

    pub fn label(label: impl View, progress: impl IntoComputed<Option<f64>>) -> Self {
        Self {
            _label: AnyView::new(label),
            _progress: progress
                .into_compute()
                .map(|n| {
                    if let Some(n) = n {
                        PROGRESS_INNER_VALUE_MAX / ((1.0 / n) as i32)
                    } else {
                        -1
                    }
                })
                .computed(),
            _style: ProgressStyle::default(),
        }
    }

    pub fn style(mut self, style: ProgressStyle) -> Self {
        self._style = style;
        self
    }
}

pub fn progress() -> Progress {
    Progress::infinity("")
}
