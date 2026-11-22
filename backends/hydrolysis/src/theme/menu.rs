//! The menu look of the theme contract (water-rs/waterui#1434): what a theme
//! returns for every menu presentation ([`MenuLook`]).
//!
//! One look serves the picker popup, the text context menu and the popup
//! window a menu button opens, so they are one surface: the same row
//! measures, the same panel and the same separators. Hydrolysis measures,
//! places and hit-tests a menu from [`MenuLayout`], records the panel as the
//! [`Surface`] the look states and the separators its parts draw, so the
//! geometry it measures is the geometry it draws and the accessibility
//! option rectangles are the drawn ones.

use std::rc::Rc;

use kurbo::Rect;
use waterui_core::layout::LayoutDirection;
use waterui_interaction::interaction::InteractionState;

use super::Surface;
#[cfg(hydrolysis_runtime)]
use super::look::record_at_rest;
use super::look::{Custom, CustomPart, DrivenSet, Inputs};
#[cfg(hydrolysis_runtime)]
use cherenkov_record::Recorder;

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
    /// Horizontal inset of a separator.
    pub separator_horizontal_inset: f64,
    /// Separator thickness.
    pub separator_thickness: f64,
}

/// How a menu looks: its [`MenuLayout`], its panel as a [`Surface`], and the
/// parts that draw its separators and the selected row of a picker popup.
///
/// The panel's shape, fill, border and elevation are the surface's, recorded
/// at rest over the menu's frame as a control chrome records its surface; a
/// popup window leaves the elevation's reach around the panel for its
/// shadow. The parts draw in the widget's own space and record once per
/// frame they are listed in, at rest: a menu has no live state of its own,
/// its rows are buttons with looks of their own.
#[derive(Clone)]
pub struct MenuLook {
    layout: MenuLayout,
    panel: Surface,
    #[cfg_attr(
        not(hydrolysis_runtime),
        expect(
            dead_code,
            reason = "only the renderer records the menu's separators; a build without a painter compiles the theme contract alone"
        )
    )]
    #[cfg(any(feature = "text-field", feature = "secure-field"))]
    separator: Rc<dyn CustomPart<()>>,
    selected_row: Option<Rc<dyn CustomPart<()>>>,
}

impl core::fmt::Debug for MenuLook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MenuLook")
            .field("layout", &self.layout)
            .field("panel", &self.panel)
            .field("selected_row", &self.selected_row.is_some())
            .finish_non_exhaustive()
    }
}

impl MenuLook {
    /// A menu measured by `layout`, whose panel is `panel` over the menu's
    /// frame and whose `separator` draws over the line a divider row leaves
    /// for it.
    #[must_use]
    pub fn new<S>(layout: MenuLayout, panel: Surface, separator: Custom<(), S>) -> Self
    where
        S: DrivenSet<()> + 'static,
        S::Live: 'static,
    {
        #[cfg(not(any(feature = "text-field", feature = "secure-field")))]
        drop(separator);
        Self {
            layout,
            panel,
            #[cfg(any(feature = "text-field", feature = "secure-field"))]
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

    /// The panel's surface: its shape, fill, border and elevation.
    #[must_use]
    pub const fn panel(&self) -> &Surface {
        &self.panel
    }

    /// How far the panel's shadows reach beyond its edge when it spans
    /// `frame`, at rest. A popup window leaves this room on every side of
    /// the panel; `0.0` when the panel casts no shadow.
    #[must_use]
    pub fn panel_extent(&self, frame: Rect) -> f64 {
        self.panel
            .get_elevation()
            .target(&Inputs::at_rest(frame, InteractionState::empty(), &()))
            .extent()
    }

    /// Records the panel at rest over `frame`: its elevation, fill and
    /// border, as a control chrome records its surface.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn record_panel(&self, recorder: &mut Recorder, frame: Rect) {
        crate::renderer::chrome::record_surface_at_rest(&self.panel, recorder, frame);
    }

    /// Records a separator over `frame`.
    #[cfg(hydrolysis_runtime)]
    #[cfg(any(feature = "text-field", feature = "secure-field"))]
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
