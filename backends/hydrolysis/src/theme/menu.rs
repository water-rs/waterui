//! The menu look of the theme contract (water-rs/waterui#1434): what a theme
//! returns for every menu presentation ([`MenuLook`]).
//!
//! One look serves the picker popup, the text context menu and the popup
//! window a menu button opens, so they are one surface: the same row
//! measures, the same panel and the same separators. Hydrolysis measures,
//! places and hit-tests a menu from [`MenuLayout`] and records the panel and
//! separators the look's parts draw, so the geometry it measures is the
//! geometry it draws and the accessibility option rectangles are the drawn
//! ones.

use std::rc::Rc;

use kurbo::Rect;
use waterui_core::layout::LayoutDirection;

#[cfg(hydrolysis_runtime)]
use super::look::Inputs;
use super::look::{Custom, CustomPart, DrivenSet};
#[cfg(hydrolysis_runtime)]
use cherenkov_record::Recorder;
#[cfg(hydrolysis_runtime)]
use waterui_core::interaction::InteractionState;

/// What Hydrolysis asks a theme's [`menu`](super::WidgetTheme::menu) look
/// for: everything structural about one menu.
///
/// A spec change requests the look again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuSpec {
    direction: LayoutDirection,
}

impl MenuSpec {
    /// The spec of a menu laid out `direction`.
    #[must_use]
    pub const fn new(direction: LayoutDirection) -> Self {
        Self { direction }
    }

    /// The layout direction the menu is laid out in.
    #[must_use]
    pub const fn direction(&self) -> LayoutDirection {
        self.direction
    }
}

/// The measures a menu is laid out by, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuLayout {
    /// One-line menu row height.
    pub row_height: f64,
    /// Horizontal text inset.
    pub horizontal_padding: f64,
    /// Vertical inset of the rows inside the panel.
    pub vertical_padding: f64,
    /// Minimum menu width.
    pub min_width: f64,
    /// Maximum menu width.
    pub max_width: f64,
    /// Estimated width per character, used before text layout is available.
    pub width_per_char: f64,
    /// The panel's corner radius.
    pub corner_radius: f64,
    /// Horizontal inset of a separator.
    pub separator_horizontal_inset: f64,
    /// Separator thickness.
    pub separator_thickness: f64,
}

/// How a menu looks: its [`MenuLayout`] and the parts that draw its panel,
/// its separators and the selected row of a picker popup.
///
/// Every part draws in the widget's own space and records once per frame
/// it is listed in, at rest: a menu has no live state of its own, its rows
/// are buttons with looks of their own.
#[derive(Clone)]
pub struct MenuLook {
    layout: MenuLayout,
    #[cfg_attr(
        not(hydrolysis_runtime),
        expect(
            dead_code,
            reason = "only the renderer records the menu's panel; a build without a painter compiles the theme contract alone"
        )
    )]
    panel: Rc<dyn CustomPart<()>>,
    #[cfg_attr(
        not(hydrolysis_runtime),
        expect(
            dead_code,
            reason = "only the renderer records the menu's separators; a build without a painter compiles the theme contract alone"
        )
    )]
    separator: Rc<dyn CustomPart<()>>,
    selected_row: Option<Rc<dyn CustomPart<()>>>,
}

impl core::fmt::Debug for MenuLook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MenuLook")
            .field("layout", &self.layout)
            .field("selected_row", &self.selected_row.is_some())
            .finish_non_exhaustive()
    }
}

impl MenuLook {
    /// A menu measured by `layout`, whose `panel` draws over the menu's
    /// frame and whose `separator` draws over the line a divider row leaves
    /// for it.
    #[must_use]
    pub fn new<P, S>(layout: MenuLayout, panel: Custom<(), P>, separator: Custom<(), S>) -> Self
    where
        P: DrivenSet<()> + 'static,
        P::Live: 'static,
        S: DrivenSet<()> + 'static,
        S::Live: 'static,
    {
        Self {
            layout,
            panel: Rc::new(panel),
            separator: Rc::new(separator),
            selected_row: None,
        }
    }

    /// Draws the selected row of a picker popup with `part`, over the row's
    /// frame and beneath its label. Without it the selected row is drawn
    /// like any other.
    #[must_use]
    pub fn selected_row<D>(mut self, part: Custom<(), D>) -> Self
    where
        D: DrivenSet<()> + 'static,
        D::Live: 'static,
    {
        self.selected_row = Some(Rc::new(part));
        self
    }

    /// The menu's measures.
    #[must_use]
    pub const fn layout(&self) -> MenuLayout {
        self.layout
    }

    /// Whether the look draws its selected row itself.
    #[must_use]
    pub const fn has_selected_row(&self) -> bool {
        self.selected_row.is_some()
    }

    /// Records the panel over `frame`.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn record_panel(&self, recorder: &mut Recorder, frame: Rect) {
        record_at_rest(&*self.panel, recorder, frame);
    }

    /// Records a separator over `frame`.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn record_separator(&self, recorder: &mut Recorder, frame: Rect) {
        record_at_rest(&*self.separator, recorder, frame);
    }

    /// Records the selected row's part over `frame`, when the look has one.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn record_selected_row(&self, recorder: &mut Recorder, frame: Rect) {
        if let Some(part) = &self.selected_row {
            record_at_rest(&**part, recorder, frame);
        }
    }

    /// The rect a separator occupies inside a divider row's `row` frame.
    #[must_use]
    pub fn separator_rect(&self, row: Rect) -> Rect {
        let y = row.center().y;
        Rect::new(
            row.x0 + self.layout.separator_horizontal_inset,
            y - self.layout.separator_thickness / 2.0,
            row.x1 - self.layout.separator_horizontal_inset,
            y + self.layout.separator_thickness / 2.0,
        )
    }
}

/// Records `part` over `frame` with every value at rest.
#[cfg(hydrolysis_runtime)]
fn record_at_rest(part: &dyn CustomPart<()>, recorder: &mut Recorder, frame: Rect) {
    part.realize(&Inputs::at_rest(frame, InteractionState::empty(), &()))
        .record(recorder, frame);
}
