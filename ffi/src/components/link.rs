use crate::WuiAnyView;
use crate::reactive::WuiComputed;
use waterui::Str;
use waterui::component::link::LinkConfig;

into_ffi! {LinkConfig,
    pub struct WuiLink {
        label: *mut WuiAnyView,
        url: *mut WuiComputed<Str>,
    }
}

ffi_view!(LinkConfig, WuiLink, link);
