//! Stack-based layout primitives.
//!
//! The submodules implement horizontal, vertical, and overlay stacks. These
//! views arrange child content according to alignments and spacing and are the
//! backbone of most declarative layouts in `WaterUI`.

mod vstack;
pub use vstack::*;
mod hstack;
pub use hstack::*;
mod zstack;
pub use zstack::*;

/// Defines vertical alignment options for layout components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VerticalAlignment {
    /// Pin the child to the top edge.
    Top,
    /// Center the child along the vertical axis.
    #[default]
    Center,
    /// Pin the child to the bottom edge.
    Bottom,
}

/// Defines horizontal alignment options for layout components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HorizontalAlignment {
    /// Pin the child to the leading edge (left in LTR locales).
    Leading,
    /// Center the child along the horizontal axis.
    #[default]
    Center,
    /// Pin the child to the trailing edge (right in LTR locales).
    Trailing,
}

/// Combined two-dimensional alignment used by overlay stacks.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord, Default)]
pub enum Alignment {
    /// Place the child in the top centre.
    Top,
    /// Place the child in the top-left corner.
    TopLeading,
    /// Place the child in the top-right corner.
    TopTrailing,
    #[default]
    /// Place the child exactly in the centre.
    Center,
    /// Place the child in the bottom centre.
    Bottom,
    /// Place the child in the bottom-left corner.
    BottomLeading,
    /// Place the child in the bottom-right corner.
    BottomTrailing,
    /// Place the child in the vertical centre along the leading edge.
    Leading,
    /// Place the child in the vertical centre along the trailing edge.
    Trailing,
}

impl Alignment {
    /// Returns the horizontal alignment component of this alignment.
    #[must_use]
    pub const fn horizontal(&self) -> HorizontalAlignment {
        match self {
            Self::TopLeading | Self::Leading | Self::BottomLeading => HorizontalAlignment::Leading,
            Self::TopTrailing | Self::Trailing | Self::BottomTrailing => {
                HorizontalAlignment::Trailing
            }
            _ => HorizontalAlignment::Center,
        }
    }

    /// Returns the vertical alignment component of this alignment.
    #[must_use]
    pub const fn vertical(&self) -> VerticalAlignment {
        match self {
            Self::TopLeading | Self::Top | Self::TopTrailing => VerticalAlignment::Top,
            Self::BottomLeading | Self::Bottom | Self::BottomTrailing => VerticalAlignment::Bottom,
            _ => VerticalAlignment::Center,
        }
    }
}
