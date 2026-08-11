//! CPU-only Filter trait benchmarks.
//!
//! These benches exercise the public surface of `filtrate-core::Filter`
//! (params snapshot, stage collection, signal visitation) on representative
//! built-in filters and chains. They intentionally do not touch wgpu, so
//! they run in any CI environment and serve as a regression baseline for
//! trait-shape changes.
//!
//! Run with:
//!
//! ```text
//! cargo bench -p filtrate --bench filter_trait
//! ```

use divan::Bencher;
use filtrate::filters::{
    Bloom, Blur, Brightness, ColorMatrix, Contrast, Convolution3x3, Convolution5x5, Median3x3,
    MorphologyGradient, MorphologyMax, MorphologyMin, PhotoEffectChrome, PhotoEffectMono,
    PhotoEffectNoir, Prewitt, Saturation, Sepia, Sobel,
};
use filtrate::{Filter, FilterExt, ParamArray, SignalVisitor, StageCollector};

fn main() {
    divan::main();
}

/// No-op stage collector — measures pure dispatch cost.
struct NoopStages;
impl StageCollector for NoopStages {
    fn color_fragment(&mut self, source: &'static str, param_count: usize) {
        divan::black_box(source);
        divan::black_box(param_count);
    }
    fn spatial_shader(&mut self, source: &'static str, param_count: usize) {
        divan::black_box(source);
        divan::black_box(param_count);
    }
    fn spatial_shader_with_original(&mut self, source: &'static str, param_count: usize) {
        divan::black_box(source);
        divan::black_box(param_count);
    }
}

/// No-op signal visitor — measures pure dispatch cost.
struct NoopVisitor;
impl SignalVisitor for NoopVisitor {
    fn visit<P: filtrate::FilterParam + ?Sized>(&mut self, idx: usize, param: &P) {
        divan::black_box(idx);
        divan::black_box(param.snapshot());
    }
}

// ----------------------------------------------------------------------------
// Single filters: params / collect_stages / visit_signals
// ----------------------------------------------------------------------------

#[divan::bench]
fn brightness_params(b: Bencher) {
    let f = Brightness(0.42_f32);
    b.bench_local(|| divan::black_box(f.params()));
}

#[divan::bench]
fn brightness_collect_stages(b: Bencher) {
    let f = Brightness(0.42_f32);
    b.bench_local(|| {
        let mut sink = NoopStages;
        f.collect_stages(&mut sink);
        divan::black_box(&sink);
    });
}

#[divan::bench]
fn brightness_visit_signals(b: Bencher) {
    let f = Brightness(0.42_f32);
    b.bench_local(|| {
        let mut v = NoopVisitor;
        f.visit_signals(&mut v);
        divan::black_box(&v);
    });
}

// ----------------------------------------------------------------------------
// Chains: 4-deep color + color-then-spatial
// ----------------------------------------------------------------------------

#[divan::bench]
fn chain_4_color_params(b: Bencher) {
    let chain = Brightness(0.1_f32)
        .then(Contrast(1.2_f32))
        .then(Saturation(1.3_f32))
        .then(Sepia(0.4_f32));
    b.bench_local(|| divan::black_box(chain.params()));
}

#[divan::bench]
fn chain_4_color_collect_stages(b: Bencher) {
    let chain = Brightness(0.1_f32)
        .then(Contrast(1.2_f32))
        .then(Saturation(1.3_f32))
        .then(Sepia(0.4_f32));
    b.bench_local(|| {
        let mut sink = NoopStages;
        chain.collect_stages(&mut sink);
        divan::black_box(&sink);
    });
}

#[divan::bench]
fn chain_4_color_visit_signals(b: Bencher) {
    let chain = Brightness(0.1_f32)
        .then(Contrast(1.2_f32))
        .then(Saturation(1.3_f32))
        .then(Sepia(0.4_f32));
    b.bench_local(|| {
        let mut v = NoopVisitor;
        chain.visit_signals(&mut v);
        divan::black_box(&v);
    });
}

#[divan::bench]
fn brightness_then_blur_collect_stages(b: Bencher) {
    let chain = Brightness(0.1_f32).then(Blur(5.0_f32));
    b.bench_local(|| {
        let mut sink = NoopStages;
        chain.collect_stages(&mut sink);
        divan::black_box(&sink);
    });
}

// ----------------------------------------------------------------------------
// Array-style filter (ColorMatrix, 12 elements)
// ----------------------------------------------------------------------------

#[divan::bench]
fn color_matrix_params(b: Bencher) {
    let f = ColorMatrix([
        1.0_f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ]);
    b.bench_local(|| divan::black_box(f.params()));
}

#[divan::bench]
fn color_matrix_visit_signals(b: Bencher) {
    let f = ColorMatrix([
        1.0_f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
    ]);
    b.bench_local(|| {
        let mut v = NoopVisitor;
        f.visit_signals(&mut v);
        divan::black_box(&v);
    });
}

// ----------------------------------------------------------------------------
// Compile-time const lookup sanity (monomorphization stays const)
// ----------------------------------------------------------------------------

#[divan::bench]
fn param_array_len_const_lookup(b: Bencher) {
    type ChainParams = <filtrate_core::Chain<
        Brightness<f32>,
        filtrate_core::Chain<Contrast<f32>, Saturation<f32>>,
    > as Filter>::Params;
    b.bench_local(|| {
        let n: usize = <ChainParams as ParamArray>::LEN;
        divan::black_box(n);
    });
}

// ----------------------------------------------------------------------------
// P9 spatial filters (Sobel/Prewitt/Median/Morphology/Convolution/Bloom)
// ----------------------------------------------------------------------------

#[divan::bench(types = [Sobel, Prewitt, Median3x3, MorphologyMin, MorphologyMax, MorphologyGradient])]
fn p9_spatial_zero_param_collect_stages<F: Filter + Default>(b: Bencher) {
    let f = F::default();
    b.bench_local(|| {
        let mut sink = NoopStages;
        f.collect_stages(&mut sink);
        divan::black_box(&sink);
    });
}

#[divan::bench(types = [Sobel, Prewitt, Median3x3, MorphologyMin, MorphologyMax, MorphologyGradient])]
fn p9_spatial_zero_param_visit_signals<F: Filter + Default>(b: Bencher) {
    let f = F::default();
    b.bench_local(|| {
        let mut v = NoopVisitor;
        f.visit_signals(&mut v);
        divan::black_box(&v);
    });
}

#[divan::bench]
fn convolution3x3_params(b: Bencher) {
    let conv = Convolution3x3([0.0_f32, -1.0, 0.0, -1.0, 5.0, -1.0, 0.0, -1.0, 0.0]);
    b.bench_local(|| divan::black_box(conv.params()));
}

#[divan::bench]
fn convolution5x5_params(b: Bencher) {
    let mut k = [0.0_f32; 25];
    k[12] = 1.0;
    let conv = Convolution5x5(k);
    b.bench_local(|| divan::black_box(conv.params()));
}

#[divan::bench]
fn bloom_params(b: Bencher) {
    let f = Bloom {
        radius: 8.0_f32,
        intensity: 1.5,
        threshold: 0.7,
    };
    b.bench_local(|| divan::black_box(f.params()));
}

// ----------------------------------------------------------------------------
// P9 photo presets (zero-param color)
// ----------------------------------------------------------------------------

#[divan::bench(types = [PhotoEffectMono, PhotoEffectNoir, PhotoEffectChrome])]
fn photo_preset_collect_stages<F: Filter + Default>(b: Bencher) {
    let f = F::default();
    b.bench_local(|| {
        let mut sink = NoopStages;
        f.collect_stages(&mut sink);
        divan::black_box(&sink);
    });
}

// ----------------------------------------------------------------------------
// Cross-category chain: photo preset + tunable color filters (fusion path)
// ----------------------------------------------------------------------------

#[divan::bench]
fn photo_chain_params(b: Bencher) {
    let chain = PhotoEffectChrome
        .then(Brightness(0.05_f32))
        .then(Contrast(1.1_f32));
    b.bench_local(|| divan::black_box(chain.params()));
}

#[divan::bench]
fn photo_chain_collect_stages(b: Bencher) {
    let chain = PhotoEffectChrome
        .then(Brightness(0.05_f32))
        .then(Contrast(1.1_f32));
    b.bench_local(|| {
        let mut sink = NoopStages;
        chain.collect_stages(&mut sink);
        divan::black_box(&sink);
    });
}
