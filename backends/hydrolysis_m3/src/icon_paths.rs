//! Material icon outlines, drawn as real paths rather than approximated with
//! boxes.
//!
//! Every Material icon is authored on a 24dp grid. [`IconGrid`] maps that grid
//! onto a destination square so an icon keeps its proportions at any size, and
//! the path data below is the same geometry Compose ships in
//! `androidx.compose.material.icons`.

use vello::kurbo::{BezPath, Point, Rect};

/// The grid every Material icon is authored on.
const ICON_GRID: f64 = 24.0;

/// Maps the 24dp icon grid onto a concrete square on screen.
#[derive(Clone, Copy)]
pub struct IconGrid {
    scale: f64,
    origin_x: f64,
    origin_y: f64,
}

impl IconGrid {
    /// Fits the icon grid into `bounds`, centered, preserving proportions.
    #[must_use]
    pub fn fit(bounds: Rect) -> Self {
        let side = bounds.width().min(bounds.height()).max(0.0);
        let scale = side / ICON_GRID;
        Self {
            scale,
            origin_x: bounds.x0 + (bounds.width() - side) / 2.0,
            origin_y: bounds.y0 + (bounds.height() - side) / 2.0,
        }
    }

    /// Centers a `size`-wide icon grid on `center`.
    #[must_use]
    pub fn centered(center: Point, size: f64) -> Self {
        Self::fit(Rect::new(
            center.x - size / 2.0,
            center.y - size / 2.0,
            center.x + size / 2.0,
            center.y + size / 2.0,
        ))
    }

    const fn point(self, x: f64, y: f64) -> Point {
        Point::new(
            x.mul_add(self.scale, self.origin_x),
            y.mul_add(self.scale, self.origin_y),
        )
    }
}

/// Material `drag_handle`: the two stacked bars used as a reorder grip.
#[must_use]
pub fn drag_handle(grid: IconGrid) -> BezPath {
    let mut path = BezPath::new();
    for (top, bottom) in [(9.0, 11.0), (13.0, 15.0)] {
        path.move_to(grid.point(4.0, top));
        path.line_to(grid.point(20.0, top));
        path.line_to(grid.point(20.0, bottom));
        path.line_to(grid.point(4.0, bottom));
        path.close_path();
    }
    path
}

/// Material `delete`: the filled waste-basket, body plus lid.
#[must_use]
pub fn delete(grid: IconGrid) -> BezPath {
    let mut path = BezPath::new();

    // Basket: straight sides with rounded lower corners.
    path.move_to(grid.point(6.0, 19.0));
    path.curve_to(
        grid.point(6.0, 20.1),
        grid.point(6.9, 21.0),
        grid.point(8.0, 21.0),
    );
    path.line_to(grid.point(16.0, 21.0));
    path.curve_to(
        grid.point(17.1, 21.0),
        grid.point(18.0, 20.1),
        grid.point(18.0, 19.0),
    );
    path.line_to(grid.point(18.0, 7.0));
    path.line_to(grid.point(6.0, 7.0));
    path.close_path();

    // Lid, with the raised handle notched into its top edge.
    path.move_to(grid.point(19.0, 4.0));
    path.line_to(grid.point(15.5, 4.0));
    path.line_to(grid.point(14.5, 3.0));
    path.line_to(grid.point(9.5, 3.0));
    path.line_to(grid.point(8.5, 4.0));
    path.line_to(grid.point(5.0, 4.0));
    path.line_to(grid.point(5.0, 6.0));
    path.line_to(grid.point(19.0, 6.0));
    path.close_path();

    path
}

/// Material `keyboard_arrow_down` on the [`ICON_GRID`], as the corners of the
/// filled chevron. Shared so a consumer that needs unit-square path commands
/// rather than a `BezPath` describes the same shape.
pub const CHEVRON_DOWN_OUTLINE: [(f64, f64); 6] = [
    (7.41, 8.59),
    (12.0, 13.17),
    (16.59, 8.59),
    (18.0, 10.0),
    (12.0, 16.0),
    (6.0, 10.0),
];

/// The grid every Material icon is authored on, exposed so callers can
/// normalize [`CHEVRON_DOWN_OUTLINE`] themselves.
#[must_use]
pub const fn icon_grid_size() -> f64 {
    ICON_GRID
}

#[cfg(test)]
mod tests {
    use super::{IconGrid, delete, drag_handle};
    use vello::kurbo::{Rect, Shape as _};

    /// Both icons must stay inside the box they are fitted to, or they would
    /// bleed into neighbouring row content.
    #[test]
    fn icons_stay_within_their_fitted_box() {
        let bounds = Rect::new(10.0, 20.0, 34.0, 44.0);
        let grid = IconGrid::fit(bounds);
        for path in [drag_handle(grid), delete(grid)] {
            let box_ = path.bounding_box();
            assert!(
                box_.x0 >= bounds.x0 - 0.01
                    && box_.y0 >= bounds.y0 - 0.01
                    && box_.x1 <= bounds.x1 + 0.01
                    && box_.y1 <= bounds.y1 + 0.01,
                "icon {box_:?} escaped its {bounds:?} box"
            );
        }
    }

    /// A non-square box must not stretch an icon: it stays square and centered.
    #[test]
    fn icons_keep_their_proportions_in_a_wide_box() {
        let grid = IconGrid::fit(Rect::new(0.0, 0.0, 100.0, 24.0));
        let box_ = drag_handle(grid).bounding_box();
        // The handle spans 4..20 of the 24 grid, so 16/24 of a 24pt square.
        assert!((box_.width() - 16.0).abs() < 0.01, "width was {box_:?}");
        assert!(
            (box_.center().x - 50.0).abs() < 0.01,
            "not centered: {box_:?}"
        );
    }
}
