//! The table look of the theme contract (water-rs/waterui#1434): what a
//! theme returns for a table ([`TableLook`]).
//!
//! Hydrolysis measures and places a table from the look's [`TableMetrics`]
//! and records its background, header background, cell borders and column
//! separators from the look's parts, so the geometry it measures is the
//! geometry it draws.

use std::rc::Rc;

#[cfg(hydrolysis_runtime)]
use cherenkov_record::Recorder;
#[cfg(hydrolysis_runtime)]
use kurbo::Rect;

use super::TableMetrics;
#[cfg(hydrolysis_runtime)]
use super::look::record_at_rest;
use super::look::{Custom, CustomPart, DrivenSet};

/// How a table looks: its [`TableMetrics`] and the parts that draw its
/// background, its header, its cell borders and its column separators.
///
/// Every part draws in the widget's own space and records once per frame it
/// is listed in, at rest: a table has no live state of its own. A column
/// separator draws over the zero-width rect between its two ends.
#[derive(Clone)]
pub struct TableLook {
    metrics: TableMetrics,
    #[cfg_attr(
        not(hydrolysis_runtime),
        expect(
            dead_code,
            reason = "only the renderer records the table's parts; a build without a painter compiles the theme contract alone"
        )
    )]
    parts: Rc<TableParts>,
}

#[cfg_attr(
    not(hydrolysis_runtime),
    expect(
        dead_code,
        reason = "only the renderer records the table's parts; a build without a painter compiles the theme contract alone"
    )
)]
struct TableParts {
    background: Rc<dyn CustomPart<()>>,
    header_background: Rc<dyn CustomPart<()>>,
    cell_border: Rc<dyn CustomPart<()>>,
    column_separator: Rc<dyn CustomPart<()>>,
}

impl core::fmt::Debug for TableLook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TableLook")
            .field("metrics", &self.metrics)
            .finish_non_exhaustive()
    }
}

impl TableLook {
    /// A table measured by `metrics`, whose `background` draws over the
    /// table's frame, `header_background` over the header row, `cell_border`
    /// over each cell and `column_separator` over the line between two
    /// columns.
    #[must_use]
    pub fn new<B, H, C, S>(
        metrics: TableMetrics,
        background: Custom<(), B>,
        header_background: Custom<(), H>,
        cell_border: Custom<(), C>,
        column_separator: Custom<(), S>,
    ) -> Self
    where
        B: DrivenSet<()> + 'static,
        B::Live: 'static,
        H: DrivenSet<()> + 'static,
        H::Live: 'static,
        C: DrivenSet<()> + 'static,
        C::Live: 'static,
        S: DrivenSet<()> + 'static,
        S::Live: 'static,
    {
        Self {
            metrics,
            parts: Rc::new(TableParts {
                background: Rc::new(background),
                header_background: Rc::new(header_background),
                cell_border: Rc::new(cell_border),
                column_separator: Rc::new(column_separator),
            }),
        }
    }

    /// The table's measures.
    #[must_use]
    pub const fn metrics(&self) -> TableMetrics {
        self.metrics
    }

    /// Records the table's background over `frame`.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn record_background(&self, recorder: &mut Recorder, frame: Rect) {
        record_at_rest(&*self.parts.background, recorder, frame);
    }

    /// Records the header background over `frame`.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn record_header_background(&self, recorder: &mut Recorder, frame: Rect) {
        record_at_rest(&*self.parts.header_background, recorder, frame);
    }

    /// Records a cell's border over `frame`.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn record_cell_border(&self, recorder: &mut Recorder, frame: Rect) {
        record_at_rest(&*self.parts.cell_border, recorder, frame);
    }

    /// Records a column separator over the zero-width `frame` between its
    /// two ends.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn record_column_separator(&self, recorder: &mut Recorder, frame: Rect) {
        record_at_rest(&*self.parts.column_separator, recorder, frame);
    }
}
