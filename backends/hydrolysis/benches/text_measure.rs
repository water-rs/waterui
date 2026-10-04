//! Re-measure cost of a dense text workload through the real layout path.
//!
//! Workload: a 200-row list of mixed-length body text plus one long
//! paragraph, re-measured at three wrap widths — the pattern a live resize
//! or list rebuild drives through `text_dimensions_from_layout`. This is the
//! regression gate for the ink-extent work that measure performs per glyph
//! since #237.
//!
//! Run with `cargo bench --features testing`.

mod support {
    /// Truncates a `u64` index to `usize`; the values are row indices that fit.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the values are fragment indices that fit usize"
    )]
    pub const fn u64_as_usize(v: u64) -> usize {
        v as usize
    }
}

use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use hydrolysis::HeadlessRuntime;
use hydrolysis_m3::Material3;
use nami::Binding;
use nami::collection::SignalCollection;
use waterui::prelude::text;
use waterui_core::AnyView;
use waterui_core::Environment;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::frame::Frame;
use waterui_layout::stack::{VStack, vstack};

const WIDTHS: [f64; 3] = [320.0, 480.0, 640.0];

const PARAGRAPH: &str = "The measure path must cover every glyph's painted \
ink, not only the pen advance: a shaping run's outlines overhang their \
advance on real faces, and the frame that clips them has to know. WaterUI \
renders through a retained tree, so layout invalidation replays the measure \
on every resize, every list mutation, and every theme change.";

fn row_text(row: u64) -> String {
    const FRAGMENTS: [&str; 8] = [
        "inbox",
        "standup notes moved to docs",
        "WAVy jiggle",
        "release checklist: measure, paint, compare",
        "ok",
        "design review of the retained tree invalidation",
        "play fully",
        "suggestion row with a somewhat longer label that wraps on narrow widths",
    ];
    let mut text = String::from(FRAGMENTS[(support::u64_as_usize(row)) % FRAGMENTS.len()]);
    for extra in 1..=(row % 3) {
        text.push(' ');
        text.push_str(
            FRAGMENTS
                [(support::u64_as_usize(row) + support::u64_as_usize(extra)) % FRAGMENTS.len()],
        );
    }
    text
}

fn bench_remeasure_at_three_widths(c: &mut Criterion) {
    let width = Binding::container(WIDTHS[0]);
    let driver = width.clone();
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let rows = VStack::for_each(
            SignalCollection::new(Binding::container(
                (0..200u64).map(SelfId::new).collect::<Vec<_>>(),
            )),
            |id: SelfId<u64>| text(row_text(id.into_inner())).body(),
        );
        let paragraph = text(PARAGRAPH).body();
        AnyView::new(Frame::new(vstack((rows, paragraph))).max_width(width.clone()))
    });

    let mut env = Environment::new();
    hydrolysis::testing::install_theme(&mut env);
    let mut runtime =
        HeadlessRuntime::new_for_tests(env, builder, 800, 600, Material3::defaults());
    let at = Instant::now();
    // Settle: first layout, font decode and shader warm-up happen here, not
    // inside the measured loop.
    runtime.pump_at(false, at);

    c.bench_function("list200_plus_paragraph_at_3_widths", |b| {
        let mut iter_idx = 0usize;
        b.iter(|| {
            driver.set(WIDTHS[iter_idx % WIDTHS.len()]);
            iter_idx += 1;
            runtime.pump_at(false, at);
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(2))
        .measurement_time(Duration::from_secs(8));
    targets = bench_remeasure_at_three_widths
}
criterion_main!(benches);
