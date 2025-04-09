use waterui_core::Str;
use waterui_core::{AnyView, configurable};
use waterui_reactive::Computed;

#[derive(Debug)]
pub struct LinkConfig {
    pub label: AnyView,
    pub url: Computed<Str>,
}

configurable!(Link, LinkConfig);

pub(crate) mod ffi {
    use waterui_core::{AnyView, Computed, Str, components::Native, ffi::ffi_struct, ffi_view};

    use super::LinkConfig;

    #[repr(C)]
    pub struct WuiLink {
        pub label: *mut AnyView,
        pub url: *mut Computed<Str>,
    }

    ffi_struct!(LinkConfig, WuiLink, label, url);
    ffi_view!(
        Native<LinkConfig>,
        WuiLink,
        waterui_link_id,
        waterui_force_as_link
    );
}
