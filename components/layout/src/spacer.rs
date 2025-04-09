use waterui_core::raw_view;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct Spacer;

raw_view!(Spacer);

pub const fn spacer() -> Spacer {
    Spacer
}

pub(crate) mod ffi {
    use super::Spacer;
    use waterui_core::ffi_view;
    ffi_view!(Spacer, waterui_spacer_id);
}
