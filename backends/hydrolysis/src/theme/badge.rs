//! The badge look of the theme contract (water-rs/waterui#1434): what a
//! theme returns for a badge ([`BadgeLook`]).
//!
//! A badge is an indicator anchored on its content's corner: a small dot
//! for a count of zero, a count pill for anything else. Hydrolysis measures
//! and places the indicator from the look's [`BadgeMetrics`] and the count's
//! text, and records the dot or the pill from the look's parts.

use std::rc::Rc;

#[cfg(hydrolysis_runtime)]
use cherenkov_record::Recorder;
#[cfg(hydrolysis_runtime)]
use kurbo::Rect;
use waterui_graphics::color::Color;
use waterui_text::font::Font;

use super::BadgeMetrics;
#[cfg(hydrolysis_runtime)]
use super::look::record_at_rest;
use super::look::{Custom, CustomPart, DrivenSet};

/// How a badge looks: its [`BadgeMetrics`], the font and colour of its
/// count, and the parts that draw the small dot and the count pill.
///
/// Both parts draw in the widget's own space and record once per frame they
/// are listed in, at rest.
#[derive(Clone)]
pub struct BadgeLook {
    metrics: BadgeMetrics,
    label_font: Font,
    label_color: Color,
    #[cfg_attr(
        not(hydrolysis_runtime),
        expect(
            dead_code,
            reason = "only the renderer records the badge's dot; a build without a painter compiles the theme contract alone"
        )
    )]
    small: Rc<dyn CustomPart<()>>,
    #[cfg_attr(
        not(hydrolysis_runtime),
        expect(
            dead_code,
            reason = "only the renderer records the badge's pill; a build without a painter compiles the theme contract alone"
        )
    )]
    large: Rc<dyn CustomPart<()>>,
}

impl core::fmt::Debug for BadgeLook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BadgeLook")
            .field("metrics", &self.metrics)
            .field("label_font", &self.label_font)
            .field("label_color", &self.label_color)
            .finish_non_exhaustive()
    }
}

impl BadgeLook {
    /// A badge measured by `metrics` whose count is set in `label_font` and
    /// `label_color`, whose `small` part draws the dot over its rect and
    /// whose `large` part draws the count pill over its rect.
    #[must_use]
    pub fn new<S, L>(
        metrics: BadgeMetrics,
        label_font: Font,
        label_color: Color,
        small: Custom<(), S>,
        large: Custom<(), L>,
    ) -> Self
    where
        S: DrivenSet<()> + 'static,
        S::Live: 'static,
        L: DrivenSet<()> + 'static,
        L::Live: 'static,
    {
        Self {
            metrics,
            label_font,
            label_color,
            small: Rc::new(small),
            large: Rc::new(large),
        }
    }

    /// The badge's measures.
    #[must_use]
    pub const fn metrics(&self) -> BadgeMetrics {
        self.metrics
    }

    /// The font of the count.
    #[must_use]
    pub const fn label_font(&self) -> &Font {
        &self.label_font
    }

    /// The colour of the count.
    #[must_use]
    pub const fn label_color(&self) -> &Color {
        &self.label_color
    }

    /// Records the small dot over `frame`.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn record_small(&self, recorder: &mut Recorder, frame: Rect) {
        record_at_rest(&*self.small, recorder, frame);
    }

    /// Records the count pill over `frame`.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn record_large(&self, recorder: &mut Recorder, frame: Rect) {
        record_at_rest(&*self.large, recorder, frame);
    }
}
