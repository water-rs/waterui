//! The retained scene: replayable draw commands.
//!
//! Dew never holds a rasterized copy of the screen, so the scene itself
//! must be replayable: any sub-region of the screen can be re-rasterized at
//! any time from these commands. The vocabulary is deliberately the one
//! shared with hydrolysis — [`kurbo`] geometry and [`peniko`] brushes — so
//! widget code translating `WaterUI` semantics into draws is portable
//! between the two engines.
//!
//! Clipping is retained per command: while a clip is active (see
//! [`DisplayList::push_clip`]), every pushed command records the effective
//! window-coordinate clip rectangle. Keeping the clip on the command rather
//! than as a structural layer preserves the flat, pairwise-diffable shape
//! of the list, so scroll viewports do not degrade dirty-region tracking.

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
        /// Window-coordinate clip rectangle, when one was active.
        clip: Option<Rect>,
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
        /// Window-coordinate clip rectangle, when one was active.
        clip: Option<Rect>,
    },
    /// Fills one shaped glyph run (after `transform`) with `brush`.
    GlyphRun {
        /// The font file backing the run.
        font: peniko::FontData,
        /// Font size in logical pixels.
        font_size: f32,
        /// Shaped glyphs positioned relative to `transform`.
        glyphs: Vec<vello_cpu::Glyph>,
        /// Local-to-window transform for the run.
        transform: Affine,
        /// Paint for the glyphs.
        brush: Brush,
        /// Pre-computed local bounds of the run (text layout box).
        bounds: Rect,
        /// Window-coordinate clip rectangle, when one was active.
        clip: Option<Rect>,
    },
}

impl DrawCommand {
    /// Window-coordinate bounding box of the pixels this command may touch.
    ///
    /// The box is intersected with the command's clip, so fully clipped
    /// commands report an empty (zero or negative area) rectangle.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        let raw = match self {
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
            Self::GlyphRun {
                transform, bounds, ..
            } => transform.transform_rect_bbox(*bounds),
        };
        self.clip().map_or(raw, |clip| raw.intersect(clip))
    }

    /// The window-coordinate clip rectangle recorded for this command.
    #[must_use]
    pub const fn clip(&self) -> Option<Rect> {
        match self {
            Self::FillPath { clip, .. }
            | Self::StrokePath { clip, .. }
            | Self::GlyphRun { clip, .. } => *clip,
        }
    }

    const fn clip_mut(&mut self) -> &mut Option<Rect> {
        match self {
            Self::FillPath { clip, .. }
            | Self::StrokePath { clip, .. }
            | Self::GlyphRun { clip, .. } => clip,
        }
    }
}

impl PartialEq for DrawCommand {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::FillPath {
                    path: p1,
                    transform: t1,
                    brush: b1,
                    clip: c1,
                },
                Self::FillPath {
                    path: p2,
                    transform: t2,
                    brush: b2,
                    clip: c2,
                },
            ) => p1 == p2 && t1 == t2 && b1 == b2 && c1 == c2,
            (
                Self::StrokePath {
                    path: p1,
                    transform: t1,
                    stroke: s1,
                    brush: b1,
                    clip: c1,
                },
                Self::StrokePath {
                    path: p2,
                    transform: t2,
                    stroke: s2,
                    brush: b2,
                    clip: c2,
                },
            ) => p1 == p2 && t1 == t2 && b1 == b2 && c1 == c2 && stroke_eq(s1, s2),
            (
                Self::GlyphRun {
                    font: f1,
                    font_size: s1,
                    glyphs: g1,
                    transform: t1,
                    brush: b1,
                    bounds: r1,
                    clip: c1,
                },
                Self::GlyphRun {
                    font: f2,
                    font_size: s2,
                    glyphs: g2,
                    transform: t2,
                    brush: b2,
                    bounds: r2,
                    clip: c2,
                },
            ) => {
                f1.data.id() == f2.data.id()
                    && f1.index == f2.index
                    && s1 == s2
                    && t1 == t2
                    && b1 == b2
                    && r1 == r2
                    && c1 == c2
                    && glyphs_eq(g1, g2)
            }
            _ => false,
        }
    }
}

fn stroke_eq(a: &Stroke, b: &Stroke) -> bool {
    a.width == b.width
        && a.join == b.join
        && a.miter_limit == b.miter_limit
        && a.start_cap == b.start_cap
        && a.end_cap == b.end_cap
        && a.dash_pattern == b.dash_pattern
        && a.dash_offset == b.dash_offset
}

fn glyphs_eq(a: &[vello_cpu::Glyph], b: &[vello_cpu::Glyph]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(g1, g2)| g1.id == g2.id && g1.x == g2.x && g1.y == g2.y)
}

/// An ordered list of [`DrawCommand`]s with a running bounds union.
///
/// This is the unit the painter replays into a region-sized scratch pixmap.
#[derive(Debug, Clone, Default)]
pub struct DisplayList {
    commands: Vec<DrawCommand>,
    bounds: Option<Rect>,
    clip_stack: Vec<Rect>,
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
            clip: None,
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
            clip: None,
        });
    }

    /// Appends a raw command, recording the active clip.
    ///
    /// When a clip is active, it is intersected with any clip the command
    /// already carries.
    pub fn push(&mut self, mut command: DrawCommand) {
        if let Some(active) = self.clip_stack.last().copied() {
            let clip = command.clip_mut();
            *clip = Some(clip.map_or(active, |own| own.intersect(active)));
        }
        let bounds = command.bounds();
        if bounds.width() > 0.0 && bounds.height() > 0.0 {
            self.bounds = Some(self.bounds.map_or(bounds, |current| current.union(bounds)));
        }
        self.commands.push(command);
    }

    /// Pushes a window-coordinate clip rectangle.
    ///
    /// Nested clips intersect; every command pushed until the matching
    /// [`DisplayList::pop_clip`] records the effective rectangle.
    pub fn push_clip(&mut self, clip: Rect) {
        let effective = self
            .clip_stack
            .last()
            .map_or(clip, |current| current.intersect(clip));
        self.clip_stack.push(effective);
    }

    /// Pops the innermost clip pushed by [`DisplayList::push_clip`].
    ///
    /// # Panics
    ///
    /// Panics when no clip is active — an unbalanced pop is a widget bug.
    pub fn pop_clip(&mut self) {
        self.clip_stack
            .pop()
            .expect("DisplayList::pop_clip without a matching push_clip");
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

    /// Removes all commands and resets the bounds and clip stack.
    pub fn clear(&mut self) {
        self.commands.clear();
        self.bounds = None;
        self.clip_stack.clear();
    }
}

/// Tolerance for flattening curves when converting shapes to Bézier paths;
/// well below one device pixel so the approximation is invisible.
pub(crate) const BEZIER_TOLERANCE: f64 = 0.05;

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

    #[test]
    fn active_clip_is_recorded_and_bounds_are_clipped() {
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(0.0, 0.0, 50.0, 50.0));
        list.fill(
            &Rect::new(40.0, 40.0, 80.0, 80.0),
            Affine::IDENTITY,
            Color::WHITE,
        );
        list.pop_clip();
        let command = &list.commands()[0];
        assert_eq!(command.clip(), Some(Rect::new(0.0, 0.0, 50.0, 50.0)));
        assert_eq!(command.bounds(), Rect::new(40.0, 40.0, 50.0, 50.0));
        assert_eq!(list.bounds(), Some(Rect::new(40.0, 40.0, 50.0, 50.0)));
    }

    #[test]
    fn nested_clips_intersect() {
        let mut list = DisplayList::new();
        list.push_clip(Rect::new(0.0, 0.0, 50.0, 50.0));
        list.push_clip(Rect::new(25.0, 25.0, 100.0, 100.0));
        list.fill(
            &Rect::new(0.0, 0.0, 200.0, 200.0),
            Affine::IDENTITY,
            Color::WHITE,
        );
        list.pop_clip();
        list.pop_clip();
        assert_eq!(
            list.commands()[0].clip(),
            Some(Rect::new(25.0, 25.0, 50.0, 50.0))
        );
    }

    #[test]
    fn clip_participates_in_command_equality() {
        let mut clipped = DisplayList::new();
        clipped.push_clip(Rect::new(0.0, 0.0, 50.0, 50.0));
        clipped.fill(
            &Rect::new(0.0, 0.0, 10.0, 10.0),
            Affine::IDENTITY,
            Color::WHITE,
        );
        clipped.pop_clip();
        let mut unclipped = DisplayList::new();
        unclipped.fill(
            &Rect::new(0.0, 0.0, 10.0, 10.0),
            Affine::IDENTITY,
            Color::WHITE,
        );
        assert_ne!(clipped.commands()[0], unclipped.commands()[0]);
    }
}
