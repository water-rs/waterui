use waterui::component::progress::{ProgressConfig, ProgressStyle};

use crate::{WuiAnyView, reactive::WuiComputed};

into_ffi! {ProgressStyle, non_exhaustive,
    pub enum WuiProgressStyle {
        Linear,
        Circular,
        Loading,
    }
}

into_ffi! {
    ProgressConfig,
    pub struct WuiProgress {
        label: *mut WuiAnyView,
        value_label: *mut WuiAnyView,
        value: *mut WuiComputed<f64>,
        style: WuiProgressStyle,
        four_color: bool,
    }
}

ffi_view!(ProgressConfig, WuiProgress, progress);
