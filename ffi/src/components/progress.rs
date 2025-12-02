use waterui::{
    component::progress::{ProgressConfig, ProgressStyle},
    prelude::Progress,
};

use crate::{WuiAnyView, reactive::WuiComputed};

into_ffi! {ProgressStyle,Circular,
    pub enum WuiProgressStyle {
        Linear,
        Circular,
    }
}

into_ffi! {
    ProgressConfig,
    pub struct WuiProgress {
        label: *mut WuiAnyView,
        value_label: *mut WuiAnyView,
        value: *mut WuiComputed<f64>,
        style: WuiProgressStyle,
    }
}

ffi_view!(ProgressConfig, WuiProgress, progress);
