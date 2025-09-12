ffi_enum!(
    waterui::component::layout::Alignment,
    WuiAlignment,
    Default,
    Leading,
    Center,
    Trailing
);

pub mod scroll {
    use waterui::component::layout::scroll::{Axis, ScrollView};

    use crate::{WuiAnyView, ffi_enum, ffi_struct};

    #[repr(C)]
    pub struct WuiScrollView {
        pub content: *mut WuiAnyView,
        pub axis: WuiAxis,
    }

    ffi_enum!(Axis, WuiAxis, Horizontal, Vertical, All);

    ffi_struct!(ScrollView, WuiScrollView, content, axis);
    ffi_view!(
        ScrollView,
        WuiScrollView,
        waterui_scroll_id,
        waterui_force_as_scroll
    );
}
