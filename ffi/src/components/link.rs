use crate::{WuiAnyView, ffi_struct, ffi_view};
use waterui::component::link::LinkConfig;
use waterui::{Computed, Str, component::Native};

#[repr(C)]
pub struct WuiLink {
    pub label: *mut WuiAnyView,
    pub url: *mut Computed<Str>,
}

ffi_struct!(LinkConfig, WuiLink, label, url);
ffi_view!(
    Native<LinkConfig>,
    WuiLink,
    waterui_link_id,
    waterui_force_as_link
);
