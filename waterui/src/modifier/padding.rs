use crate::layout::Edge;
#[repr(C)]
#[derive(Default)]
pub struct Padding {
    pub _inner: Edge,
}

impl Padding {
    pub fn new(padding: Edge) -> Self {
        Self { _inner: padding }
    }
}

#[doc(hidden)]
pub mod ffi {
    use waterui_ffi::IntoFFI;

    use super::Padding;

    impl IntoFFI for Padding {
        type FFI = Padding;
        fn into_ffi(self) -> Self::FFI {
            self
        }
    }
    ffi_with_modifier!(
        Padding,
        waterui_modifier_force_as_padding,
        waterui_modifier_padding_id
    );
}
