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

use core::fmt;
use std::rc::Rc;

use cherenkov_record::Recorder;
use kurbo::{Rect, Vec2};
use waterui_core::animation::Animation;
use waterui_core::layout::LayoutDirection;
use waterui_interaction::interaction::InteractionState;

use super::Surface;
use super::look::{Custom, CustomPart, DrivenSet, Inputs, record_at_rest};

/// Cubic Hermite smoothstep on `[edge0, edge1]`.
fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    if edge1 <= edge0 {
        return if x >= edge1 { 1.0 } else { 0.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * 2.0_f64.mul_add(-t, 3.0)
}

/// How an anchored menu or popover animates between its triggering anchor
/// (`progress = 0.0`) and its settled panel (`progress = 1.0`).
///
/// Themes attach this to a [`MenuLook`] via [`MenuLook::presentation_motion`];
/// Hydrolysis drives presentation progress with [`present`](Self::present) on
/// open and [`dismiss`](Self::dismiss) on close (retaining the dismissed
/// presentation as an inert ghost until settled), and passes the sampled
/// [`MenuPresentationSample`] to the panel surface, content, and trigger.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuPresentationMotion {
    /// Animation driving presentation progress from `0.0` toward `1.0` on open.
    pub present: Animation,
    /// Animation driving presentation progress back toward `0.0` on close while
    /// retaining the dismissed ghost until settled.
    pub dismiss: Animation,
    /// Smoothstep window `(edge0, edge1)` over clamped progress for the menu
    /// content's reveal opacity (`0.0 -> 1.0`). Defaults to `(0.0, 1.0)`.
    pub content_opacity_window: (f64, f64),
    /// Starting scale of the menu content at `progress = 0.0`, interpolated to
    /// `1.0` across [`content_scale_window`](Self::content_scale_window).
    /// Defaults to `1.0`.
    pub content_scale_from: f64,
    /// Smoothstep window `(edge0, edge1)` over clamped progress for the menu
    /// content's scale reveal. Defaults to `(0.0, 1.0)`.
    pub content_scale_window: (f64, f64),
    /// Optional smoothstep window `(edge0, edge1)` over clamped progress for
    /// fading the triggering anchor's content out
    /// (`1.0 - smoothstep(edge0, edge1, q)`). Defaults to `None` (trigger
    /// remains at opacity `1.0`).
    pub trigger_opacity_window: Option<(f64, f64)>,
}

impl MenuPresentationMotion {
    /// A presentation motion driven by `present` on open and `dismiss` on close,
    /// with linear-window content fade, unit content scale, and no trigger fade.
    #[must_use]
    pub const fn new(present: Animation, dismiss: Animation) -> Self {
        Self {
            present,
            dismiss,
            content_opacity_window: (0.0, 1.0),
            content_scale_from: 1.0,
            content_scale_window: (0.0, 1.0),
            trigger_opacity_window: None,
        }
    }

    /// Sets the smoothstep window `(edge0, edge1)` over progress for menu
    /// content opacity (`0.0 -> 1.0`).
    #[must_use]
    pub const fn content_opacity_window(mut self, edge0: f64, edge1: f64) -> Self {
        self.content_opacity_window = (edge0, edge1);
        self
    }

    /// Sets the starting scale of menu content at `progress = 0.0`.
    #[must_use]
    pub const fn content_scale_from(mut self, scale: f64) -> Self {
        self.content_scale_from = scale;
        self
    }

    /// Sets the smoothstep window `(edge0, edge1)` over progress for menu
    /// content scale (`content_scale_from -> 1.0`).
    #[must_use]
    pub const fn content_scale_window(mut self, edge0: f64, edge1: f64) -> Self {
        self.content_scale_window = (edge0, edge1);
        self
    }

    /// Sets the smoothstep window `(edge0, edge1)` over progress for fading the
    /// triggering anchor's content out (`1.0 -> 0.0`).
    #[must_use]
    pub const fn trigger_opacity_window(mut self, edge0: f64, edge1: f64) -> Self {
        self.trigger_opacity_window = Some((edge0, edge1));
        self
    }

    /// Menu content opacity in `[0.0, 1.0]` at `progress`.
    #[must_use]
    pub fn content_opacity(&self, progress: f64) -> f64 {
        smoothstep(
            self.content_opacity_window.0,
            self.content_opacity_window.1,
            progress.clamp(0.0, 1.0),
        )
    }

    /// Menu content scale at `progress`.
    #[must_use]
    pub fn content_scale(&self, progress: f64) -> f64 {
        let t = smoothstep(
            self.content_scale_window.0,
            self.content_scale_window.1,
            progress.clamp(0.0, 1.0),
        );
        (1.0 - self.content_scale_from).mul_add(t, self.content_scale_from)
    }

    /// Triggering anchor opacity in `[0.0, 1.0]` at `progress`.
    #[must_use]
    pub fn trigger_opacity(&self, progress: f64) -> f64 {
        self.trigger_opacity_window.map_or(1.0, |(edge0, edge1)| {
            1.0 - smoothstep(edge0, edge1, progress.clamp(0.0, 1.0))
        })
    }

    /// Samples the interpolated surface bounds and reveal curves at `progress`
    /// between `origin` (trigger anchor bounds) and `panel` (settled menu
    /// panel bounds).
    #[must_use]
    pub fn sample(&self, origin: Rect, panel: Rect, progress: f64) -> MenuPresentationSample {
        let x = (panel.x0 - origin.x0).mul_add(progress, origin.x0);
        let y = (panel.y0 - origin.y0).mul_add(progress, origin.y0);
        let width = (panel.width() - origin.width())
            .mul_add(progress, origin.width())
            .max(0.0);
        let height = (panel.height() - origin.height())
            .mul_add(progress, origin.height())
            .max(0.0);
        let surface = Rect::new(x, y, x + width, y + height);
        MenuPresentationSample {
            origin,
            panel,
            surface,
            progress,
            content_opacity: self.content_opacity(progress),
            content_scale: self.content_scale(progress),
            trigger_opacity: self.trigger_opacity(progress),
        }
    }
}

/// One sampled instant of an animated menu or popover presentation, passed to
/// [`MenuLook::presentation_panel`] when recording the morphing panel surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuPresentationSample {
    /// The triggering anchor's bounds in the same coordinate space as
    /// [`panel`](Self::panel).
    pub origin: Rect,
    /// The menu panel's settled target bounds.
    pub panel: Rect,
    /// The interpolated panel surface bounds at [`progress`](Self::progress).
    pub surface: Rect,
    /// Presentation progress (`0.0` at the trigger, `1.0` settled; may briefly
    /// overshoot under an underdamped spring).
    pub progress: f64,
    /// Opacity in `[0.0, 1.0]` applied to the menu's content.
    pub content_opacity: f64,
    /// Uniform scale applied to the menu's content.
    pub content_scale: f64,
    /// Opacity in `[0.0, 1.0]` applied to the triggering anchor's content.
    pub trigger_opacity: f64,
}

impl MenuPresentationSample {
    /// A settled sample at `progress = 1.0` over `panel` anchored at `origin`.
    #[must_use]
    pub const fn at_rest(origin: Rect, panel: Rect) -> Self {
        Self {
            origin,
            panel,
            surface: panel,
            progress: 1.0,
            content_opacity: 1.0,
            content_scale: 1.0,
            trigger_opacity: 1.0,
        }
    }

    /// Shifts `origin`, `panel`, and `surface` by `offset`, converting the
    /// sample into a local coordinate space.
    #[must_use]
    pub fn translated(self, offset: Vec2) -> Self {
        Self {
            origin: self.origin + offset,
            panel: self.panel + offset,
            surface: self.surface + offset,
            ..self
        }
    }
}

type PresentationPanelDrawer = Rc<dyn Fn(&mut Recorder, &MenuPresentationSample)>;

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
    separator: Rc<dyn CustomPart<()>>,
    selected_row: Option<Rc<dyn CustomPart<()>>>,
    presentation_motion: Option<MenuPresentationMotion>,
    presentation_panel: Option<PresentationPanelDrawer>,
}

impl fmt::Debug for MenuLook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MenuLook")
            .field("layout", &self.layout)
            .field("panel", &self.panel)
            .field("selected_row", &self.selected_row.is_some())
            .field("presentation_motion", &self.presentation_motion)
            .field("presentation_panel", &self.presentation_panel.is_some())
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
        Self {
            layout,
            panel,
            separator: Rc::new(separator),
            selected_row: None,
            presentation_motion: None,
            presentation_panel: None,
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

    /// Attaches theme-driven present/dismiss motion and reveal curves to this
    /// menu look. Without it, anchored overlays present without a theme-driven
    /// morph and popup windows use the default enter curve.
    #[must_use]
    pub const fn presentation_motion(mut self, motion: MenuPresentationMotion) -> Self {
        self.presentation_motion = Some(motion);
        self
    }

    /// Supplies a custom panel drawer invoked with the live
    /// [`MenuPresentationSample`] (`origin`, `panel`, `surface`, `progress`)
    /// so a theme can draw one continuously morphing surface (including
    /// backdrop materials) between the trigger anchor and the menu panel.
    /// Without it, Hydrolysis records [`panel`](Self::panel) over
    /// `sample.surface`.
    #[must_use]
    pub fn presentation_panel(
        mut self,
        drawer: impl Fn(&mut Recorder, &MenuPresentationSample) + 'static,
    ) -> Self {
        self.presentation_panel = Some(Rc::new(drawer));
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

    /// The theme-driven presentation motion, when configured.
    #[must_use]
    pub const fn get_presentation_motion(&self) -> Option<&MenuPresentationMotion> {
        self.presentation_motion.as_ref()
    }

    /// Whether the look supplies a custom morphing panel drawer.
    #[must_use]
    pub const fn has_presentation_panel(&self) -> bool {
        self.presentation_panel.is_some()
    }

    /// The panel's surface: its shape, fill, border and elevation.
    #[must_use]
    pub const fn panel(&self) -> &Surface {
        &self.panel
    }

    /// Records the morphing panel surface through [`Self::presentation_panel`]
    /// when configured, returning `true` if drawn or `false` if the caller
    /// should fall back to [`Self::panel`].
    #[doc(hidden)]
    pub fn record_presentation_panel(
        &self,
        recorder: &mut Recorder,
        sample: &MenuPresentationSample,
    ) -> bool {
        self.presentation_panel.as_ref().is_some_and(|drawer| {
            drawer(recorder, sample);
            true
        })
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

    /// Records a separator over `frame`.
    #[doc(hidden)]
    pub fn record_separator(&self, recorder: &mut Recorder, frame: Rect) {
        record_at_rest(&*self.separator, recorder, frame);
    }

    /// Records the selected row's part over `frame`, when the look has one.
    #[doc(hidden)]
    pub fn record_selected_row(&self, recorder: &mut Recorder, frame: Rect) {
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

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use cherenkov_record::{Paint, WorkingColor};
    use kurbo::{Rect, Vec2};
    use waterui_core::animation::Animation;

    use super::super::CornerShape;
    use super::{
        Custom, MenuLayout, MenuLook, MenuPresentationMotion, MenuPresentationSample, Surface,
    };

    fn test_layout() -> MenuLayout {
        MenuLayout {
            row_height: 40.0,
            horizontal_padding: 12.0,
            vertical_padding: 6.0,
            min_width: 112.0,
            max_width: 280.0,
            width_per_char: 8.0,
            separator_horizontal_inset: 0.0,
            separator_thickness: 1.0,
        }
    }

    fn test_look() -> MenuLook {
        MenuLook::new(
            test_layout(),
            Surface::new(
                CornerShape::radius(12.0),
                Paint::Solid(WorkingColor::new([1.0, 1.0, 1.0, 1.0])),
            ),
            Custom::new((), |_recorder, _frame, ()| {}),
        )
    }

    #[test]
    fn menu_look_defaults_have_no_presentation_motion_or_custom_panel() {
        let look = test_look();
        assert!(look.get_presentation_motion().is_none());
        assert!(!look.has_presentation_panel());
    }

    #[test]
    fn presentation_motion_samples_bounds_opacity_scale_and_trigger_fade() {
        let motion = MenuPresentationMotion::new(
            Animation::linear(Duration::from_millis(360)),
            Animation::linear(Duration::from_millis(280)),
        )
        .content_opacity_window(0.25, 0.75)
        .content_scale_from(0.92)
        .content_scale_window(0.15, 0.85)
        .trigger_opacity_window(0.0, 0.25);

        let origin = Rect::new(20.0, 10.0, 100.0, 44.0);
        let panel = Rect::new(20.0, 48.0, 220.0, 208.0);

        let start = motion.sample(origin, panel, 0.0);
        assert_eq!(start.surface, origin);
        assert!((start.content_opacity - 0.0).abs() < 1e-12);
        assert!((start.content_scale - 0.92).abs() < 1e-12);
        assert!((start.trigger_opacity - 1.0).abs() < 1e-12);

        let mid = motion.sample(origin, panel, 0.5);
        assert!((mid.surface.x0 - 20.0).abs() < 1e-12);
        assert!((mid.surface.y0 - 29.0).abs() < 1e-12);
        assert!((mid.surface.width() - 140.0).abs() < 1e-12);
        assert!((mid.surface.height() - 97.0).abs() < 1e-12);
        assert!((mid.content_opacity - 0.5).abs() < 1e-12);
        assert!((mid.content_scale - 0.96).abs() < 1e-12);
        assert!((mid.trigger_opacity - 0.0).abs() < 1e-12);

        let end = motion.sample(origin, panel, 1.0);
        assert_eq!(end.surface, panel);
        assert!((end.content_opacity - 1.0).abs() < 1e-12);
        assert!((end.content_scale - 1.0).abs() < 1e-12);
        assert!((end.trigger_opacity - 0.0).abs() < 1e-12);

        // Underdamped spring overshoot expands surface slightly past `panel`
        // while keeping reveal curves clamped at their endpoints.
        let overshoot = motion.sample(origin, panel, 1.05);
        assert!(overshoot.surface.width() > panel.width());
        assert!((overshoot.content_opacity - 1.0).abs() < 1e-12);
        assert!((overshoot.content_scale - 1.0).abs() < 1e-12);
        assert!((overshoot.trigger_opacity - 0.0).abs() < 1e-12);

        let shifted = mid.translated(Vec2::new(-20.0, -48.0));
        assert_eq!(shifted.panel, panel + Vec2::new(-20.0, -48.0));
        assert_eq!(shifted.origin, origin + Vec2::new(-20.0, -48.0));
        assert_eq!(shifted.surface, mid.surface + Vec2::new(-20.0, -48.0));

        let rest = MenuPresentationSample::at_rest(origin, panel);
        assert_eq!(rest.surface, panel);
        assert!((rest.progress - 1.0).abs() < 1e-12);
        assert!((rest.trigger_opacity - 1.0).abs() < 1e-12);
    }
}
