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

#[cfg(not(target_arch = "wasm32"))]
mod cases;

#[cfg(not(target_arch = "wasm32"))]
mod kernels_bench {
    use std::hint::black_box;
    use std::mem::MaybeUninit;

    use cherenkov_cpu::testing::kernels as bench_kernels;
    use gungraun::{library_benchmark, library_benchmark_group};

    use super::cases::{self, Case, Composite, Eval, Kind};

    fn geometry_setup(name: &str) -> cherenkov_cpu::testing::GeometryChange {
        use cherenkov_scene::{Draw, Item, Layer, Shape};
        fn stroke(
            layer: &Layer,
        ) -> Option<(&cherenkov::kurbo::BezPath, &cherenkov_scene::StrokeStyle)> {
            layer.items.iter().find_map(|item| match item {
                Item::Draw(Draw::Stroke {
                    shape: Shape::Path { path },
                    stroke,
                    ..
                }) => Some((path, stroke)),
                Item::Layer(layer) => stroke(layer),
                _ => None,
            })
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../scenes/perf")
            .join(name);
        let scene = cherenkov_scene::Scene::load(&path).expect("generated perf scene");
        let (path, style) = stroke(&scene.root).expect("road-network has a path stroke");
        cherenkov_cpu::testing::GeometryChange::new(path.clone(), style.into())
    }

    #[library_benchmark]
    #[benches::roads(args = ["road-network-n10", "road-network-n20", "road-network"], setup = geometry_setup)]
    fn geometry_change(
        input: cherenkov_cpu::testing::GeometryChange,
    ) -> cherenkov_cpu::testing::GeometryChange {
        black_box(
            input.recompile(black_box(cherenkov::kurbo::Affine::scale_non_uniform(
                1.01, 0.99,
            ))),
        )
    }

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

    #[library_benchmark]
    #[benches::lengths(iter = cases::LENS.iter().copied(), setup = cases::opaque_store)]
    fn opaque_store(mut input: Composite) -> Vec<[f32; 4]> {
        bench_kernels::composite_solid(
            input.level,
            black_box(&mut input.dst),
            black_box(&input.cov),
            input.clip,
            black_box(input.src[0]),
            input.space,
        );
        black_box(input.dst)
    }

    #[library_benchmark]
    #[benches::lengths(iter = cases::LENS.iter().copied(), setup = cases::opaque_store)]
    fn uniform_store(mut input: Composite) -> Vec<[f32; 4]> {
        bench_kernels::composite_uniform(
            input.level,
            black_box(&mut input.dst),
            black_box(1.0),
            input.clip,
            black_box(input.src[0]),
            input.space,
        );
        black_box(input.dst)
    }

    #[library_benchmark]
    #[benches::widths(args = [64, 256, 2048], setup = bench_kernels::RowReader::new)]
    fn row_reader(mut row: bench_kernels::RowReader) -> bench_kernels::RowReader {
        black_box(row.read());
        row
    }

    fn encode_setup(len: usize) -> (cherenkov_cpu::Level, Vec<[f32; 4]>) {
        (
            cherenkov_cpu::Level::new(),
            cases::conversion_pixels(len, false),
        )
    }
    fn decode_setup(len: usize) -> (cherenkov_cpu::Level, Vec<[f32; 4]>) {
        (
            cherenkov_cpu::Level::new(),
            cases::conversion_pixels(len, true),
        )
    }

    #[library_benchmark]
    #[benches::lengths(iter = cases::LENS.iter().copied(), setup = encode_setup)]
    fn encode((level, mut pixels): (cherenkov_cpu::Level, Vec<[f32; 4]>)) -> Vec<[f32; 4]> {
        bench_kernels::convert_span(
            level,
            black_box(&mut pixels),
            bench_kernels::BlendSpace::Linear,
            bench_kernels::BlendSpace::SrgbEncoded,
        );
        black_box(pixels)
    }

    #[library_benchmark]
    #[benches::lengths(iter = cases::LENS.iter().copied(), setup = decode_setup)]
    fn decode((level, mut pixels): (cherenkov_cpu::Level, Vec<[f32; 4]>)) -> Vec<[f32; 4]> {
        bench_kernels::convert_span(
            level,
            black_box(&mut pixels),
            bench_kernels::BlendSpace::SrgbEncoded,
            bench_kernels::BlendSpace::Linear,
        );
        black_box(pixels)
    }

    library_benchmark_group!(
        name = kernels,
        benchmarks = [
            eval,
            composite,
            opaque_store,
            encode,
            decode,
            uniform_store,
            row_reader,
            geometry_change
        ]
    );
}

#[cfg(not(target_arch = "wasm32"))]
use gungraun::main;
#[cfg(not(target_arch = "wasm32"))]
use kernels_bench::kernels;

#[cfg(not(target_arch = "wasm32"))]
main!(library_benchmark_groups = kernels);

// `harness = false` still needs a `main` to link on wasm32.
#[cfg(target_arch = "wasm32")]
fn main() {}
