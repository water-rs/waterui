//! Alignment tokens: unit values that name one position and convert into
//! exactly the alignment types that position is legal for.
//!
//! `vstack(..).alignment(Leading)` reads as the intent; `vstack(..).alignment(Top)`
//! does not compile, because `Top` has no `Into<HorizontalAlignment>`. The
//! per-container methods (`.leading()`, `.top()`, `.top_leading()`, …) are the
//! same positions spelled without an argument.

use waterui_core::layout::{Alignment, HorizontalAlignment, VerticalAlignment};

/// Declares one alignment token and the conversions it is legal for.
macro_rules! token {
    ($(#[$meta:meta])* $name:ident => $($target:ident),+) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        pub struct $name;

        $(
            impl From<$name> for $target {
                fn from(_: $name) -> Self {
                    Self::$name
                }
            }
        )+
    };
}

token!(
    /// The leading edge: a `VStack` column, or a `ZStack`/`Frame`/`Grid`
    /// cell centered vertically.
    Leading => HorizontalAlignment, Alignment
);
token!(
    /// The trailing edge: a `VStack` column, or a two-dimensional cell
    /// centered vertically.
    Trailing => HorizontalAlignment, Alignment
);
token!(
    /// The center on whichever axes the container aligns.
    Center => HorizontalAlignment, VerticalAlignment, Alignment
);
token!(
    /// The top edge: an `HStack` row, or a two-dimensional cell centered
    /// horizontally.
    Top => VerticalAlignment, Alignment
);
token!(
    /// The bottom edge: an `HStack` row, or a two-dimensional cell centered
    /// horizontally.
    Bottom => VerticalAlignment, Alignment
);
token!(
    /// The first text baseline of an `HStack` row.
    FirstBaseline => VerticalAlignment
);
token!(
    /// The last text baseline of an `HStack` row.
    LastBaseline => VerticalAlignment
);
token!(
    /// The top-leading corner of a two-dimensional cell.
    TopLeading => Alignment
);
token!(
    /// The top-trailing corner of a two-dimensional cell.
    TopTrailing => Alignment
);
token!(
    /// The bottom-leading corner of a two-dimensional cell.
    BottomLeading => Alignment
);
token!(
    /// The bottom-trailing corner of a two-dimensional cell.
    BottomTrailing => Alignment
);

/// The horizontal positions, as methods: `.leading()`, `.centered()`,
/// `.trailing()`. Expands inside an `impl` block whose type has
/// `fn alignment(self, impl Into<HorizontalAlignment>) -> Self`.
macro_rules! horizontal_alignment_methods {
    () => {
        /// Aligns children to the leading edge.
        #[must_use]
        pub fn leading(self) -> Self {
            self.alignment($crate::alignment::Leading)
        }

        /// Centers children horizontally.
        #[must_use]
        pub fn centered(self) -> Self {
            self.alignment($crate::alignment::Center)
        }

        /// Aligns children to the trailing edge.
        #[must_use]
        pub fn trailing(self) -> Self {
            self.alignment($crate::alignment::Trailing)
        }
    };
}

/// The vertical positions, as methods: `.top()`, `.centered()`, `.bottom()`,
/// `.first_baseline()`, `.last_baseline()`. Expands inside an `impl` block
/// whose type has `fn alignment(self, impl Into<VerticalAlignment>) -> Self`.
macro_rules! vertical_alignment_methods {
    () => {
        /// Aligns children to the top edge.
        #[must_use]
        pub fn top(self) -> Self {
            self.alignment($crate::alignment::Top)
        }

        /// Centers children vertically.
        #[must_use]
        pub fn centered(self) -> Self {
            self.alignment($crate::alignment::Center)
        }

        /// Aligns children to the bottom edge.
        #[must_use]
        pub fn bottom(self) -> Self {
            self.alignment($crate::alignment::Bottom)
        }

        /// Aligns children on their first text baseline.
        #[must_use]
        pub fn first_baseline(self) -> Self {
            self.alignment($crate::alignment::FirstBaseline)
        }

        /// Aligns children on their last text baseline.
        #[must_use]
        pub fn last_baseline(self) -> Self {
            self.alignment($crate::alignment::LastBaseline)
        }
    };
}

/// The nine two-dimensional positions, as methods. Expands inside an `impl`
/// block whose type has `fn alignment(self, impl Into<Alignment>) -> Self`.
macro_rules! two_dimensional_alignment_methods {
    () => {
        /// Aligns content to the top-leading corner.
        #[must_use]
        pub fn top_leading(self) -> Self {
            self.alignment($crate::alignment::TopLeading)
        }

        /// Aligns content to the top edge, centered horizontally.
        #[must_use]
        pub fn top(self) -> Self {
            self.alignment($crate::alignment::Top)
        }

        /// Aligns content to the top-trailing corner.
        #[must_use]
        pub fn top_trailing(self) -> Self {
            self.alignment($crate::alignment::TopTrailing)
        }

        /// Aligns content to the leading edge, centered vertically.
        #[must_use]
        pub fn leading(self) -> Self {
            self.alignment($crate::alignment::Leading)
        }

        /// Centers content on both axes.
        #[must_use]
        pub fn centered(self) -> Self {
            self.alignment($crate::alignment::Center)
        }

        /// Aligns content to the trailing edge, centered vertically.
        #[must_use]
        pub fn trailing(self) -> Self {
            self.alignment($crate::alignment::Trailing)
        }

        /// Aligns content to the bottom-leading corner.
        #[must_use]
        pub fn bottom_leading(self) -> Self {
            self.alignment($crate::alignment::BottomLeading)
        }

        /// Aligns content to the bottom edge, centered horizontally.
        #[must_use]
        pub fn bottom(self) -> Self {
            self.alignment($crate::alignment::Bottom)
        }

        /// Aligns content to the bottom-trailing corner.
        #[must_use]
        pub fn bottom_trailing(self) -> Self {
            self.alignment($crate::alignment::BottomTrailing)
        }
    };
}

pub(crate) use {
    horizontal_alignment_methods, two_dimensional_alignment_methods, vertical_alignment_methods,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_convert_to_the_positions_they_name() {
        assert_eq!(
            HorizontalAlignment::from(Leading),
            HorizontalAlignment::Leading
        );
        assert_eq!(VerticalAlignment::from(Bottom), VerticalAlignment::Bottom);
        assert_eq!(Alignment::from(Center), Alignment::Center);
        assert_eq!(Alignment::from(Leading), Alignment::Leading);
        assert_eq!(Alignment::from(BottomTrailing), Alignment::BottomTrailing);
    }
}
