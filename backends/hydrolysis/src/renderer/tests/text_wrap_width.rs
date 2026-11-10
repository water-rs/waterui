//! A text breaks lines at the width of its own layout answer
//! (water-rs/waterui#4125): the response rectangle positions the glyphs, and
//! its corners, widened to `f64`, are never the source of the line-break
//! width. A label centred in a wide window starts hundreds of points from the
//! origin, so its far edge `x + width` rounds to a coarser float than the
//! width itself; a width recovered from the corners is then a step short of
//! the measured advance and wraps the last word.

use core::time::Duration;
use std::time::Instant;

use waterui::component::text;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;

const WINDOW_HEIGHT: u32 = 120;
/// Windows wide enough that the centred label's far edge needs coarser floats
/// than its width does.
const WINDOW_WIDTHS: core::ops::RangeInclusive<u32> = 1001..=1016;
/// A size whose advances are not multiples of a power of two.
const FONT_SIZE: f64 = 13.37;
const LABELS: [&str; 3] = ["Menu Style", "Radio Style", "Fruit Basket"];

/// The number of glyph runs the settled frame records: one per laid-out line.
fn drawn_runs(label: &'static str, width: u32) -> usize {
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(move || AnyView::new(text(label).size(FONT_SIZE))),
        width,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    );
    let mut at = Instant::now();
    for _ in 0..4 {
        at += Duration::from_millis(16);
        let _ = runtime.pump_at(false, at);
    }
    runtime.renderer().painted_scene().glyph_runs().count()
}

#[test]
fn a_text_draws_on_one_line_at_every_fractional_origin() {
    let wrapped: Vec<_> = LABELS
        .into_iter()
        .flat_map(|label| WINDOW_WIDTHS.map(move |width| (label, width)))
        .filter(|&(label, width)| drawn_runs(label, width) != 1)
        .collect();
    assert!(
        wrapped.is_empty(),
        "texts that wrapped although they fit, as (label, window width): {wrapped:?}"
    );
}
