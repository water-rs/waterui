use super::Text;
use crate::view::ViewExt;
use waterui_reactive::{
    compute::{IntoCompute, IntoComputed},
    Computed, CowStr,
};
use waterui_reactive::{ComputeExt, Int};
use waterui_view::{AnyView, View};

const PROGRESS_INNER_VALUE_MAX: Int = 10 ^ 5;
pub struct Progress<Label> {
    label: Label,
    progress: Computed<Int>,
    style: ProgressStyle,
}

#[repr(C)]
#[non_exhaustive]
pub enum ProgressStyle {
    Default,
    Circular,
    Linear,
}

impl_label!(Progress);

impl Default for ProgressStyle {
    fn default() -> Self {
        Self::Default
    }
}

impl Progress<()> {
    pub fn infinity() -> Self {
        Self {
            label: (),
            progress: Computed::constant(-1),
            style: ProgressStyle::default(),
        }
    }

    pub fn new(progress: impl IntoCompute<Option<f64>> + 'static) -> Self {
        Self::infinity().progress(progress)
    }
}

impl<Label: View> Progress<Label> {
    pub fn style(mut self, style: ProgressStyle) -> Self {
        self.style = style;
        self
    }

    pub fn progress(mut self, progress: impl IntoCompute<Option<f64>> + 'static) -> Self {
        self.progress = progress
            .into_compute()
            .map(|n| {
                if let Some(n) = n {
                    PROGRESS_INNER_VALUE_MAX / ((1.0 / n) as Int)
                } else {
                    -1
                }
            })
            .computed();
        self
    }

    pub fn label_view<V: View>(self, label: V) -> Progress<V> {
        Progress {
            label,
            progress: self.progress,
            style: self.style,
        }
    }

    pub fn label(self, label: impl IntoComputed<CowStr>) -> Progress<Text> {
        self.label_view(Text::new(label))
    }
}

impl<Label: View + 'static> View for Progress<Label> {
    fn body(self, _env: waterui_view::Environment) -> impl View {
        RawProgress {
            _label: self.label.anyview(),
            _progress: self.progress,
            _style: self.style,
        }
    }
}

pub struct RawProgress {
    pub _label: AnyView,
    pub _progress: Computed<Int>,
    pub _style: ProgressStyle,
}

raw_view!(RawProgress);

pub fn progress() -> Progress<()> {
    Progress::infinity()
}

mod ffi {
    use waterui_ffi::{computed::waterui_computed_int, ffi_view, waterui_anyview, IntoFFI};

    use super::ProgressStyle;

    pub type waterui_style_progress = ProgressStyle;

    #[repr(C)]
    pub struct Progress {
        label: *mut waterui_anyview,
        progress: *mut waterui_computed_int,
        style: waterui_style_progress,
    }

    impl IntoFFI for super::RawProgress {
        type FFI = Progress;
        fn into_ffi(self) -> Self::FFI {
            Progress {
                label: self._label.into_ffi(),
                progress: self._progress.into_ffi(),
                style: self._style,
            }
        }
    }

    ffi_view!(
        super::RawProgress,
        Progress,
        waterui_view_force_as_progress,
        waterui_view_progress_id
    );
}
