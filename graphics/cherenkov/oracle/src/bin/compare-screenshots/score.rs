use std::{io, num::NonZeroU32};

use cherenkov_oracle::{
    F32Image,
    metrics::{
        self, DEFAULT_PPD, FLIP_MAX_LIMIT, FLIP_MEAN_LIMIT, FlipFeatures, HDR_EXPOSURES,
        LOCAL_ERROR_LIMIT, Overlap,
    },
};
use serde::Serialize;

use super::Error;

/// How far, in whole pixels along each axis, a `SwiftUI` twin may be displaced
/// against its `WaterUI` capture before FLIP compares them: two layouts of one
/// screen place an edge up to 2 px apart without that being drift.
pub const TWIN_SHIFT_RADIUS: u32 = 2;

/// Side, in points, of the square window over which
/// [`Score::max_block_flip_mean`] averages a twin's FLIP map. An element that
/// is missing, extra or moved beyond the shift radius fills the window with
/// error; glyph rasterization along an aligned edge does not. The window is
/// sized in points, not pixels, because the noise two text stacks leave on
/// one glyph run scales with the display: at 3x the same 8 px window holds a
/// third of a glyph and rasterization differences alone reach 0.40, while a
/// misplaced word in a 1x capture reaches only 0.41.
pub const TWIN_BLOCK_PT: NonZeroU32 = NonZeroU32::new(8).unwrap();

/// `flip_mean` ceiling for a `SwiftUI` twin.
///
/// Calibrated on the nightly captures of every registered twin on iOS (3x)
/// and macOS (1x), 19 pairs equivalent by eye and 21 drifted. The
/// equivalent pairs reach 0.0120 (ios-hover); the drifted pairs this metric
/// alone must catch, whose drift is spread across the frame rather than
/// concentrated in one element (mirrored right-to-left typography), start at
/// 0.0285. The ceiling sits between them with a margin of at least 1.5x on
/// each side.
pub const TWIN_FLIP_MEAN_LIMIT: f64 = 0.018;

/// `max_block_flip_mean` ceiling for a `SwiftUI` twin.
///
/// On the same calibration set the equivalent pairs reach 0.323 (ios-shape;
/// 0.320 on macOS, macos-locale) and the drifted pairs not already over
/// [`TWIN_FLIP_MEAN_LIMIT`] start at 0.411 (macos-edge_text, whitespace
/// dropped before a word) and 0.416 (ios-picker, wheel rows at a different
/// pitch), a 27% gap. The ceiling sits near its geometric middle.
pub const TWIN_BLOCK_FLIP_LIMIT: f64 = 0.36;

/// The acceptance rule a comparison is gated by.
pub trait Limits: Serialize {
    /// How far, in whole pixels along each axis, the actual image is
    /// displaced against the reference in search of the best alignment.
    fn shift_radius(&self) -> u32;
    /// Side, in points, of the window the block score averages over, when
    /// the rule judges one.
    fn block_pt(&self) -> Option<NonZeroU32>;
    fn accepts(&self, score: &Score) -> bool;
}

/// Route equivalence: two routes to the same pixels, judged by the oracle's
/// limits on the unshifted images. Frame settling and stored-baseline
/// comparisons use these.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct RouteLimits {
    pub flip_mean: f64,
    pub flip_max: f64,
    pub max_local_error: f64,
}

impl RouteLimits {
    pub const ORACLE: Self = Self {
        flip_mean: FLIP_MEAN_LIMIT,
        flip_max: FLIP_MAX_LIMIT,
        max_local_error: LOCAL_ERROR_LIMIT,
    };
}

impl Limits for RouteLimits {
    fn shift_radius(&self) -> u32 {
        0
    }

    fn block_pt(&self) -> Option<NonZeroU32> {
        None
    }

    fn accepts(&self, score: &Score) -> bool {
        score.flip_mean <= self.flip_mean
            && score.flip_max <= self.flip_max
            && score.max_local_error <= self.max_local_error
    }
}

/// Twin parity: a `WaterUI` screen against its `SwiftUI` twin, two different
/// implementations. Edge offsets up to the shift radius are not drift;
/// missing, extra or clearly misplaced elements are. Judged by FLIP alone,
/// after alignment: a whole-frame mean ceiling and a ceiling on the
/// block-averaged FLIP map.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct TwinLimits {
    pub shift_radius_px: u32,
    pub block_pt: NonZeroU32,
    pub flip_mean: f64,
    pub max_block_flip_mean: f64,
}

impl TwinLimits {
    pub const CALIBRATED: Self = Self {
        shift_radius_px: TWIN_SHIFT_RADIUS,
        block_pt: TWIN_BLOCK_PT,
        flip_mean: TWIN_FLIP_MEAN_LIMIT,
        max_block_flip_mean: TWIN_BLOCK_FLIP_LIMIT,
    };
}

impl Limits for TwinLimits {
    fn shift_radius(&self) -> u32 {
        self.shift_radius_px
    }

    fn block_pt(&self) -> Option<NonZeroU32> {
        Some(self.block_pt)
    }

    fn accepts(&self, score: &Score) -> bool {
        let block = score
            .max_block_flip_mean
            .expect("a twin comparison measures its block score");
        score.flip_mean <= self.flip_mean && block <= self.max_block_flip_mean
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Metric {
    LdrFlip,
    HdrFlip,
}

/// FLIP scores of the actual image against the reference. Every whole-pixel
/// displacement of the actual image within `shift_radius_px` is compared over
/// the rectangle where both images exist. `flip_mean`, `flip_max` and the
/// heatmap come from the single displacement with the lowest mean; the block
/// score, measured when the limits judge one, aligns each window on its own,
/// since two layouts of one screen can offset different regions in different
/// directions.
#[derive(Debug, Serialize)]
pub struct Score {
    pub metric: Metric,
    pub flip_alpha_comparison: &'static str,
    pub shift_radius_px: u32,
    /// The displacement `[dx, dy]` of the actual image with the lowest
    /// `flip_mean`: reference pixel `(x, y)` is compared with actual pixel
    /// `(x + dx, y + dy)`.
    pub flip_shift_px: [isize; 2],
    pub flip_mean: f64,
    pub flip_max: f64,
    /// The oracle's 3x3 box-filtered difference of the unshifted images, a
    /// route metric.
    pub max_local_error: f64,
    /// The largest, over every `flip_block_px`-sided window of the reference
    /// frame, of that window's lowest mean FLIP error over every displacement
    /// whose overlap contains it.
    pub max_block_flip_mean: Option<f64>,
    pub flip_block_px: Option<NonZeroU32>,
    pub pixels_per_degree: f64,
    pub hdr_flip_exposure_ev: Vec<f64>,
}

/// The magma-mapped FLIP map of the best displacement, over its overlap.
pub struct Heatmap {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// Both images prepared for FLIP over black, and over white when either is
/// transparent: FLIP compares transparent inputs over both mattes and keeps
/// the larger per-pixel error.
struct Mattes {
    black: [FlipFeatures; 2],
    white: Option<[FlipFeatures; 2]>,
}

impl Mattes {
    fn error(&self, offset: [isize; 2], error: &mut Vec<f64>, scratch: &mut Vec<f64>) -> Overlap {
        let [reference, actual] = &self.black;
        let overlap = reference.offset_error(actual, offset, error);
        if let Some([reference, actual]) = &self.white {
            reference.offset_error(actual, offset, scratch);
            for (error, white) in error.iter_mut().zip(scratch.iter()) {
                *error = error.max(*white);
            }
        }
        overlap
    }
}

/// The FLIP map of one displacement.
struct Aligned {
    offset: [isize; 2],
    overlap: Overlap,
    mean: f64,
    error: Vec<f64>,
}

pub fn compare(
    reference: &F32Image,
    actual: &F32Image,
    shift_radius: u32,
    block_px: Option<NonZeroU32>,
) -> Result<(Score, Heatmap), Error> {
    if (reference.width, reference.height) != (actual.width, actual.height) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "image size mismatch: reference {}x{}, actual {}x{}",
                reference.width, reference.height, actual.width, actual.height
            ),
        )
        .into());
    }
    let smallest = block_px.map_or(1, NonZeroU32::get) + 2 * shift_radius;
    if reference.width < smallest || reference.height < smallest {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "image {}x{} is smaller than {smallest} px, a {} px FLIP block \
                 displaced by {shift_radius} px each way",
                reference.width,
                reference.height,
                block_px.map_or(1, NonZeroU32::get)
            ),
        )
        .into());
    }
    let width = reference.width as usize;
    let height = reference.height as usize;
    let (mattes, hdr) = prepare(reference, actual);
    let radius = isize::try_from(shift_radius)?;
    let mut offsets: Vec<[isize; 2]> = (-radius..=radius)
        .flat_map(|dy| (-radius..=radius).map(move |dx| [dx, dy]))
        .collect();
    // The unshifted comparison goes first, so it wins every tie.
    offsets.sort_by_key(|[dx, dy]| dx.abs() + dy.abs());
    let mut blocks = block_px.map(|block| BlockMinima::new(block, width, height));
    let mut error = Vec::new();
    let mut scratch = Vec::new();
    let mut best: Option<Aligned> = None;
    for offset in offsets {
        let overlap = mattes.error(offset, &mut error, &mut scratch);
        if let Some(blocks) = &mut blocks {
            blocks.update(&error, overlap);
        }
        let mean = mean(&error);
        if best.as_ref().is_none_or(|best| mean < best.mean) {
            let replaced = best.replace(Aligned {
                offset,
                overlap,
                mean,
                error: std::mem::take(&mut error),
            });
            if let Some(replaced) = replaced {
                error = replaced.error;
            }
        }
    }
    let best = best.expect("the unshifted comparison is always made");
    let score = Score {
        metric: if hdr {
            Metric::HdrFlip
        } else {
            Metric::LdrFlip
        },
        flip_alpha_comparison: "max-over-black-and-white",
        shift_radius_px: shift_radius,
        flip_shift_px: best.offset,
        flip_mean: best.mean,
        flip_max: best.error.iter().copied().fold(0.0, f64::max),
        max_local_error: metrics::max_local_error(reference, actual),
        max_block_flip_mean: blocks.as_ref().map(BlockMinima::max),
        flip_block_px: block_px,
        pixels_per_degree: DEFAULT_PPD,
        hdr_flip_exposure_ev: if hdr {
            HDR_EXPOSURES.to_vec()
        } else {
            Vec::new()
        },
    };
    let heatmap = Heatmap {
        width: u32::try_from(best.overlap.width)?,
        height: u32::try_from(best.overlap.height)?,
        rgb: metrics::heat_map(&best.error)
            .into_iter()
            .flatten()
            .collect(),
    };
    Ok((score, heatmap))
}

/// Both images' FLIP features, and whether HDR-FLIP measures them.
fn prepare(reference: &F32Image, actual: &F32Image) -> (Mattes, bool) {
    let reference_rgb = metrics::to_linear_p3(reference);
    let actual_rgb = metrics::to_linear_p3(actual);
    // Both mattes are identical for opaque images, so do not repeat FLIP.
    let white = reference
        .pixels
        .iter()
        .chain(&actual.pixels)
        .any(|pixel| pixel[3] < 1.0)
        .then(|| [over_white(reference), over_white(actual)]);
    let (reference_range, actual_range) = white
        .as_ref()
        .map_or((&reference_rgb, &actual_rgb), |[reference, actual]| {
            (reference, actual)
        });
    let hdr = metrics::is_hdr(reference_range) || metrics::is_hdr(actual_range);
    let (width, height) = (reference.width as usize, reference.height as usize);
    let features = |rgb: &[[f64; 3]]| FlipFeatures::new(rgb, width, height, DEFAULT_PPD, hdr);
    let mattes = Mattes {
        black: [features(&reference_rgb), features(&actual_rgb)],
        white: white.map(|[reference, actual]| [features(&reference), features(&actual)]),
    };
    (mattes, hdr)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "pixel counts are far below 2^53"
)]
fn mean(error: &[f64]) -> f64 {
    error.iter().sum::<f64>() / error.len() as f64
}

/// Per `block`-sided window of the reference frame, the lowest mean FLIP
/// error over every displacement whose overlap contains it.
struct BlockMinima {
    block: usize,
    columns: usize,
    minima: Vec<f64>,
    table: Vec<f64>,
}

impl BlockMinima {
    fn new(block: NonZeroU32, width: usize, height: usize) -> Self {
        let block = block.get() as usize;
        let columns = width - block + 1;
        Self {
            block,
            columns,
            minima: vec![f64::INFINITY; columns * (height - block + 1)],
            table: Vec::new(),
        }
    }

    /// Fold in the FLIP map of one displacement, which covers `overlap`,
    /// through a summed-area table of it.
    fn update(&mut self, error: &[f64], overlap: Overlap) {
        let block = self.block;
        let stride = overlap.width + 1;
        self.table.clear();
        self.table.resize(stride * (overlap.height + 1), 0.0);
        for (y, row) in error.chunks_exact(overlap.width).enumerate() {
            let mut sum = 0.0;
            for (x, value) in row.iter().enumerate() {
                sum += value;
                self.table[(y + 1) * stride + x + 1] = self.table[y * stride + x + 1] + sum;
            }
        }
        let table = &self.table;
        #[expect(
            clippy::cast_precision_loss,
            reason = "a block holds far fewer than 2^53 pixels"
        )]
        let area = (block * block) as f64;
        for y in 0..=overlap.height - block {
            let row = (overlap.y + y) * self.columns + overlap.x;
            let minima = &mut self.minima[row..=row + overlap.width - block];
            for (x, minimum) in minima.iter_mut().enumerate() {
                let (top, bottom) = (y * stride, (y + block) * stride);
                let sum = table[bottom + x + block] - table[top + x + block] - table[bottom + x]
                    + table[top + x];
                *minimum = minimum.min(sum / area);
            }
        }
    }

    fn max(&self) -> f64 {
        self.minima.iter().copied().fold(0.0, f64::max)
    }
}

fn over_white(image: &F32Image) -> Vec<[f64; 3]> {
    image
        .pixels
        .iter()
        .map(|pixel| {
            let background = 1.0 - f64::from(pixel[3]);
            [pixel[0], pixel[1], pixel[2]].map(|c| f64::from(c) + background)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use cherenkov_oracle::{F32Image, metrics::LOCAL_ERROR_LIMIT};

    use super::{Limits, Metric, RouteLimits, Score, TwinLimits, compare};

    fn solid(value: f32) -> F32Image {
        F32Image {
            width: 32,
            height: 32,
            pixels: vec![[value, value, value, 1.0]; 32 * 32],
        }
    }

    /// Scores a pair captured at 1x, where a point is a pixel.
    fn score(reference: &F32Image, actual: &F32Image, limits: &impl Limits) -> Score {
        compare(reference, actual, limits.shift_radius(), limits.block_pt())
            .unwrap()
            .0
    }

    #[test]
    fn uniform_images_are_valid_comparison_inputs() {
        let image = solid(0.25);
        let mut score = score(&image, &image, &RouteLimits::ORACLE);
        assert!(RouteLimits::ORACLE.accepts(&score), "{score:?}");
        score.max_local_error = 2.0 * LOCAL_ERROR_LIMIT;
        assert!(!RouteLimits::ORACLE.accepts(&score), "{score:?}");
    }

    #[test]
    fn transparent_black_is_not_opaque_black() {
        let reference = solid(0.0);
        let mut actual = reference.clone();
        actual.pixels.fill([0.0; 4]);
        let score = score(&reference, &actual, &RouteLimits::ORACLE);
        assert!(!RouteLimits::ORACLE.accepts(&score), "{score:?}");
    }

    #[test]
    fn a_changed_bottom_band_is_not_a_platform_exemption() {
        let reference = solid(1.0);
        let mut actual = reference.clone();
        actual.pixels[24 * 32..].fill([0.0, 0.0, 0.0, 1.0]);
        let score = score(&reference, &actual, &RouteLimits::ORACLE);
        assert!(!RouteLimits::ORACLE.accepts(&score), "{score:?}");
    }

    #[test]
    fn hdr_on_either_side_selects_hdr_flip() {
        let reference = solid(1.0);
        let actual = solid(4.0);
        let score = score(&reference, &actual, &RouteLimits::ORACLE);
        assert!(matches!(score.metric, Metric::HdrFlip));
        assert!(!RouteLimits::ORACLE.accepts(&score), "{score:?}");
    }

    #[test]
    fn dimensions_are_an_input_error() {
        let reference = solid(0.25);
        let mut actual = solid(0.25);
        actual.width = 16;
        actual.pixels.truncate(16 * 32);
        assert!(compare(&reference, &actual, 0, None).is_err());
    }

    /// A white 256×256 frame with dark 2 px vertical strokes 12 px apart,
    /// like a line of glyphs, moved right by `stroke_shift`, and a dark
    /// 16×16 block at `block_x`.
    fn twin_scene(stroke_shift: u32, block_x: Option<u32>) -> F32Image {
        let side = 256;
        let mut pixels = vec![[1.0f32, 1.0, 1.0, 1.0]; (side * side) as usize];
        for y in 8..20 {
            for x in (8..88).step_by(12).flat_map(|x| [x, x + 1]) {
                pixels[(y * side + x + stroke_shift) as usize] = [0.0, 0.0, 0.0, 1.0];
            }
        }
        if let Some(block_x) = block_x {
            for y in 48..64 {
                for x in block_x..block_x + 16 {
                    pixels[(y * side + x) as usize] = [0.0, 0.0, 0.0, 1.0];
                }
            }
        }
        F32Image {
            width: side,
            height: side,
            pixels,
        }
    }

    #[test]
    fn twin_limits_accept_regions_offset_within_the_shift_radius() {
        let reference = twin_scene(0, Some(40));
        // The strokes move right and the block left, so no single
        // displacement aligns both.
        let actual = twin_scene(2, Some(38));
        assert!(!RouteLimits::ORACLE.accepts(&score(&reference, &actual, &RouteLimits::ORACLE)));
        let twin = score(&reference, &actual, &TwinLimits::CALIBRATED);
        assert!(TwinLimits::CALIBRATED.accepts(&twin), "{twin:?}");
    }

    #[test]
    fn twin_limits_reject_missing_or_displaced_elements() {
        let reference = twin_scene(0, Some(40));
        for actual in [
            twin_scene(6, Some(40)),
            twin_scene(0, None),
            twin_scene(0, Some(64)),
        ] {
            let twin = score(&reference, &actual, &TwinLimits::CALIBRATED);
            assert!(!TwinLimits::CALIBRATED.accepts(&twin), "{twin:?}");
        }
    }
}
