use waterui_core::{AnyView, View};
#[derive(Debug)]
#[must_use]
pub struct Overlay {
    pub content: AnyView,
}

pub fn overlay(content: impl View) -> Overlay {
    Overlay {
        content: AnyView::new(content),
    }
}

pub(crate) mod ffi {
    use waterui_core::{AnyView, ffi_view};
    use waterui_ffi::ffi_struct;

    use super::Overlay;

    #[repr(C)]
    pub struct WuiOverlay {
        pub content: *mut AnyView,
    }

    ffi_struct!(Overlay, WuiOverlay, content);
    ffi_view!(
        Overlay,
        WuiOverlay,
        waterui_overlay_id,
        waterui_force_as_overlay
    );
}
