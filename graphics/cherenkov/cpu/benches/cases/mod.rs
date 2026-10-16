//! The span-kernel case table shared by `kernels_wall` and
//! `kernels_ir`: every paint kind × run length × start offset, evaluated
//! and composited, with deterministic inputs and no RNG. Paints are
//! `cherenkov::Paint` values lowered through the real `paint_data` —
//! the bench measures data the renderer actually produces.

use std::fmt;
use std::sync::Arc;

use cherenkov::kurbo::Affine;
use cherenkov::{
    ColorStop, Extend, ImageColorSpace, ImageFormat, ImageId, ImagePattern, ImageUpload,
    Interpolation, LinearGradient, Paint, RadialGradient, Sampling, SweepGradient,
    TransformedPaint, WorkingColor,
};
use cherenkov_cpu::testing::kernels::{self, ClipRun, PaintData};

/// The device row every case shades.
pub const PY: usize = 13;
/// The surface width the clip source reports a stride of.
pub const W: usize = 1024;
/// The registered image the pattern kinds sample.
const IMAGE: ImageId = ImageId::new(1);

/// Run lengths: short runs, SIMD-lane boundaries and a full surface row.
pub const LENS: &[usize] = &[1, 3, 4, 7, 8, 15, 16, 31, 64, 255, 1024];
/// Run start columns: aligned and remainder cases.
pub const OFFSETS: &[usize] = &[0, 1, 3];

/// A paint kind in the table.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// `Paint::Solid` at alpha 1.
    SolidOpaque,
    /// `Paint::Solid` below alpha 1.
    SolidTranslucent,
    /// `Paint::Linear`.
    Linear,
    /// `Paint::Radial` with distinct circles.
    Radial,
    /// `Paint::Radial` with identical circles (the hard edge).
    RadialIdentical,
    /// `Paint::Sweep`.
    Sweep,
    /// `Paint::Image` with nearest sampling.
    ImageNearest,
    /// `Paint::Image` with bilinear sampling.
    ImageBilinear,
    /// `Paint::Transformed` over a linear gradient.
    Transformed,
}

/// The gradient stops every gradient kind shares.
fn stops() -> Vec<ColorStop> {
    [
        (0.0, [0.9, 0.1, 0.2, 1.0]),
        (0.35, [0.1, 0.8, 0.3, 1.0]),
        (0.7, [0.2, 0.3, 0.9, 0.8]),
        (1.0, [0.05, 0.05, 0.4, 1.0]),
    ]
    .into_iter()
    .map(|(offset, color)| ColorStop {
        offset,
        color: WorkingColor::new(color),
    })
    .collect()
}

fn linear_gradient() -> LinearGradient {
    let mut gradient = LinearGradient::new((0.0, 0.0), (1024.0, 0.0));
    gradient.stops = stops();
    gradient
}

/// The deterministic 64×64 `Rgba8` upload the image kinds share.
fn upload() -> ImageUpload {
    let mut data = Vec::with_capacity(64 * 64 * 4);
    for y in 0..64u8 {
        for x in 0..64u8 {
            data.extend_from_slice(&[x * 4, y * 4, (x + y) * 2, 255]);
        }
    }
    ImageUpload {
        width: 64,
        height: 64,
        data: data.into(),
        color_space: ImageColorSpace::LinearP3,
        premultiplied: false,
        format: ImageFormat::Rgba8,
    }
}

impl Kind {
    /// Every kind in table order.
    pub const ALL: &[Self] = &[
        Self::SolidOpaque,
        Self::SolidTranslucent,
        Self::Linear,
        Self::Radial,
        Self::RadialIdentical,
        Self::Sweep,
        Self::ImageNearest,
        Self::ImageBilinear,
        Self::Transformed,
    ];

    /// The kind's case-table name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::SolidOpaque => "solid_opaque",
            Self::SolidTranslucent => "solid_translucent",
            Self::Linear => "linear",
            Self::Radial => "radial",
            Self::RadialIdentical => "radial_identical",
            Self::Sweep => "sweep",
            Self::ImageNearest => "image_nearest",
            Self::ImageBilinear => "image_bilinear",
            Self::Transformed => "transformed",
        }
    }

    /// The `cherenkov::Paint` this kind lowers, built through the
    /// engine's public paint API.
    pub fn paint(self) -> Paint {
        match self {
            Self::SolidOpaque => Paint::Solid(WorkingColor::new([0.2, 0.5, 0.9, 1.0])),
            Self::SolidTranslucent => Paint::Solid(WorkingColor::new([0.2, 0.5, 0.9, 0.6])),
            Self::Linear => Paint::Linear(linear_gradient()),
            Self::Radial => Paint::Radial(RadialGradient {
                start_center: (512.0, 256.0).into(),
                start_radius: 32.0,
                end_center: (640.0, 256.0).into(),
                end_radius: 400.0,
                stops: stops(),
                extend: Extend::Pad,
                interpolation: Interpolation::Working,
            }),
            Self::RadialIdentical => Paint::Radial(RadialGradient {
                start_center: (512.0, 256.0).into(),
                start_radius: 256.0,
                end_center: (512.0, 256.0).into(),
                end_radius: 256.0,
                stops: stops(),
                extend: Extend::Pad,
                interpolation: Interpolation::Working,
            }),
            Self::Sweep => {
                let mut sweep = SweepGradient::new((512.0, 256.0), 0.0, std::f64::consts::TAU);
                sweep.stops = stops();
                Paint::Sweep(sweep)
            }
            Self::ImageNearest | Self::ImageBilinear => Paint::Image(ImagePattern {
                image: IMAGE,
                // A full-width run sweeps the 64×64 image once.
                transform: Affine::scale(1024.0 / 64.0),
                extend_x: Extend::Pad,
                extend_y: Extend::Pad,
                sampling: match self {
                    Self::ImageNearest => Sampling::Nearest,
                    _ => Sampling::Linear,
                },
            }),
            Self::Transformed => Paint::Transformed(TransformedPaint::new(
                Paint::Linear(linear_gradient()),
                Affine::translate((20.0, 5.0)) * Affine::rotate(0.3),
            )),
        }
    }

    /// The registered images `paint_data` resolves this kind's paint
    /// against; empty for every non-image kind.
    pub fn images(self) -> Vec<(u64, Arc<kernels::CpuImage>)> {
        match self {
            Self::ImageNearest | Self::ImageBilinear => vec![(
                IMAGE.raw(),
                Arc::new(kernels::image(&upload()).expect("the case table's upload decodes")),
            )],
            _ => Vec::new(),
        }
    }

    /// Asserts `data` is the `PaintData` variant this kind names.
    fn assert_variant(self, data: &PaintData) {
        assert!(
            matches!(
                (self, data),
                (
                    Self::SolidOpaque | Self::SolidTranslucent,
                    PaintData::Solid(_)
                ) | (Self::Linear, PaintData::Linear { .. })
                    | (
                        Self::Radial,
                        PaintData::Radial {
                            identical: false,
                            ..
                        }
                    )
                    | (
                        Self::RadialIdentical,
                        PaintData::Radial {
                            identical: true,
                            ..
                        }
                    )
                    | (Self::Sweep, PaintData::Sweep { .. })
                    | (
                        Self::ImageNearest | Self::ImageBilinear,
                        PaintData::Image(_)
                    )
                    | (Self::Transformed, PaintData::Transformed(..))
            ),
            "kind {} lowered to {data:?}",
            self.name()
        );
    }
}

/// One case: a paint kind, a run length and a start column.
#[derive(Clone, Copy, Debug)]
pub struct Case {
    /// The paint kind.
    pub kind: Kind,
    /// The run length in pixels.
    pub len: usize,
    /// The run's first device column.
    pub offset: usize,
}

impl fmt::Display for Case {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.kind.name(), self.len, self.offset)
    }
}

/// `eval_span`'s inputs: the paint and the run's output buffer.
#[derive(Debug)]
pub struct Eval {
    /// The paint.
    pub paint: PaintData,
    /// The run's first device column.
    pub x0: usize,
    /// The run's device row.
    pub py: usize,
    /// The output buffer, `len` pixels.
    pub out: Vec<[f32; 4]>,
}

/// `composite_span`'s inputs: a destination row, the paint evaluated
/// over the run, the run's coverage and its clip source.
#[derive(Debug)]
pub struct Composite {
    /// The destination row, `len` pixels.
    pub dst: Vec<[f32; 4]>,
    /// The evaluated run, `len` pixels.
    pub src: Vec<[f32; 4]>,
    /// The run's coverage.
    pub cov: Vec<f32>,
    /// The run's clip-coverage source.
    pub clip: ClipRun<'static>,
    /// The destination's storage space.
    pub space: cherenkov::BlendSpace,
}

impl Case {
    /// The `eval_span` input of this case: the kind's `Paint` lowered
    /// through `paint_data`, asserting the variant the kind names.
    pub fn eval(self) -> Eval {
        let paint = kernels::paint(&self.kind.paint(), Affine::IDENTITY, self.kind.images())
            .expect("the case table's paints lower");
        self.kind.assert_variant(&paint);
        Eval {
            paint,
            x0: self.offset,
            py: PY,
            out: vec![[0.0; 4]; self.len],
        }
    }

    /// The `composite_span` input of this case: `src` is the kind's
    /// paint evaluated over the run, `cov` a deterministic ramp and
    /// `dst` a deterministic destination row.
    #[expect(
        clippy::cast_precision_loss,
        reason = "run lengths and offsets are far below 2^24"
    )]
    pub fn composite(self) -> Composite {
        let Eval {
            paint,
            x0,
            py,
            mut out,
        } = self.eval();
        paint.eval_span(x0, py, &mut out);
        Composite {
            dst: (0..self.len)
                .map(|i| {
                    [
                        (i % 5) as f32 / 10.0,
                        (i % 3) as f32 / 10.0,
                        (i % 7) as f32 / 10.0,
                        0.5,
                    ]
                })
                .collect(),
            src: out,
            cov: (0..self.len)
                .map(|i| 0.25 + 0.75 * ((i * 29 + self.offset) % 13) as f32 / 12.0)
                .collect(),
            clip: ClipRun::new(None, W, self.offset, PY),
            space: cherenkov::BlendSpace::Linear,
        }
    }
}

/// Every case in table order: kind, then run length, then start offset.
pub fn cases() -> impl Iterator<Item = Case> {
    Kind::ALL.iter().flat_map(|&kind| {
        LENS.iter().flat_map(move |&len| {
            OFFSETS
                .iter()
                .map(move |&offset| Case { kind, len, offset })
        })
    })
}

/// One kind's len × offset grid in the same fixed order — the table
/// filtered to `kind`.
pub fn of(kind: Kind) -> impl Iterator<Item = Case> {
    cases().filter(move |case| case.kind.name() == kind.name())
}
