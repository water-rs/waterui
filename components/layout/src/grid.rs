use crate::Alignment;
use crate::Size;
use crate::engine::{Constraint, Container, Layout, LayoutResult, MeasuredChild};
use alloc::vec::Vec;
use waterui_core::{AnyView, View, view::TupleViews};
#[derive(Debug, Default)]
#[must_use]
/// Represents a grid layout for arranging views in rows and columns.
pub struct Grid {
    /// The alignment of the grid within its parent container.
    pub alignment: Alignment,
    /// The horizontal space between columns in the grid.
    pub h_space: f64,
    /// The vertical space between rows in the grid.
    pub v_space: f64,
    /// The rows of the grid, each containing a set of columns.
    pub rows: Vec<GridRow>,
}

// Grid implements View manually

impl Grid {
    /// Creates a new `Grid` with the specified alignment and rows.
    pub fn new(rows: impl IntoIterator<Item = GridRow>) -> Self {
        Self {
            rows: rows.into_iter().collect(),
            ..Default::default()
        }
    }

    /// Sets the alignment of the grid within its container.
    pub const fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }

    /// Sets the grid alignment to center.
    pub const fn center(mut self) -> Self {
        self.alignment = Alignment::Center;
        self
    }

    /// Sets the grid alignment to leading (left in LTR, right in RTL).
    pub const fn leading(mut self) -> Self {
        self.alignment = Alignment::Leading;
        self
    }

    /// Sets the horizontal space between columns in the grid.
    pub fn h_space(mut self, space: impl Into<f64>) -> Self {
        self.h_space = space.into();
        self
    }

    /// Sets the vertical space between rows in the grid.
    pub fn v_space(mut self, space: impl Into<f64>) -> Self {
        self.v_space = space.into();
        self
    }
}
#[derive(Debug)]
/// Represents a row in the grid layout.
pub struct GridRow {
    /// The columns in this row.
    pub columns: Vec<AnyView>,
}

/// Creates a new `GridRow` from a tuple of views.
pub fn row(columns: impl TupleViews) -> GridRow {
    GridRow {
        columns: columns.into_views(),
    }
}

/// Creates a new grid layout with the specified rows.
///
/// Each row contains a collection of views that will be arranged
/// in a grid pattern with equal spacing.
pub fn grid(rows: impl IntoIterator<Item = GridRow>) -> Grid {
    Grid::new(rows)
}

// Note: Grid layout needs to be implemented differently since the Layout trait
// doesn't have access to self. For now, we'll implement a basic grid with
// fixed spacing that can be configured via the Container.
impl Layout for Grid {
    fn layout(self, constraint: Constraint, measured_children: &[MeasuredChild]) -> LayoutResult {
        // For simplicity, assume a square grid based on the number of children
        let child_count = measured_children.len();
        if child_count == 0 {
            return LayoutResult {
                size: Size {
                    width: 0.0,
                    height: 0.0,
                },
                child_positions: Vec::new(),
                child: AnyView::new(self),
            };
        }

        // Calculate approximate square grid dimensions
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let cols = (child_count as f64).sqrt().ceil() as usize;
        let rows = child_count.div_ceil(cols); // Ceiling division

        let spacing = 8.0; // Default spacing

        // Calculate column and row sizes
        #[allow(clippy::cast_precision_loss)]
        let available_width = constraint.max.width - (spacing * (cols - 1) as f64);
        #[allow(clippy::cast_precision_loss)]
        let available_height = constraint.max.height - (spacing * (rows - 1) as f64);

        #[allow(clippy::cast_precision_loss)]
        let col_width = available_width / cols as f64;
        #[allow(clippy::cast_precision_loss)]
        let row_height = available_height / rows as f64;

        let mut child_positions = Vec::new();

        for (index, child) in measured_children.iter().enumerate() {
            let row = index / cols;
            let col = index % cols;

            let child_width = child
                .ideal_size
                .width
                .min(col_width)
                .min(child.max_size.width)
                .max(child.min_size.width);

            let child_height = child
                .ideal_size
                .height
                .min(row_height)
                .min(child.max_size.height)
                .max(child.min_size.height);

            // Center within grid cell
            #[allow(clippy::cast_precision_loss)]
            let cell_x = col as f64 * (col_width + spacing);
            #[allow(clippy::cast_precision_loss)]
            let cell_y = row as f64 * (row_height + spacing);

            let child_x = cell_x + (col_width - child_width) / 2.0;
            let child_y = cell_y + (row_height - child_height) / 2.0;

            child_positions.push(Size {
                width: child_x,
                height: child_y,
            });
        }

        #[allow(clippy::cast_precision_loss, clippy::suboptimal_flops)]
        let total_width = cols as f64 * col_width + (cols - 1) as f64 * spacing;
        #[allow(clippy::cast_precision_loss, clippy::suboptimal_flops)]
        let total_height = rows as f64 * row_height + (rows - 1) as f64 * spacing;

        LayoutResult {
            size: Size {
                width: total_width
                    .min(constraint.max.width)
                    .max(constraint.min.width),
                height: total_height
                    .min(constraint.max.height)
                    .max(constraint.min.height),
            },
            child_positions,
            child: AnyView::new(self),
        }
    }
}

impl View for Grid {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Container::new(self)
    }
}
