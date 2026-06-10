//! The retained scene: replayable draw commands.
//!
//! Dew never holds a rasterized copy of the screen, so the scene itself
//! must be replayable: any sub-region of the screen can be re-rasterized at
//! any time from these commands. The vocabulary is deliberately the one
//! shared with hydrolysis — [`kurbo`] geometry and [`peniko`] brushes — so
//! widget code translating `WaterUI` semantics into draws is portable
//! between the two engines.

use kurbo::{Affine, BezPath, Rect, Shape, Stroke};
use peniko::Brush;

/// One retained draw operation, in window coordinates.
#[derive(Debug, Clone)]
pub enum DrawCommand {
    /// Fills `path` (after `transform`) with `brush`.
    FillPath {
        /// Geometry in local coordinates.
        path: BezPath,
        /// Local-to-window transform applied to `path`.
        transform: Affine,
        /// Paint for the fill.
        brush: Brush,
    },
    /// Strokes `path` (after `transform`) with `brush`.
    StrokePath {
        /// Geometry in local coordinates.
        path: BezPath,
        /// Local-to-window transform applied to `path`.
        transform: Affine,
        /// Stroke geometry (width, joins, caps, dashes).
        stroke: Stroke,
        /// Paint for the stroke.
        brush: Brush,
    },
}

impl DrawCommand {
    /// Window-coordinate bounding box of the pixels this command may touch.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        match self {
            Self::FillPath {
                path, transform, ..
            } => transform.transform_rect_bbox(path.bounding_box()),
            Self::StrokePath {
                path,
                transform,
                stroke,
                ..
            } => transform
                .transform_rect_bbox(path.bounding_box())
                .inflate(stroke.width / 2.0, stroke.width / 2.0),
        }
    }
}

/// An ordered list of [`DrawCommand`]s with a running bounds union.
///
/// This is the unit the painter replays into a region-sized scratch pixmap.
#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    commands: Vec<DrawCommand>,
    bounds: Option<Rect>,
}

impl DisplayList {
    /// Creates an empty list.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a filled shape.
    pub fn fill(&mut self, shape: &impl Shape, transform: Affine, brush: impl Into<Brush>) {
        self.push(DrawCommand::FillPath {
            path: shape.to_path(BEZIER_TOLERANCE),
            transform,
            brush: brush.into(),
        });
    }

    /// Appends a stroked shape.
    pub fn stroke(
        &mut self,
        shape: &impl Shape,
        transform: Affine,
        stroke: Stroke,
        brush: impl Into<Brush>,
    ) {
        self.push(DrawCommand::StrokePath {
            path: shape.to_path(BEZIER_TOLERANCE),
            transform,
            stroke,
            brush: brush.into(),
        });
    }

    /// Appends a raw command.
    pub fn push(&mut self, command: DrawCommand) {
        let bounds = command.bounds();
        self.bounds = Some(self.bounds.map_or(bounds, |current| current.union(bounds)));
        self.commands.push(command);
    }

    /// The retained commands in draw order.
    #[must_use]
    pub fn commands(&self) -> &[DrawCommand] {
        &self.commands
    }

    /// Window-coordinate bounds of everything in the list, or [`None`] when
    /// empty.
    #[must_use]
    pub const fn bounds(&self) -> Option<Rect> {
        self.bounds
    }

    /// Whether the list contains no commands.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Removes all commands and resets the bounds.
    pub fn clear(&mut self) {
        self.commands.clear();
        self.bounds = None;
    }
}

/// Tolerance for flattening curves when converting shapes to Bézier paths;
/// well below one device pixel so the approximation is invisible.
const BEZIER_TOLERANCE: f64 = 0.05;

#[cfg(test)]
mod tests {
    use super::*;
    use peniko::Color;

    #[test]
    fn bounds_accumulate_across_commands() {
        let mut list = DisplayList::new();
        assert!(list.bounds().is_none());
        list.fill(
            &Rect::new(10.0, 10.0, 20.0, 20.0),
            Affine::IDENTITY,
            Color::WHITE,
        );
        list.fill(
            &Rect::new(50.0, 5.0, 60.0, 15.0),
            Affine::IDENTITY,
            Color::WHITE,
        );
        assert_eq!(list.bounds(), Some(Rect::new(10.0, 5.0, 60.0, 20.0)));
    }

    #[test]
    fn stroke_bounds_include_stroke_width() {
        let mut list = DisplayList::new();
        list.stroke(
            &Rect::new(10.0, 10.0, 20.0, 20.0),
            Affine::IDENTITY,
            Stroke::new(4.0),
            Color::WHITE,
        );
        assert_eq!(list.bounds(), Some(Rect::new(8.0, 8.0, 22.0, 22.0)));
    }

    #[test]
    fn transform_moves_command_bounds() {
        let mut list = DisplayList::new();
        list.fill(
            &Rect::new(0.0, 0.0, 10.0, 10.0),
            Affine::translate((100.0, 200.0)),
            Color::WHITE,
        );
        assert_eq!(list.bounds(), Some(Rect::new(100.0, 200.0, 110.0, 210.0)));
    }
}
