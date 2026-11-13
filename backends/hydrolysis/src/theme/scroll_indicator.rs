//! The scroll indicator look of the theme contract (water-rs/waterui#1434):
//! what a theme returns for the thumb a scrolling surface draws
//! ([`ScrollIndicatorLook`]).
//!
//! Hydrolysis sizes and places the thumb on its track from the look's
//! geometry and records it from the look's part, so the thumb it hit-tests
//! is the thumb it draws.

use std::rc::Rc;

#[cfg(hydrolysis_runtime)]
use cherenkov_record::Recorder;
#[cfg(hydrolysis_runtime)]
use kurbo::Rect;

#[cfg(hydrolysis_runtime)]
use super::look::record_at_rest;
use super::look::{Custom, CustomPart, DrivenSet};

/// How a scroll indicator looks.
///
/// The look holds the thumb's thickness at rest and while it is dragged, how
/// it is inset from the surface's edge, the shortest the thumb gets, the
/// width of the gutter that takes a drag, and the part that draws the thumb.
///
/// The thumb part draws over the thumb's rect, vertical or horizontal, and
/// records once per frame it is listed in, at rest: the thumb's size and
/// place follow the scroll offset, and its drag widening is a frame the
/// part draws over.
#[derive(Clone)]
pub struct ScrollIndicatorLook {
    thickness: f64,
    drag_thickness: f64,
    edge_inset: f64,
    min_extent: f64,
    gutter: f64,
    #[cfg_attr(
        not(hydrolysis_runtime),
        expect(
            dead_code,
            reason = "only the renderer records the thumb; a build without a painter compiles the theme contract alone"
        )
    )]
    thumb: Rc<dyn CustomPart<()>>,
}

impl core::fmt::Debug for ScrollIndicatorLook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ScrollIndicatorLook")
            .field("thickness", &self.thickness)
            .field("drag_thickness", &self.drag_thickness)
            .field("edge_inset", &self.edge_inset)
            .field("min_extent", &self.min_extent)
            .field("gutter", &self.gutter)
            .finish_non_exhaustive()
    }
}

impl ScrollIndicatorLook {
    /// A thumb `thickness` thick at rest and `drag_thickness` while it is
    /// dragged, drawn by `thumb`, with no inset, no minimum extent and no
    /// gutter.
    #[must_use]
    pub fn new<D>(thickness: f64, drag_thickness: f64, thumb: Custom<(), D>) -> Self
    where
        D: DrivenSet<()> + 'static,
        D::Live: 'static,
    {
        Self {
            thickness,
            drag_thickness,
            edge_inset: 0.0,
            min_extent: 0.0,
            gutter: 0.0,
            thumb: Rc::new(thumb),
        }
    }

    /// Insets the thumb `inset` from the surface's edge.
    #[must_use]
    pub const fn edge_inset(mut self, inset: f64) -> Self {
        self.edge_inset = inset;
        self
    }

    /// Holds the thumb at least `extent` long along its track.
    #[must_use]
    pub const fn min_extent(mut self, extent: f64) -> Self {
        self.min_extent = extent;
        self
    }

    /// Takes a drag in a gutter `width` wide along the surface's edge.
    #[must_use]
    pub const fn gutter(mut self, width: f64) -> Self {
        self.gutter = width;
        self
    }

    /// The thumb's thickness at rest.
    #[must_use]
    pub const fn get_thickness(&self) -> f64 {
        self.thickness
    }

    /// The thumb's thickness while it is dragged.
    #[must_use]
    pub const fn get_drag_thickness(&self) -> f64 {
        self.drag_thickness
    }

    /// The thumb's inset from the surface's edge.
    #[must_use]
    pub const fn get_edge_inset(&self) -> f64 {
        self.edge_inset
    }

    /// The shortest the thumb gets along its track.
    #[must_use]
    pub const fn get_min_extent(&self) -> f64 {
        self.min_extent
    }

    /// The width of the gutter that takes a drag.
    #[must_use]
    pub const fn get_gutter(&self) -> f64 {
        self.gutter
    }

    /// Records the thumb over `frame`.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn record_thumb(&self, recorder: &mut Recorder, frame: Rect) {
        record_at_rest(&*self.thumb, recorder, frame);
    }
}
