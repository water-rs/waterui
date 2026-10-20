//! water-rs/waterui#2719: a lone resized text's glyph tops must not clip at
//! the top edge of the capture. The theme's `Body` slot carries an absolute
//! line height (the way `Material3` publishes 16/24), and `.size(40)` drops
//! that pitch on the resized run — the paragraph default must not reapply it.

use std::time::Instant;

use waterui::prelude::text;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_text::font::{Body, FontWeight, ResolvedFont};

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;

const W: usize = 375;
const H: usize = 667;

#[test]
fn a_lone_resized_text_keeps_its_glyph_tops_inside_the_window() {
    let mut env = test_environment();
    waterui::theme::install_font_signal::<Body>(
        &mut env,
        nami::Computed::constant(
            ResolvedFont::new(16.0, FontWeight::Normal).with_typography_metrics(24.0, 0.0),
        ),
    );
    let builder = AnyViewBuilder::<AnyView>::new(|| AnyView::new(text("Hello 0").size(40.0)));
    let mut runtime = HeadlessRuntime::new_for_tests(
        env,
        builder,
        crate::num_cast::usize_as_u32(W),
        crate::num_cast::usize_as_u32(H),
        MinimalTestTheme::default(),
    )
    .with_scale_factor(2.0);
    let at = Instant::now();
    for _ in 0..4 {
        runtime.pump_at(false, at);
    }
    let snapshot = runtime.pump_at(true, at).snapshot.expect("capture");

    let row_bytes = snapshot.width as usize * 4;
    let mut first_ink_row = None;
    let mut last_ink_row = None;
    for (y, row) in snapshot.rgba8.chunks(row_bytes).enumerate() {
        let painted = row
            .chunks(4)
            .filter(|px| px[0] < 0xC0 || px[1] < 0xC0 || px[2] < 0xC0)
            .count();
        if painted > 0 {
            first_ink_row.get_or_insert((y, painted));
            last_ink_row = Some((y, painted));
        }
    }
    let (first, _) = first_ink_row.expect("the text painted");
    let (last, _) = last_ink_row.expect("the text painted");
    // The clipped run began painting in row 0 — strokes sliced by the window
    // edge. A correct pitch leaves the glyph tops several device rows in;
    // `first > 0` is the face-independent form of that contract.
    assert!(
        first > 0,
        "glyph tops clipped at the window edge (first ink row {first}, last {last})"
    );
}
