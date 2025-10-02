//! A two-dimensional layout that arranges views in columns and rows.

use alloc::{vec, vec::Vec};
use core::num::NonZeroUsize;
use waterui_core::{AnyView, Environment, View, view::TupleViews};

use crate::{
    ChildMetadata, Container, Layout, Point, ProposalSize, Rect, Size,
    stack::{Alignment, HorizontalAlignment, VerticalAlignment},
};

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
    /// Propose a size to each child cell.
    fn propose(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Vec<ProposalSize> {
        if children.is_empty() {
            return vec![];
        }

        // Calculate the width available for each column.
        // A Grid requires a defined width from its parent to function correctly.
        let child_width = parent.width.map(|w| {
            let total_spacing = self.spacing.width * (self.columns.get() - 1) as f64;
            ((w - total_spacing) / self.columns.get() as f64).max(0.0)
        });

        // Grids are vertically unconstrained during the proposal phase.
        // Each child is asked for its ideal height given the calculated column width.
        let child_proposal = ProposalSize::new(child_width, None);
        vec![child_proposal; children.len()]
    }

    /// Calculate the total size required by the grid.
    fn size(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Size {
        if children.is_empty() {
            return Size::zero();
        }

        let num_columns = self.columns.get();
        let num_rows = children.len().div_ceil(num_columns);

        // The grid's height is the sum of the tallest item in each row, plus vertical spacing.
        let mut total_height = 0.0;
        for row_children in children.chunks(num_columns) {
            let row_height = row_children
                .iter()
                .map(|c| c.proposal_height().unwrap_or(0.0))
                .fold(0.0, f64::max); // Find the max height in the row
            total_height += row_height;
        }

        total_height += self.spacing.height * (num_rows.saturating_sub(1) as f64);

        // A Grid's width is defined by its parent. If not, it has no intrinsic width.
        let final_width = parent.width.unwrap_or(0.0);

        Size::new(final_width, total_height)
    }

    /// Place each child within its calculated cell in the grid.
    fn place(
        &mut self,
        bound: Rect,
        _proposal: ProposalSize,
        children: &[ChildMetadata],
    ) -> Vec<Rect> {
        if children.is_empty() || !bound.width().is_finite() {
            // A grid cannot be placed in an infinitely wide space. Return zero-rects.
            return vec![Rect::new(Point::zero(), Size::zero()); children.len()];
        }

        let num_columns = self.columns.get();

        // Pre-calculate the height of each row by finding the tallest child in that row.
        let row_heights: Vec<f64> = children
            .chunks(num_columns)
            .map(|row_children| {
                row_children
                    .iter()
                    .map(|c| c.proposal_height().unwrap_or(0.0))
                    .fold(0.0, f64::max)
            })
            .collect();

        let total_h_spacing = self.spacing.width * (num_columns - 1) as f64;
        let column_width = ((bound.width() - total_h_spacing) / num_columns as f64).max(0.0);

        let mut final_rects = Vec::with_capacity(children.len());
        let mut cursor_y = bound.y();

        for (row_index, row_children) in children.chunks(num_columns).enumerate() {
            let row_height = row_heights.get(row_index).copied().unwrap_or(0.0);
            let mut cursor_x = bound.x();

            for child in row_children {
                let cell_frame = Rect::new(
                    Point::new(cursor_x, cursor_y),
                    Size::new(column_width, row_height),
                );

                let child_size = Size::new(
                    child.proposal_width().unwrap_or(0.0),
                    child.proposal_height().unwrap_or(0.0),
                );

                // Align the child within its cell (same logic as Frame layout)
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

                final_rects.push(Rect::new(Point::new(child_x, child_y), child_size));

                cursor_x += column_width + self.spacing.width;
            }

            cursor_y += row_height + self.spacing.height;
        }

        final_rects
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
    pub const fn spacing(mut self, spacing: f64) -> Self {
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

        Container::new(self.layout, flattened_children)
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
