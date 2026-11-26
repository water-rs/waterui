//! The navigation bar look of the theme contract (water-rs/waterui#1434):
//! what a theme returns for the navigation chrome ([`NavigationBarLook`]).
//!
//! One look serves every navigation bar, the back button of a pushed page
//! and of a collapsed split, and the motion of a page transition. The bar's
//! surface and separator are [`Custom`] parts over the [`BarSurface`] the
//! bar draws in; the back button is a [`Chrome`] whose icon is a custom part
//! of it, so a press, a hover or a focus change moves live operands and
//! records nothing.

use core::time::Duration;
use std::rc::Rc;

use cherenkov_record::WorkingColor;
use waterui_core::EasingCurve;
use waterui_core::layout::LayoutDirection;
use waterui_text::font::Font;

use super::Chrome;
use super::look::{Custom, CustomPart, DrivenSet};

/// What Hydrolysis asks a theme's
/// [`navigation_bar`](super::WidgetTheme::navigation_bar) look for:
/// everything structural about the navigation chrome.
///
/// A spec change requests the look again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavigationSpec {
    direction: LayoutDirection,
}

impl NavigationSpec {
    /// The spec of navigation chrome laid out `direction`.
    #[must_use]
    pub const fn new(direction: LayoutDirection) -> Self {
        Self { direction }
    }

    /// The layout direction the navigation chrome is laid out in.
    #[must_use]
    pub const fn direction(&self) -> LayoutDirection {
        self.direction
    }
}

/// The surface a navigation bar draws in: what the bar's parts follow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarSurface {
    /// The bar's background colour: the application's, or the theme's
    /// `Surface` token when it sets none.
    pub color: WorkingColor,
}

/// Motion policy for backend-rendered navigation transitions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavigationMotion {
    /// Duration for a complete navigation transition.
    pub transition_duration: Duration,
    /// Easing applied to programmatic navigation progress.
    pub transition_easing: EasingCurve,
    /// Absolute travel distance for the Material shared-axis X pattern.
    pub shared_axis_slide_distance: f32,
    /// Progress at which the outgoing layer has faded out and the incoming
    /// layer begins fading in.
    pub fade_through_threshold: f32,
}

/// Layout metrics for navigation bars.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavigationMetrics {
    /// Automatic title-mode bar height.
    pub automatic_bar_height: f32,
    /// Inline title-mode bar height.
    pub inline_bar_height: f32,
    /// Medium title-mode bar height.
    pub medium_bar_height: f32,
    /// Large title-mode bar height.
    pub large_bar_height: f32,
    /// Navigation title slot height for inline bars.
    pub inline_title_height: f32,
    /// Navigation title slot height for medium bars.
    pub medium_title_height: f32,
    /// Navigation title slot height for large bars.
    pub large_title_height: f32,
    /// Title leading inset when no leading bar item is present.
    pub title_leading_inset: f32,
    /// Title trailing inset when no trailing bar item is present.
    pub title_trailing_inset: f32,
    /// Bottom inset for large navigation titles.
    pub large_title_bottom_inset: f32,
    /// Horizontal leading/trailing bar inset.
    pub horizontal_inset: f32,
    /// Gap between title and adjacent items.
    pub item_spacing: f32,
    /// Search field height.
    pub search_height: f32,
    /// Vertical search inset.
    pub search_vertical_inset: f32,
    /// Navigation back button size.
    pub back_button_size: f32,
    /// Navigation back button leading inset.
    pub back_button_leading_inset: f32,
    /// Navigation back button top inset.
    pub back_button_top_inset: f32,
}

/// How the navigation chrome looks and responds: its [`NavigationMetrics`],
/// the page transition's [`NavigationMotion`], the bar's surface and
/// separator, and the back button's [`Chrome`].
///
/// The back button's chrome fills the button's rect, which Hydrolysis places
/// from the metrics; the chrome draws the button's container, its icon (a
/// custom part of the chrome) and its interaction feedback. The bar's parts
/// draw in the bar's own space and record once while the bar's frame and
/// colour hold: a bar has no live state of its own.
#[derive(Clone)]
pub struct NavigationBarLook {
    metrics: NavigationMetrics,
    motion: NavigationMotion,
    back: Chrome,
    bar: Rc<dyn CustomPart<BarSurface>>,
    separator: Option<Rc<dyn CustomPart<BarSurface>>>,
    title_font: Option<Font>,
}

impl core::fmt::Debug for NavigationBarLook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NavigationBarLook")
            .field("metrics", &self.metrics)
            .field("motion", &self.motion)
            .field("back", &self.back)
            .field("separator", &self.separator.is_some())
            .field("title_font", &self.title_font)
            .finish_non_exhaustive()
    }
}

impl NavigationBarLook {
    /// Navigation chrome laid out by `metrics`, its pages moving with
    /// `motion`, its back button wearing `back` and its bars' surface drawn
    /// by `bar`, over the bar's frame. It draws no separator.
    #[must_use]
    pub fn new<B>(
        metrics: NavigationMetrics,
        motion: NavigationMotion,
        back: Chrome,
        bar: Custom<BarSurface, B>,
    ) -> Self
    where
        B: DrivenSet<BarSurface> + 'static,
        B::Live: 'static,
    {
        Self {
            metrics,
            motion,
            back,
            bar: Rc::new(bar),
            separator: None,
            title_font: None,
        }
    }

    /// Draws the line under a top bar with `separator`, over the line's
    /// frame: the bar's inner edge, one point thick.
    #[must_use]
    pub fn separator<S>(mut self, separator: Custom<BarSurface, S>) -> Self
    where
        S: DrivenSet<BarSurface> + 'static,
        S::Live: 'static,
    {
        self.separator = Some(Rc::new(separator));
        self
    }

    /// Overrides the typography used for inline navigation bar titles (`Body`
    /// slot), leaving titles that explicitly specify their own slot untouched.
    #[must_use]
    pub fn title_font(mut self, font: Font) -> Self {
        self.title_font = Some(font);
        self
    }

    /// The measures the navigation chrome is laid out by.
    #[must_use]
    pub const fn metrics(&self) -> NavigationMetrics {
        self.metrics
    }

    /// The motion of a page transition.
    #[must_use]
    pub const fn motion(&self) -> NavigationMotion {
        self.motion
    }

    /// The back button's chrome.
    #[must_use]
    pub const fn back(&self) -> &Chrome {
        &self.back
    }

    /// The bar's surface part.
    #[doc(hidden)]
    #[must_use]
    pub fn bar(&self) -> &Rc<dyn CustomPart<BarSurface>> {
        &self.bar
    }

    /// The separator part, when the look draws one.
    #[doc(hidden)]
    #[must_use]
    pub fn separator_part(&self) -> Option<&Rc<dyn CustomPart<BarSurface>>> {
        self.separator.as_ref()
    }

    /// Inline navigation bar title font override, if configured.
    #[must_use]
    pub const fn get_title_font(&self) -> Option<&Font> {
        self.title_font.as_ref()
    }
}
