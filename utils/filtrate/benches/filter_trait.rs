//! CPU-only Filter trait benchmarks.
//!
//! These benches exercise the public surface of `filtrate-core::Filter`
//! (params snapshot, stage collection, signal visitation) on representative
//! built-in filters and chains. They intentionally do not touch wgpu, so
//! they can run in any CI environment and serve as a regression baseline
//! for trait-shape changes.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use filtrate::filters::{Blur, Brightness, ColorMatrix, Contrast, Saturation, Sepia};
use filtrate::{Filter, FilterExt, ParamArray, SignalVisitor, StageCollector};
use std::hint;

/// No-op stage collector — measures pure dispatch cost.
struct NoopStages;
impl StageCollector for NoopStages {
    fn color_fragment(&mut self, source: &'static str, param_count: usize) {
        hint::black_box(source);
        hint::black_box(param_count);
    }
    fn spatial_shader(&mut self, source: &'static str, param_count: usize) {
        hint::black_box(source);
        hint::black_box(param_count);
    }
}

/// No-op signal visitor — measures pure dispatch cost.
struct NoopVisitor;
impl SignalVisitor for NoopVisitor {
    fn visit<P: filtrate::FilterParam + ?Sized>(&mut self, idx: usize, param: &P) {
        hint::black_box(idx);
        hint::black_box(param.snapshot());
    }
}

fn bench_single_color(c: &mut Criterion) {
    let f = Brightness(0.42_f32);

    c.bench_function("Brightness::params", |b| {
        b.iter(|| {
            let p = f.params();
            black_box(p);
        });
    });

    c.bench_function("Brightness::collect_stages", |b| {
        b.iter(|| {
            let mut sink = NoopStages;
            f.collect_stages(&mut sink);
            black_box(&sink);
        });
    });

    c.bench_function("Brightness::visit_signals", |b| {
        b.iter(|| {
            let mut visitor = NoopVisitor;
            f.visit_signals(&mut visitor);
            black_box(&visitor);
        });
    });
}

fn bench_chain(c: &mut Criterion) {
    let chain = Brightness(0.1_f32)
        .then(Contrast(1.2_f32))
        .then(Saturation(1.3_f32))
        .then(Sepia(0.4_f32));

    c.bench_function("Chain<4 color>::params", |b| {
        b.iter(|| {
            let p = chain.params();
            black_box(p);
        });
    });

    c.bench_function("Chain<4 color>::collect_stages", |b| {
        b.iter(|| {
            let mut sink = NoopStages;
            chain.collect_stages(&mut sink);
            black_box(&sink);
        });
    });

    c.bench_function("Chain<4 color>::visit_signals", |b| {
        b.iter(|| {
            let mut visitor = NoopVisitor;
            chain.visit_signals(&mut visitor);
            black_box(&visitor);
        });
    });
}

fn bench_color_then_spatial(c: &mut Criterion) {
    let chain = Brightness(0.1_f32).then(Blur(5.0_f32));

    c.bench_function("Brightness.then(Blur)::collect_stages", |b| {
        b.iter(|| {
            let mut sink = NoopStages;
            chain.collect_stages(&mut sink);
            black_box(&sink);
        });
    });

    c.bench_function("Brightness.then(Blur)::pass_count", |b| {
        b.iter(|| {
            let n = chain.pass_count();
            black_box(n);
        });
    });
}

fn bench_color_matrix(c: &mut Criterion) {
    let f = ColorMatrix([
        1.0_f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ]);

    c.bench_function("ColorMatrix::params", |b| {
        b.iter(|| {
            let p = f.params();
            black_box(p);
        });
    });

    c.bench_function("ColorMatrix::visit_signals", |b| {
        b.iter(|| {
            let mut visitor = NoopVisitor;
            f.visit_signals(&mut visitor);
            black_box(&visitor);
        });
    });
}

fn bench_param_array_len(c: &mut Criterion) {
    type ChainParams = <filtrate_core::Chain<
        Brightness<f32>,
        filtrate_core::Chain<Contrast<f32>, Saturation<f32>>,
    > as Filter>::Params;

    c.bench_function("ParamArray::LEN compile-time", |b| {
        b.iter(|| {
            let n: usize = <ChainParams as ParamArray>::LEN;
            black_box(n);
        });
    });
}

// Wrapping the macro-generated `benches` item in a private module hides it
// from `missing_docs` without needing a crate-wide allow attribute.
mod groups {
    use super::*;

    criterion_group!(
        benches,
        bench_single_color,
        bench_chain,
        bench_color_then_spatial,
        bench_color_matrix,
        bench_param_array_len,
    );
}
criterion_main!(groups::benches);
