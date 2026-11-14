//! Filters without a CPU kernel, run from their stage description on the
//! lane executor (`CpuEffect`), and the display scale every CPU filter's
//! lengths run at.

#![cfg(all(feature = "shaders", not(target_arch = "wasm32")))]

use cherenkov::kurbo::Rect;
use cherenkov::{
    Display, Draw, Engine, FrameTime, Offscreen, OffscreenFormat, Surface, WorkingColor,
};
use cherenkov_cpu::{CpuEffect, Raster, RasterConfig};
use cherenkov_oracle::F32Image;
use cherenkov_oracle::metrics::assert_images_match;
use filtrate::{CpuFilter, CpuImage, Filter, LaneProgram, ParamArray, WorkingSpace, filters};

#[path = "support/visual.rs"]
mod visual;

const SIZE: (usize, usize) = (32, 96);

fn engine() -> Engine<Raster> {
    Engine::<Raster>::new(RasterConfig::default()).expect("engine")
}

fn surface(engine: &Engine<Raster>) -> Surface<Raster> {
    #[expect(clippy::cast_possible_truncation, reason = "a small test surface")]
    let size = (SIZE.0 as u32, SIZE.1 as u32);
    engine
        .surface(Offscreen::new(size, OffscreenFormat::LinearF32), || {})
        .expect("surface")
}

/// Thin bars on the band boundaries, where a band without a footprint's
/// apron would read clamped rows instead of its neighbour's.
fn bars(surface: &Surface<Raster>) -> cherenkov::Content {
    surface.record(|r| {
        for y in [15.0, 16.0, 31.0, 32.0, 47.0, 48.0, 63.0, 64.0, 79.0, 80.0] {
            r.fill(
                Rect::new(2.0, y, 30.0, y + 1.0),
                WorkingColor::new([0.9, 0.6, 0.2, 0.8]),
            );
        }
    })
}

/// `filter`'s described stages applied to the whole image.
fn described<F: Filter>(filter: &F, pixels: &mut [[f32; 4]]) {
    let mut params = vec![0.0; <F::Params as ParamArray>::LEN];
    filter.params().write_to(&mut params);
    LaneProgram::new(filter)
        .expect("the filter runs on the lane executor")
        .apply(
            &params,
            &WorkingSpace::LINEAR_DISPLAY_P3,
            &mut CpuImage {
                pixels,
                top: 0,
                size: SIZE,
            },
        );
}

fn assert_matches(expected: Vec<[f32; 4]>, surface: &Surface<Raster>) {
    let actual = surface.readback().expect("filtered readback");
    assert_images_match(
        &F32Image {
            width: actual.width,
            height: actual.height,
            pixels: expected,
        },
        &visual::image(&actual),
    );
}

#[test]
fn described_filters_match_their_full_surface_application_at_band_edges() {
    let engine = engine();
    let unfiltered = surface(&engine);
    let filtered = surface(&engine);
    // Two spatial passes, the second reading the first one's input, over
    // a spatial stage without a CPU kernel.
    let bloom = filters::Bloom {
        radius: 3.0_f32,
        intensity: 0.9,
        threshold: 0.2,
    };
    let median = filters::Median3x3;
    let outer_filter = engine.effect(CpuEffect::new(bloom.clone()).expect("bloom runs"));
    let inner_filter = engine.effect(CpuEffect::new(median).expect("median runs"));
    unfiltered.update(|tx| {
        tx[unfiltered.root()].content(bars(&unfiltered));
    });
    let outer = filtered.layer();
    let inner = filtered.layer();
    filtered.update(|tx| {
        tx[filtered.root()].push(&outer);
        tx[&outer].push(&inner).filter(outer_filter.id());
        tx[&inner]
            .filter(inner_filter.id())
            .content(bars(&filtered));
    });
    engine.render(FrameTime::now()).expect("render");

    let mut expected = unfiltered.readback().expect("source readback").pixels;
    described(&median, &mut expected);
    described(&bloom, &mut expected);
    assert_matches(expected, &filtered);
}

#[test]
fn a_described_colour_filter_runs_on_a_layer() {
    let engine = engine();
    let unfiltered = surface(&engine);
    let filtered = surface(&engine);
    let sepia = filters::Sepia(0.7_f32);
    let filter = engine.effect(CpuEffect::new(sepia).expect("sepia runs"));
    unfiltered.update(|tx| {
        tx[unfiltered.root()].content(bars(&unfiltered));
    });
    let layer = filtered.layer();
    filtered.update(|tx| {
        tx[filtered.root()].push(&layer);
        tx[&layer].filter(filter.id()).content(bars(&filtered));
    });
    engine.render(FrameTime::now()).expect("render");

    let mut expected = unfiltered.readback().expect("source readback").pixels;
    described(&sepia, &mut expected);
    assert_matches(expected, &filtered);
}

/// A filter's lengths are points: at display scale 2 a 2-point blur
/// spreads 4 pixels, for a kernel filter and a described one alike.
#[test]
fn filter_lengths_run_at_the_display_scale() {
    let engine = engine();
    let unfiltered = surface(&engine);
    let kernel_surface = surface(&engine);
    let described_surface = surface(&engine);
    let display = Display {
        scale: 2.0,
        headroom: 1.0,
    };
    for surface in [&unfiltered, &kernel_surface, &described_surface] {
        surface.display(display).expect("display");
    }
    let kernel = engine.filter(filters::Blur(2.0_f32));
    let effect = engine.effect(CpuEffect::new(filters::Blur(2.0_f32)).expect("blur runs"));
    unfiltered.update(|tx| {
        tx[unfiltered.root()].content(bars(&unfiltered));
    });
    // The layers live to the readback: dropping one removes it.
    let layers =
        [(&kernel_surface, &kernel), (&described_surface, &effect)].map(|(surface, filter)| {
            let layer = surface.layer();
            surface.update(|tx| {
                tx[surface.root()].push(&layer);
                tx[&layer].filter(filter.id()).content(bars(surface));
            });
            layer
        });
    engine.render(FrameTime::now()).expect("render");

    let mut expected = unfiltered.readback().expect("source readback").pixels;
    let pixels = filters::Blur(4.0_f32);
    pixels
        .apply_cpu_image(
            &pixels.params(),
            &WorkingSpace::LINEAR_DISPLAY_P3,
            &mut CpuImage {
                pixels: &mut expected,
                top: 0,
                size: SIZE,
            },
        )
        .expect("CPU box blur");
    assert_matches(expected.clone(), &kernel_surface);
    assert_matches(expected, &described_surface);
    drop(layers);
}
