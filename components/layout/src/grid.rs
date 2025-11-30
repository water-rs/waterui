//! A two-dimensional layout that arranges views in columns and rows.

use alloc::{vec, vec::Vec};
use core::num::NonZeroUsize;
use waterui_core::{AnyView, Environment, View, view::TupleViews};

use crate::{
    Layout, Point, ProposalSize, Rect, Size, SubView,
    container::FixedContainer,
    stack::{Alignment, HorizontalAlignment, VerticalAlignment},
};

/// Cached measurement for a child during layout
struct ChildMeasurement {
    size: Size,
}

/// The core layout engine for a `Grid`.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub struct GridLayout {
    columns: NonZeroUsize,
    spacing: Size, // (horizontal, vertical)
    alignment: Alignment,
}

impl GridLayout {
    /// Creates a new `GridLayout` with the specified columns, spacing, and alignment.
    #[must_use]
    pub const fn new(columns: NonZeroUsize, spacing: Size, alignment: Alignment) -> Self {
        Self {
            columns,
            spacing,
            alignment,
        }
    }
}

#[allow(clippy::cast_precision_loss)]
impl Layout for GridLayout {
    fn size_that_fits(
        &self,
        proposal: ProposalSize,
        children: &mut [&mut dyn SubView],
    ) -> Size {
        if children.is_empty() {
            return Size::zero();
        }

        let num_columns = self.columns.get();
        let num_rows = children.len().div_ceil(num_columns);

        // Calculate the width available for each column.
        // A Grid requires a defined width from its parent to function correctly.
        let child_width = proposal.width.map(|w| {
            let total_spacing = self.spacing.width * (num_columns - 1) as f32;
            ((w - total_spacing) / num_columns as f32).max(0.0)
        });

        // Grids are vertically unconstrained during the proposal phase.
        // Each child is asked for its ideal height given the calculated column width.
        let child_proposal = ProposalSize::new(child_width, None);

        let measurements: Vec<ChildMeasurement> = children
            .iter_mut()
            .map(|child| ChildMeasurement {
                size: child.size_that_fits(child_proposal),
            })
            .collect();

        // The grid's height is the sum of the tallest item in each row, plus vertical spacing.
        let mut total_height = 0.0;
        for row_children in measurements.chunks(num_columns) {
            let row_height = row_children
                .iter()
                .map(|m| m.size.height)
                .filter(|h| h.is_finite())
                .fold(0.0, f32::max);
            total_height += row_height;
        }

        total_height += self.spacing.height * (num_rows.saturating_sub(1) as f32);

        // A Grid's width is defined by its parent. If not, it has no intrinsic width.
        let final_width = proposal.width.unwrap_or(0.0);

        Size::new(final_width, total_height)
    }

    fn place(
        &self,
        bounds: Rect,
        children: &mut [&mut dyn SubView],
    ) -> Vec<Rect> {
        if children.is_empty() || !bounds.width().is_finite() {
            // A grid cannot be placed in an infinitely wide space. Return zero-rects.
            return vec![
                Rect::new(Point::zero(), Size::zero());
                children.len()
            ];
        }

        let num_columns = self.columns.get();

        // Calculate column width
        let total_h_spacing = self.spacing.width * (num_columns - 1) as f32;
        let column_width = ((bounds.width() - total_h_spacing) / num_columns as f32).max(0.0);

        // Measure all children with the column width constraint
        let child_proposal = ProposalSize::new(Some(column_width), None);

        let measurements: Vec<ChildMeasurement> = children
            .iter_mut()
            .map(|child| ChildMeasurement {
                size: child.size_that_fits(child_proposal),
            })
            .collect();

        // Pre-calculate the height of each row by finding the tallest child in that row.
        let row_heights: Vec<f32> = measurements
            .chunks(num_columns)
            .map(|row_children| {
                row_children
                    .iter()
                    .map(|m| m.size.height)
                    .filter(|h| h.is_finite())
                    .fold(0.0, f32::max)
            })
            .collect();

        let mut placements = Vec::with_capacity(children.len());
        let mut cursor_y = bounds.y();

        for (row_index, row_measurements) in measurements.chunks(num_columns).enumerate() {
            let row_height = row_heights.get(row_index).copied().unwrap_or(0.0);
            let mut cursor_x = bounds.x();

            for measurement in row_measurements {
                let cell_frame = Rect::new(
                    Point::new(cursor_x, cursor_y),
                    Size::new(column_width, row_height),
                );

                // Handle infinite dimensions
                let child_width = if measurement.size.width.is_infinite() {
                    column_width
                } else {
                    measurement.size.width
                };

                let child_height = if measurement.size.height.is_infinite() {
                    row_height
                } else {
                    measurement.size.height
                };

                let child_size = Size::new(child_width, child_height);

                // Align the child within its cell
                let child_x = match self.alignment.horizontal() {
                    HorizontalAlignment::Leading => cell_frame.x(),
                    HorizontalAlignment::Center => {
                        cell_frame.x() + (cell_frame.width() - child_size.width) / 2.0
                    }
                    HorizontalAlignment::Trailing => cell_frame.max_x() - child_size.width,
                };

                let child_y = match self.alignment.vertical() {
                    VerticalAlignment::Top => cell_frame.y(),
                    VerticalAlignment::Center => {
                        cell_frame.y() + (cell_frame.height() - child_size.height) / 2.0
                    }
                    VerticalAlignment::Bottom => cell_frame.max_y() - child_size.height,
                };

                placements.push(Rect::new(Point::new(child_x, child_y), child_size));

                cursor_x += column_width + self.spacing.width;
            }

            cursor_y += row_height + self.spacing.height;
        }

        placements
    }
}

//=============================================================================
// 2. View DSL (Grid and GridRow)
//=============================================================================

/// A data-carrying struct that represents a single row in a `Grid`.
/// It does not implement `View` itself; it is consumed by the `Grid`.
#[derive(Debug)]
pub struct GridRow {
    pub(crate) contents: Vec<AnyView>,
}

impl GridRow {
    /// Creates a new `GridRow` with the given contents.
    pub fn new(contents: impl TupleViews) -> Self {
        Self {
            contents: contents.into_views(),
        }
    }
}

/// A view that arranges its `GridRow` children into a grid.
#[derive(Debug)]
pub struct Grid {
    layout: GridLayout,
    rows: Vec<GridRow>,
}

impl Grid {
    /// Creates a new Grid.
    ///
    /// - `columns`: The number of columns in the grid. Must be greater than 0.
    /// - `rows`: A tuple of `GridRow` views.
    ///
    /// # Panics
    ///
    /// Panics if `columns` is 0.
    pub fn new(columns: usize, rows: impl IntoIterator<Item = GridRow>) -> Self {
        Self {
            layout: GridLayout::new(
                NonZeroUsize::new(columns).expect("Grid columns must be greater than 0"),
                Size::new(8.0, 8.0), // Default spacing
                Alignment::Center,   // Default alignment
            ),
            rows: rows.into_iter().collect(),
        }
    }

    /// Sets the horizontal and vertical spacing for the grid.
    #[must_use]
    pub const fn spacing(mut self, spacing: f32) -> Self {
        self.layout.spacing = Size::new(spacing, spacing);
        self
    }

    /// Sets the alignment for children within their cells.
    #[must_use]
    pub const fn alignment(mut self, alignment: Alignment) -> Self {
        self.layout.alignment = alignment;
        self
    }
}

impl View for Grid {
    fn body(self, _env: &Environment) -> impl View {
        // Flatten the children from all GridRows into a single Vec<AnyView>.
        // This is the list that the GridLayout engine will operate on.
        let flattened_children = self
            .rows
            .into_iter()
            .flat_map(|row| row.contents)
            .collect::<Vec<AnyView>>();

        FixedContainer::new(self.layout, flattened_children)
    }
}

/// Creates a new grid with the specified number of columns and rows.
///
/// This is a convenience function that creates a `Grid` with default spacing and alignment.
///
/// # Panics
///
/// Panics if `columns` is 0.
pub fn grid(columns: usize, rows: impl IntoIterator<Item = GridRow>) -> Grid {
    Grid::new(columns, rows)
}

/// Creates a new grid row containing the specified views.
///
/// This is a convenience function for creating `GridRow` instances.
pub fn row(contents: impl TupleViews) -> GridRow {
    GridRow::new(contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::num::NonZeroUsize;

    struct MockSubView {
        size: Size,
    }

    impl SubView for MockSubView {
        fn size_that_fits(&mut self, _proposal: ProposalSize) -> Size {
            self.size
        }
        fn is_stretch(&self) -> bool {
            false
        }
        fn priority(&self) -> i32 {
            0
        }
    }

    #[test]
    fn test_grid_size_2x2() {
        let layout = GridLayout::new(
            NonZeroUsize::new(2).unwrap(),
            Size::new(10.0, 10.0),
            Alignment::Center,
        );

        let mut child1 = MockSubView { size: Size::new(50.0, 30.0) };
        let mut child2 = MockSubView { size: Size::new(50.0, 40.0) };
        let mut child3 = MockSubView { size: Size::new(50.0, 20.0) };
        let mut child4 = MockSubView { size: Size::new(50.0, 50.0) };

        let mut children: Vec<&mut dyn SubView> = vec![
            &mut child1, &mut child2,
            &mut child3, &mut child4,
        ];

        let size = layout.size_that_fits(
            ProposalSize::new(Some(200.0), None),
            &mut children,
        );

        // Width is parent-proposed
        assert_eq!(size.width, 200.0);
        // Height: row1 max(30, 40) + spacing + row2 max(20, 50) = 40 + 10 + 50 = 100
        assert_eq!(size.height, 100.0);
    }

    #[test]
    fn test_grid_placement() {
        let layout = GridLayout::new(
            NonZeroUsize::new(2).unwrap(),
            Size::new(10.0, 10.0),
            Alignment::TopLeading,
        );

        let mut child1 = MockSubView { size: Size::new(40.0, 30.0) };
        let mut child2 = MockSubView { size: Size::new(40.0, 30.0) };

        let mut children: Vec<&mut dyn SubView> = vec![&mut child1, &mut child2];

        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        let rects = layout.place(bounds, &mut children);

        // Column width: (100 - 10) / 2 = 45
        // Child 1 at (0, 0)
        assert_eq!(rects[0].x(), 0.0);
        assert_eq!(rects[0].y(), 0.0);

        // Child 2 at (45 + 10, 0) = (55, 0)
        assert_eq!(rects[1].x(), 55.0);
        assert_eq!(rects[1].y(), 0.0);
    }
}
