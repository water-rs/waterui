mod vstack;
pub use vstack::*;
mod hstack;
pub use hstack::*;
mod zstack;
pub use zstack::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalAlignment {
    Top,
    #[default]
    Center,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HorizontalAlignment {
    Leading,
    #[default]
    Center,
    Trailing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Alignment {
    Top,
    TopLeading,
    TopTrailing,
    #[default]
    Center,
    Bottom,
    BottomLeading,
    BottomTrailing,
    Leading,
    Trailing,
}
