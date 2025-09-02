pub mod stack {
    use crate::{array::WuiArray, ffi_enum, ffi_struct, ffi_view};
    use waterui::AnyView;

    use waterui::component::layout::stack::{Stack, StackMode};

    ffi_enum!(StackMode, WuiStackMode, Vertical, Horizonal, Layered);

    #[repr(C)]
    pub struct WuiStack {
        pub contents: WuiArray<*mut AnyView>,
        pub mode: WuiStackMode,
    }

    ffi_struct!(Stack, WuiStack, contents, mode);
    ffi_view!(Stack, WuiStack, waterui_stack_id, waterui_force_as_stack);
}

ffi_enum!(
    waterui::component::layout::Alignment,
    WuiAlignment,
    Default,
    Leading,
    Center,
    Trailing
);

pub mod grid {
    use crate::{array::WuiArray, ffi_struct, ffi_view};
    use waterui::AnyView;

    use super::WuiAlignment;

    use waterui::component::layout::grid::{Grid, GridRow};

    #[repr(C)]
    pub struct WuiGridRow {
        pub columns: WuiArray<*mut AnyView>,
    }

    ffi_struct!(GridRow, WuiGridRow, columns);

    #[repr(C)]
    pub struct WuiGrid {
        pub alignment: WuiAlignment,
        pub h_space: f64,
        pub v_space: f64,
        pub rows: WuiArray<WuiGridRow>,
    }

    ffi_struct!(Grid, WuiGrid, alignment, h_space, v_space, rows);

    ffi_view!(Grid, WuiGrid, waterui_grid_id, waterui_force_as_grid);
}

pub(crate) mod scroll {
    use waterui::{
        AnyView,
        component::layout::scroll::{Axis, ScrollView},
    };

    use crate::{ffi_enum, ffi_struct};

    #[repr(C)]
    pub struct WuiScrollView {
        pub content: *mut AnyView,
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

pub mod overlay {
    use waterui::{AnyView, component::layout::overlay::Overlay};

    use crate::{ffi_struct, ffi_view};

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
