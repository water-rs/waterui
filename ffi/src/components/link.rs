use crate::WuiAnyView;
use crate::reactive::WuiComputed;
use waterui::Str;
use waterui::component::link::LinkConfig;
use waterui_text::Link;

into_ffi! {LinkConfig,
    pub struct WuiLink {
        label: *mut WuiAnyView,
        url: *mut WuiComputed<Str>,
    }
}

native_view!(Link, WuiLink);
