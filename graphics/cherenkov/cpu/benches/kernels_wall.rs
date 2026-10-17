//! Wall-clock benchmarks for the span kernels over the shared case
//! table (`cases::cases()`): `eval` times `PaintData::eval_span` and
//! `composite` times `composite_span`. Instruction-count coverage of the
//! same table is `kernels_ir`.
//!
//! Each measured call goes through the `kernels::*` Level-dispatched
//! entry point, so the numbers include one `Level` match and `vectorize`
//! hop per iteration — the concrete-token kernels the renderer threads
//! are `crate::render`'s, and production performance is accepted on the
//! whole-frame `ir_gate`/`pixel_compare` evidence, not these figures.
//!
//! Run with:
//!
//! ```text
//! cargo bench -p cherenkov-cpu --features testing --bench kernels_wall
//! ```

mod cases;

use std::hint::black_box;

use cherenkov_cpu::testing::kernels;
use divan::Bencher;

use cases::{Case, Kind};

fn main() {
    divan::main();
}

#[divan::bench(args = Kind::ALL.iter().copied().flat_map(cases::of))]
fn eval(b: Bencher, case: &Case) {
    let cases::Eval {
        level,
        paint,
        x0,
        py,
        mut out,
    } = case.eval();
    b.bench_local(|| {
        black_box(kernels::eval_span(
            level,
            &paint,
            black_box(x0),
            black_box(py),
            black_box(&mut out),
        ));
    });
}

#[divan::bench(args = Kind::ALL.iter().copied().flat_map(cases::of))]
fn composite(b: Bencher, case: &Case) {
    let mut input = case.composite();
    b.bench_local(|| {
        kernels::composite_span(
            input.level,
            black_box(&mut input.dst),
            black_box(&input.src),
            black_box(&input.cov),
            input.clip,
            input.space,
        );
    });
}
