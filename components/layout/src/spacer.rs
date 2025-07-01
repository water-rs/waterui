use waterui_core::raw_view;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use]
/// A view that acts as a spacer, providing empty space in the layout.
pub struct Spacer;

raw_view!(Spacer);

/// Creates a new spacer view.
pub const fn spacer() -> Spacer {
    Spacer
}
