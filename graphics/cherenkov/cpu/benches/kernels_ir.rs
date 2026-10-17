//! Callgrind (gungraun) instruction-count benchmarks for the span
//! kernels over the shared case table (`cases::cases()`): `eval`
//! benchmarks measure `PaintData::eval_span`, `composite` benchmarks
//! measure `composite_span`, one `#[benches::<kind>]` case grid per
//! paint kind so a case's gungraun id names its kind (`linear_0`,
//! `sweep_17`) — inserting a kind's case renames only that kind's
//! numbers. Setup builds the inputs outside the measured region.
//! Wall-clock coverage of the same table is `kernels_wall`.
//!
//! Each measured call goes through the `kernels::*` Level-dispatched
//! entry point, so every count includes one `Level` match and
//! `vectorize` hop; the production kernels thread the concrete token
//! inside the run's dispatch instead.
//!
//! Run with:
//!
//! ```text
//! cargo bench -p cherenkov-cpu --features testing --bench kernels_ir
//! ```

mod cases;

mod kernels_bench {
    use std::hint::black_box;
    use std::mem::MaybeUninit;

    use cherenkov_cpu::testing::kernels as bench_kernels;
    use gungraun::{library_benchmark, library_benchmark_group};

    use super::cases::{self, Case, Composite, Eval, Kind};

    fn eval_setup(case: Case) -> Eval {
        case.eval()
    }

    fn composite_setup(case: Case) -> Composite {
        case.composite()
    }

    #[library_benchmark]
    #[benches::solid_opaque(iter = cases::of(Kind::SolidOpaque), setup = eval_setup)]
    #[benches::solid_translucent(iter = cases::of(Kind::SolidTranslucent), setup = eval_setup)]
    #[benches::linear(iter = cases::of(Kind::Linear), setup = eval_setup)]
    #[benches::radial(iter = cases::of(Kind::Radial), setup = eval_setup)]
    #[benches::radial_identical(iter = cases::of(Kind::RadialIdentical), setup = eval_setup)]
    #[benches::sweep(iter = cases::of(Kind::Sweep), setup = eval_setup)]
    #[benches::image_nearest(iter = cases::of(Kind::ImageNearest), setup = eval_setup)]
    #[benches::image_bilinear(iter = cases::of(Kind::ImageBilinear), setup = eval_setup)]
    #[benches::transformed(iter = cases::of(Kind::Transformed), setup = eval_setup)]
    fn eval(mut input: Eval) -> Vec<MaybeUninit<[f32; 4]>> {
        black_box(bench_kernels::eval_span(
            input.level,
            &input.paint,
            black_box(input.x0),
            black_box(input.py),
            black_box(&mut input.out),
        ));
        input.out
    }

    #[library_benchmark]
    #[benches::solid_opaque(iter = cases::of(Kind::SolidOpaque), setup = composite_setup)]
    #[benches::solid_translucent(iter = cases::of(Kind::SolidTranslucent), setup = composite_setup)]
    #[benches::linear(iter = cases::of(Kind::Linear), setup = composite_setup)]
    #[benches::radial(iter = cases::of(Kind::Radial), setup = composite_setup)]
    #[benches::radial_identical(iter = cases::of(Kind::RadialIdentical), setup = composite_setup)]
    #[benches::sweep(iter = cases::of(Kind::Sweep), setup = composite_setup)]
    #[benches::image_nearest(iter = cases::of(Kind::ImageNearest), setup = composite_setup)]
    #[benches::image_bilinear(iter = cases::of(Kind::ImageBilinear), setup = composite_setup)]
    #[benches::transformed(iter = cases::of(Kind::Transformed), setup = composite_setup)]
    fn composite(mut input: Composite) -> Vec<[f32; 4]> {
        bench_kernels::composite_span(
            input.level,
            black_box(&mut input.dst),
            black_box(&input.src),
            black_box(&input.cov),
            input.clip,
            input.space,
        );
        black_box(input.dst)
    }

    library_benchmark_group!(name = kernels, benchmarks = [eval, composite]);
}

use gungraun::main;
use kernels_bench::kernels;

main!(library_benchmark_groups = kernels);
