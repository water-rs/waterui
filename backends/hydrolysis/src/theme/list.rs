//! The list look of the theme contract (water-rs/waterui#1434): what a theme
//! returns for a list ([`ListLook`]).
//!
//! Hydrolysis measures, places and hit-tests a list from the look's
//! [`ListMetrics`] and records its row, separator and lifted-row surfaces,
//! and the dismiss background revealed by a swipe, from the look's parts.
//! The move and delete controls are [`Chrome`]s, realized like every other
//! chrome. A swipe or a reorder drag moves the row's layer and the dismiss
//! background's one [`Driven`](super::Driven) value as live operands, so
//! neither records a row again; the look's [`ListGestures`] say how far and
//! how a row travels.

use std::rc::Rc;

#[cfg(hydrolysis_runtime)]
use kurbo::Rect;
use waterui_core::animation::Animation;
use waterui_core::layout::LayoutDirection;

#[cfg(hydrolysis_runtime)]
use super::look::record_at_rest;
use super::look::{Custom, CustomPart, DrivenSet};
use super::{Chrome, ListMetrics};
#[cfg(hydrolysis_runtime)]
use cherenkov_record::Recorder;

/// What Hydrolysis asks a theme's [`list`](super::WidgetTheme::list) look
/// for: everything structural about one list.
///
/// A spec change requests the look again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListSpec {
    direction: LayoutDirection,
}

impl ListSpec {
    /// The spec of a list laid out `direction`.
    #[must_use]
    pub const fn new(direction: LayoutDirection) -> Self {
        Self { direction }
    }

    /// The layout direction the list is laid out in.
    #[must_use]
    pub const fn direction(&self) -> LayoutDirection {
        self.direction
    }
}

/// How a row's swipe and reorder drag are recognized and settle.
#[derive(Debug, Clone, PartialEq)]
pub struct ListGestures {
    /// The fraction of a row's width a swipe must cross, on release, to
    /// dismiss the row.
    pub swipe_threshold: f64,
    /// The fraction of a slot a reorder drag must cross to step the lifted
    /// row over its neighbour: `0.5` steps at the slot's midpoint.
    pub reorder_threshold: f64,
    /// The distance, in points, the pointer travels before a row drag is
    /// recognized; a row can be tapped at all only because a press shorter
    /// than this is no drag.
    pub slop: f32,
    /// The animation a released swipe springs back with, and the rows a
    /// reorder drag displaces slide with.
    pub settle: Animation,
}

/// The swipe-to-dismiss background of a list row.
#[derive(Clone)]
struct DismissBackground {
    #[cfg_attr(
        not(hydrolysis_runtime),
        expect(
            dead_code,
            reason = "only the renderer realizes the dismiss background; a build without a painter compiles the theme contract alone"
        )
    )]
    part: Rc<dyn CustomPart<f64>>,
}

/// How a list looks and how its rows move.
///
/// The look holds the list's [`ListMetrics`], the parts that draw its rows,
/// its separators and the row a reorder drag lifts, the [`Chrome`] of each
/// control a row shows while the list is edited, and the [`ListGestures`]
/// of a row.
///
/// The row, separator and lifted-row parts draw in the widget's own space
/// and record once per frame they are listed in, at rest: they hold no live
/// state of their own, and a swipe or a reorder moves the whole row layer
/// they are part of. The dismiss background is the one live part: its
/// value is the swipe's progress.
#[derive(Clone)]
pub struct ListLook {
    metrics: ListMetrics,
    gestures: ListGestures,
    #[cfg_attr(
        not(hydrolysis_runtime),
        expect(
            dead_code,
            reason = "only the renderer records the list's rows; a build without a painter compiles the theme contract alone"
        )
    )]
    row: Rc<dyn CustomPart<()>>,
    #[cfg_attr(
        not(hydrolysis_runtime),
        expect(
            dead_code,
            reason = "only the renderer records the list's separators; a build without a painter compiles the theme contract alone"
        )
    )]
    separator: Rc<dyn CustomPart<()>>,
    alternate_row: Option<Rc<dyn CustomPart<()>>>,
    lifted_row: Option<Rc<dyn CustomPart<()>>>,
    dismiss: Option<DismissBackground>,
    move_up: Chrome,
    move_down: Chrome,
    delete: Chrome,
}

impl core::fmt::Debug for ListLook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ListLook")
            .field("metrics", &self.metrics)
            .field("gestures", &self.gestures)
            .field("alternate_row", &self.alternate_row.is_some())
            .field("lifted_row", &self.lifted_row.is_some())
            .field("dismiss", &self.dismiss.is_some())
            .finish_non_exhaustive()
    }
}

impl ListLook {
    /// A list measured by `metrics` whose rows `row` draws over each row's
    /// frame and whose `separator` draws over the line closing a row.
    /// `move_up` and `move_down` are the two halves of a row's move control
    /// and `delete` its delete control, each a chrome over its own rect.
    #[must_use]
    pub fn new<R, S>(
        metrics: ListMetrics,
        gestures: ListGestures,
        row: Custom<(), R>,
        separator: Custom<(), S>,
        controls: ListControls,
    ) -> Self
    where
        R: DrivenSet<()> + 'static,
        R::Live: 'static,
        S: DrivenSet<()> + 'static,
        S::Live: 'static,
    {
        let ListControls {
            move_up,
            move_down,
            delete,
        } = controls;
        Self {
            metrics,
            gestures,
            row: Rc::new(row),
            separator: Rc::new(separator),
            alternate_row: None,
            lifted_row: None,
            dismiss: None,
            move_up,
            move_down,
            delete,
        }
    }

    /// Draws every second row with `part` in place of the plain row part:
    /// zebra striping.
    #[must_use]
    pub fn alternate_row<D>(mut self, part: Custom<(), D>) -> Self
    where
        D: DrivenSet<()> + 'static,
        D::Live: 'static,
    {
        self.alternate_row = Some(Rc::new(part));
        self
    }

    /// Draws the row a reorder drag lifts with `part`, over the row's frame
    /// and beneath its content: the lifted treatment.
    #[must_use]
    pub fn lifted_row<D>(mut self, part: Custom<(), D>) -> Self
    where
        D: DrivenSet<()> + 'static,
        D::Live: 'static,
    {
        self.lifted_row = Some(Rc::new(part));
        self
    }

    /// Draws the background a swipe uncovers behind the row with `part`,
    /// over the row's slot.
    ///
    /// The part's value is the swipe's progress toward the dismiss
    /// threshold: `-1.0..=1.0`, negative while the row travels toward the
    /// physical left edge, so the part can anchor its icon on the side the
    /// row is uncovering. It follows the finger with no animation and
    /// springs back with the look's [`ListGestures::settle`].
    #[must_use]
    pub fn swipe_background<D>(mut self, part: Custom<f64, D>) -> Self
    where
        D: DrivenSet<f64> + 'static,
        D::Live: 'static,
    {
        self.dismiss = Some(DismissBackground {
            part: Rc::new(part),
        });
        self
    }

    /// The list's measures.
    #[must_use]
    pub const fn metrics(&self) -> ListMetrics {
        self.metrics
    }

    /// How a row's swipe and reorder drag are recognized and settle.
    #[must_use]
    pub const fn gestures(&self) -> &ListGestures {
        &self.gestures
    }

    /// The chrome of the move control's upper half.
    #[must_use]
    pub const fn move_up(&self) -> &Chrome {
        &self.move_up
    }

    /// The chrome of the move control's lower half.
    #[must_use]
    pub const fn move_down(&self) -> &Chrome {
        &self.move_down
    }

    /// The chrome of the delete control.
    #[must_use]
    pub const fn delete(&self) -> &Chrome {
        &self.delete
    }

    /// Whether the look draws a swipe's uncovered background.
    #[must_use]
    pub const fn has_swipe_background(&self) -> bool {
        self.dismiss.is_some()
    }

    /// Records the row at `index` over `frame`: the alternate part on every
    /// second row when the look has one.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn record_row(&self, recorder: &mut Recorder, frame: Rect, index: usize) {
        let part = match &self.alternate_row {
            Some(alternate) if index % 2 == 1 => alternate,
            _ => &self.row,
        };
        record_at_rest(&**part, recorder, frame);
    }

    /// Records the lifted treatment over `frame`, when the look has one.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn record_lifted_row(&self, recorder: &mut Recorder, frame: Rect) {
        if let Some(part) = &self.lifted_row {
            record_at_rest(&**part, recorder, frame);
        }
    }

    /// Records a separator over `frame`.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn record_separator(&self, recorder: &mut Recorder, frame: Rect) {
        record_at_rest(&*self.separator, recorder, frame);
    }

    /// The swipe's dismiss background part, when the look has one.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn dismiss_part(&self) -> Option<&Rc<dyn CustomPart<f64>>> {
        self.dismiss.as_ref().map(|dismiss| &dismiss.part)
    }
}

/// The three controls a row shows while its list is edited: the two halves
/// of the move control and the delete control, each a [`Chrome`] over its
/// own rect.
#[derive(Debug, Clone)]
pub struct ListControls {
    /// The move control's upper half: steps the row up.
    pub move_up: Chrome,
    /// The move control's lower half: steps the row down.
    pub move_down: Chrome,
    /// The delete control.
    pub delete: Chrome,
}
