//! A two-dimensional layout that arranges views in columns and rows.

use alloc::{vec, vec::Vec};
use core::num::NonZeroUsize;
use num_traits::ToPrimitive;
use waterui_core::{AnyView, Environment, IntoSignalF32, View, view::TupleViews};

use crate::{
    Layout, PlacedSubview, Point, ProposalSize, Rect, Size, SubView, ViewDimensions,
    container::FixedContainer,
    stack::{Alignment, HorizontalAlignment, VerticalAlignment},
};

/// Cached measurement for a child during layout
struct ChildMeasurement {
    dimensions: ViewDimensions,
}

struct GridMeasurement {
    measurements: Vec<ChildMeasurement>,
    column_widths: Vec<f32>,
    row_heights: Vec<f32>,
    total_width: f32,
    total_height: f32,
}

/// The core layout engine for a `Grid`.
#[derive(Debug, Clone, PartialEq)]
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

    fn measure_grid(
        &self,
        proposed_width: Option<f32>,
        children: &[&dyn SubView],
    ) -> GridMeasurement {
        let num_columns = self.columns.get();
        let num_rows = children.len().div_ceil(num_columns);

        let constrained_width = proposed_width
            .filter(|width| width.is_finite())
            .map(|width| width.max(0.0));

        let mut column_widths = constrained_width.map_or_else(
            || vec![0.0; num_columns],
            |width| {
                let total_spacing =
                    self.spacing.width * usize_to_f32(num_columns.saturating_sub(1));
                let width_per_column =
                    ((width - total_spacing) / usize_to_f32(num_columns)).max(0.0);
                vec![width_per_column; num_columns]
            },
        );

        if constrained_width.is_none() {
            for (index, child) in children.iter().enumerate() {
                let intrinsic_size = child.measure(ProposalSize::new(None, None)).size;
                if intrinsic_size.width.is_finite() {
                    let column_index = index % num_columns;
                    column_widths[column_index] =
                        column_widths[column_index].max(intrinsic_size.width.max(0.0));
                }
            }
        }

        let measurements: Vec<ChildMeasurement> = children
            .iter()
            .enumerate()
            .map(|(index, child)| {
                let column_index = index % num_columns;
                let child_proposal = ProposalSize::new(Some(column_widths[column_index]), None);
                ChildMeasurement {
                    dimensions: child.measure(child_proposal),
                }
            })
            .collect();

        let row_heights: Vec<f32> = measurements
            .chunks(num_columns)
            .map(|row_children| {
                row_children
                    .iter()
                    .map(|measurement| measurement.dimensions.size.height)
                    .filter(|height| height.is_finite())
                    .fold(0.0, f32::max)
            })
            .collect();

        let total_height = row_heights.iter().sum::<f32>()
            + self.spacing.height * usize_to_f32(num_rows.saturating_sub(1));

        let total_width = constrained_width.map_or_else(
            || {
                let used_columns = children.len().min(num_columns);
                let total_spacing =
                    self.spacing.width * usize_to_f32(used_columns.saturating_sub(1));
                column_widths.iter().take(used_columns).sum::<f32>() + total_spacing
            },
            |width| width.max(0.0),
        );

        GridMeasurement {
            measurements,
            column_widths,
            row_heights,
            total_width,
            total_height,
        }
    }
}

#[allow(clippy::cast_precision_loss)]
impl Layout for GridLayout {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        if children.is_empty() {
            return Size::zero();
        }

        let measurement = self.measure_grid(proposal.width, children);
        Size::new(measurement.total_width, measurement.total_height)
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        if children.is_empty() {
            return vec![];
        }

        let num_columns = self.columns.get();
        let measurement = self.measure_grid(Some(bounds.width()), children);

        let mut placements = Vec::with_capacity(children.len());
        let mut cursor_y = bounds.y();

        for (row_index, row_measurements) in
            measurement.measurements.chunks(num_columns).enumerate()
        {
            let row_height = measurement
                .row_heights
                .get(row_index)
                .copied()
                .unwrap_or(0.0);
            let mut cursor_x = bounds.x();

            for (column_index, child_measurement) in row_measurements.iter().enumerate() {
                let column_width = measurement
                    .column_widths
                    .get(column_index)
                    .copied()
                    .unwrap_or(0.0);
                let cell_frame = Rect::new(
                    Point::new(cursor_x, cursor_y),
                    Size::new(column_width, row_height),
                );

                // Handle infinite dimensions
                let child_width = if child_measurement.dimensions.size.width.is_infinite() {
                    column_width
                } else {
                    child_measurement.dimensions.size.width
                };

                let child_height = if child_measurement.dimensions.size.height.is_infinite() {
                    row_height
                } else {
                    child_measurement.dimensions.size.height
                };

                let child_size = Size::new(child_width, child_height);
                let mut adjusted_dimensions = child_measurement.dimensions.clone();
                adjusted_dimensions.size = child_size;

                // Align the child within its cell
                let horizontal = self.alignment.horizontal();
                let target_x = if horizontal == HorizontalAlignment::Leading {
                    0.0
                } else if horizontal == HorizontalAlignment::Trailing {
                    cell_frame.width()
                } else if horizontal == HorizontalAlignment::Center {
                    cell_frame.width() * 0.5
                } else {
                    adjusted_dimensions
                        .horizontal(horizontal)
                        .clamp(0.0, child_size.width)
                };
                let child_x = cell_frame.x() + target_x
                    - adjusted_dimensions
                        .horizontal(horizontal)
                        .clamp(0.0, child_size.width);

                let vertical = self.alignment.vertical();
                let target_y = if vertical == VerticalAlignment::Top {
                    0.0
                } else if vertical == VerticalAlignment::Bottom {
                    cell_frame.height()
                } else if vertical == VerticalAlignment::Center {
                    cell_frame.height() * 0.5
                } else {
                    adjusted_dimensions
                        .vertical(vertical)
                        .clamp(0.0, child_size.height)
                };
                let child_y = cell_frame.y() + target_y
                    - adjusted_dimensions
                        .vertical(vertical)
                        .clamp(0.0, child_size.height);

                placements.push(Rect::new(Point::new(child_x, child_y), child_size));

                cursor_x += column_width + self.spacing.width;
            }

            cursor_y += row_height + self.spacing.height;
        }

        placements
    }

    fn explicit_horizontal(
        &self,
        alignment: HorizontalAlignment,
        _bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        if alignment == self.alignment.horizontal() {
            return children
                .iter()
                .filter_map(|child| child.explicit_horizontal(alignment))
                .min_by(f32::total_cmp);
        }
        None
    }

    fn explicit_vertical(
        &self,
        alignment: VerticalAlignment,
        _bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        if alignment == VerticalAlignment::LastBaseline {
            return children
                .iter()
                .filter_map(|child| child.explicit_vertical(alignment))
                .max_by(f32::total_cmp);
        }
        if alignment == VerticalAlignment::FirstBaseline || alignment == self.alignment.vertical() {
            return children
                .iter()
                .filter_map(|child| child.explicit_vertical(alignment))
                .min_by(f32::total_cmp);
        }
        None
    }
}

fn usize_to_f32(value: usize) -> f32 {
    value
        .to_f32()
        .expect("GridLayout: index must be representable as f32")
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

/// A view that arranges content in rows and columns.
///
/// ![Grid](https://raw.githubusercontent.com/water-rs/waterui/dev/docs/illustrations/grid.svg)
///
/// Use a `Grid` to create a two-dimensional layout. You define the number of columns,
/// and the grid automatically arranges your content into rows.
///
/// ```ignore
/// grid(2, [
///     row((text("A1"), text("A2"))),
///     row((text("B1"), text("B2"))),
/// ])
/// ```
///
/// Customize spacing and alignment:
///
/// ```ignore
/// Grid::new(3, rows)
///     .spacing(16.0)
///     .alignment(Alignment::Leading)
/// ```
///
/// The grid sizes columns equally based on available width, and row heights
/// are determined by the tallest item in each row.
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
    ///
    /// Accepts any numeric literal or signal of f32; snapshotted at modifier
    /// time per `WaterUI`'s Vue-like reactivity model.
    #[must_use]
    pub fn spacing(mut self, spacing: impl IntoSignalF32 + 'static) -> Self {
        use waterui_core::Signal;
        let spacing = spacing.into_signal_f32().get();
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
    use crate::StretchAxis;
    use core::num::NonZeroUsize;

    struct MockSubView {
        size: Size,
    }

    impl SubView for MockSubView {
        fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
            ViewDimensions::new(self.size)
        }
        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
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

        let mut child1 = MockSubView {
            size: Size::new(50.0, 30.0),
        };
        let mut child2 = MockSubView {
            size: Size::new(50.0, 40.0),
        };
        let mut child3 = MockSubView {
            size: Size::new(50.0, 20.0),
        };
        let mut child4 = MockSubView {
            size: Size::new(50.0, 50.0),
        };

        let children: Vec<&dyn SubView> = vec![&mut child1, &mut child2, &mut child3, &mut child4];

        let size = layout.size_that_fits(ProposalSize::new(Some(200.0), None), &children);

        // Width is parent-proposed
        assert!((size.width - 200.0).abs() < f32::EPSILON);
        // Height: row1 max(30, 40) + spacing + row2 max(20, 50) = 40 + 10 + 50 = 100
        assert!((size.height - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_grid_placement() {
        let layout = GridLayout::new(
            NonZeroUsize::new(2).unwrap(),
            Size::new(10.0, 10.0),
            Alignment::TopLeading,
        );

        let mut child1 = MockSubView {
            size: Size::new(40.0, 30.0),
        };
        let mut child2 = MockSubView {
            size: Size::new(40.0, 30.0),
        };

        let children: Vec<&dyn SubView> = vec![&mut child1, &mut child2];

        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        let rects = layout.place(bounds, &children);

        // Column width: (100 - 10) / 2 = 45
        // Child 1 at (0, 0)
        assert!((rects[0].x() - 0.0).abs() < f32::EPSILON);
        assert!((rects[0].y() - 0.0).abs() < f32::EPSILON);

        // Child 2 at (45 + 10, 0) = (55, 0)
        assert!((rects[1].x() - 55.0).abs() < f32::EPSILON);
        assert!((rects[1].y() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_grid_intrinsic_width_when_unconstrained() {
        let layout = GridLayout::new(
            NonZeroUsize::new(2).unwrap(),
            Size::new(10.0, 10.0),
            Alignment::Center,
        );

        let mut child1 = MockSubView {
            size: Size::new(30.0, 20.0),
        };
        let mut child2 = MockSubView {
            size: Size::new(50.0, 25.0),
        };
        let mut child3 = MockSubView {
            size: Size::new(40.0, 35.0),
        };
        let mut child4 = MockSubView {
            size: Size::new(20.0, 30.0),
        };

        let children: Vec<&dyn SubView> = vec![&mut child1, &mut child2, &mut child3, &mut child4];

        let size = layout.size_that_fits(ProposalSize::new(None, None), &children);

        // Column 0 max width: max(30, 40) = 40
        // Column 1 max width: max(50, 20) = 50
        // Total width = 40 + 10 + 50 = 100
        assert!((size.width - 100.0).abs() < f32::EPSILON);
        // Row 0 max height = 25, row 1 max height = 35, plus one vertical spacing (10)
        assert!((size.height - 70.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_grid_placement_with_infinite_bounds_uses_intrinsic_width() {
        let layout = GridLayout::new(
            NonZeroUsize::new(2).unwrap(),
            Size::new(10.0, 10.0),
            Alignment::TopLeading,
        );

        let mut child1 = MockSubView {
            size: Size::new(40.0, 20.0),
        };
        let mut child2 = MockSubView {
            size: Size::new(20.0, 20.0),
        };

        let children: Vec<&dyn SubView> = vec![&mut child1, &mut child2];

        let bounds = Rect::new(Point::new(5.0, 7.0), Size::new(f32::INFINITY, 100.0));
        let rects = layout.place(bounds, &children);

        assert_eq!(rects.len(), 2);
        assert!((rects[0].x() - 5.0).abs() < f32::EPSILON);
        assert!((rects[0].y() - 7.0).abs() < f32::EPSILON);
        assert!((rects[0].width() - 40.0).abs() < f32::EPSILON);
        assert!((rects[1].x() - 55.0).abs() < f32::EPSILON);
        assert!((rects[1].y() - 7.0).abs() < f32::EPSILON);
        assert!((rects[1].width() - 20.0).abs() < f32::EPSILON);
    }
}
