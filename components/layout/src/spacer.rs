use waterui_core::raw_view;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
pub struct Spacer;

raw_view!(Spacer);

pub const fn spacer() -> Spacer {
    Spacer
}
