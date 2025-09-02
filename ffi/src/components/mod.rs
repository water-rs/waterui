use crate::IntoFFI;

pub mod layout;

impl<T: IntoFFI> IntoFFI for waterui::component::Native<T> {
    type FFI = T::FFI;
    fn into_ffi(self) -> Self::FFI {
        IntoFFI::into_ffi(self.0)
    }
}

ffi_view!(waterui::component::divder::Divider, waterui_divider_id);

pub mod button {
    use crate::action::WuiAction;
    use crate::{ffi_struct, ffi_view};
    use waterui::AnyView;
    use waterui::component::Native;
    use waterui::component::button::ButtonConfig;

    /// C representation of a WaterUI button for FFI purposes.
    #[derive(Debug)]
    #[repr(C)]
    pub struct WuiButton {
        /// Pointer to the button's label view
        pub label: *mut AnyView,
        /// Pointer to the button's action handler
        pub action: *mut WuiAction,
    }

    ffi_struct!(ButtonConfig, WuiButton, label, action);
    ffi_view!(
        Native<ButtonConfig>,
        WuiButton,
        waterui_button_id,
        waterui_force_as_button
    );
}

ffi_view!(
    waterui::Str,
    waterui::Str,
    waterui_force_as_label,
    waterui_label_id
);

pub mod link {
    use crate::{ffi_struct, ffi_view};
    use waterui::component::link::LinkConfig;
    use waterui::{AnyView, Computed, Str, component::Native};

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

pub mod text {
    pub(crate) mod ffi {
        use waterui::component::text::font::Font;

        use crate::{color::WuiColor, ffi_struct};

        #[repr(C)]
        pub struct WuiFont {
            pub size: f64,
            pub italic: bool,
            pub strikethrough: WuiColor,
            pub underlined: WuiColor,
            pub bold: bool,
        }

        ffi_struct!(Font, WuiFont, size, italic, strikethrough, underlined, bold);
    }
}
