//! Test filters for the stage declarations no built-in uses yet, and the
//! shape fixtures both executors' tests check them on.

extern crate alloc;

use alloc::vec::Vec;

use crate::{
    Filter, Footprint, OperatingSpace, Placed, ShapeInput, SpatialFilter, SpatialStage,
    StageCollector, kind,
};

/// Writes the shape's signed distance field to red.
pub struct SdfReadout;

impl Filter for SdfReadout {
    type Kind = kind::Spatial;
    type Params = [f32; 0];

    fn params(&self) -> [f32; 0] {
        []
    }

    fn collect_stages<C: StageCollector>(&self, collector: &mut C) {
        const STAGE: SpatialStage = SpatialStage {
            name: "sdf_readout",
            source: include_str!("executor/test_snippets/sdf_readout.wgsl"),
            source_f16: None,
            params: &[],
            space: OperatingSpace::Working,
            shape: Some(ShapeInput::Sdf),
            aux: &[],
        };
        collector.spatial(Placed::new(&STAGE));
    }
}

impl SpatialFilter for SdfReadout {
    fn footprint_of(_params: &[f32; 0]) -> Footprint {
        Footprint::ZERO
    }
}

/// Scales the input by the shape's mask.
pub struct Masked;

impl Filter for Masked {
    type Kind = kind::Spatial;
    type Params = [f32; 0];

    fn params(&self) -> [f32; 0] {
        []
    }

    fn collect_stages<C: StageCollector>(&self, collector: &mut C) {
        const STAGE: SpatialStage = SpatialStage {
            name: "masked",
            source: include_str!("executor/test_snippets/masked.wgsl"),
            source_f16: None,
            params: &[],
            space: OperatingSpace::Working,
            shape: Some(ShapeInput::Mask),
            aux: &[],
        };
        collector.spatial(Placed::new(&STAGE));
    }
}

impl SpatialFilter for Masked {
    fn footprint_of(_params: &[f32; 0]) -> Footprint {
        Footprint::ZERO
    }
}

/// The disc a shape field is checked on: its centre and radius in pixels.
pub const DISC: ([f32; 2], f32) = ([20.0, 22.0], 13.5);

/// [`DISC`]'s anti-aliased coverage over `size`, supersampled 8 × 8 per
/// pixel.
#[expect(
    clippy::cast_precision_loss,
    reason = "test coordinates are small integers"
)]
pub fn disc_coverage(size: (u32, u32)) -> Vec<f32> {
    const SAMPLES: u32 = 8;
    let (center, radius) = DISC;
    (0..size.1)
        .flat_map(|y| (0..size.0).map(move |x| (x, y)))
        .map(|(x, y)| {
            let inside = (0..SAMPLES * SAMPLES)
                .filter(|sample| {
                    let px = x as f32 + ((sample % SAMPLES) as f32 + 0.5) / SAMPLES as f32;
                    let py = y as f32 + ((sample / SAMPLES) as f32 + 0.5) / SAMPLES as f32;
                    (px - center[0]).hypot(py - center[1]) < radius
                })
                .count();
            inside as f32 / (SAMPLES * SAMPLES) as f32
        })
        .collect()
}

/// The signed distance from `(x, y)` to [`DISC`]'s edge.
pub fn disc_distance(x: f32, y: f32) -> f32 {
    let (center, radius) = DISC;
    (x - center[0]).hypot(y - center[1]) - radius
}

/// Asserts `field` is within `tolerance` pixels of `expected` at every
/// pixel centre.
pub fn assert_field_close(
    size: (u32, u32),
    field: &[f32],
    tolerance: f32,
    expected: impl Fn(f32, f32) -> f32,
) {
    for y in 0..size.1 {
        for x in 0..size.0 {
            let got = field[usize::try_from(y * size.0 + x).expect("fits usize")];
            #[expect(
                clippy::cast_precision_loss,
                reason = "test coordinates are small integers"
            )]
            let want = expected(x as f32 + 0.5, y as f32 + 0.5);
            assert!(
                (got - want).abs() <= tolerance,
                "pixel ({x}, {y}): distance {got}, expected {want}"
            );
        }
    }
}
