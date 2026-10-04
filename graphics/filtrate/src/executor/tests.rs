use core::time::Duration;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};

use cherenkov_shader::{
    SamplerFilter,
    eval::{Eval, Value},
    naga,
};
use image::RgbaImage;

use super::{
    Executor, entry,
    plan::{PassAux, Plan},
};
use crate::{
    AnimatedCallback, AnimatedTarget, AuxSource, ColorFilter, ColorStage, CpuFilter, CpuImage,
    CpuKernel, Effect, EffectContext, EffectFrameTiming, EffectInput, EffectOutput,
    EffectRenderError, EffectSetupError, Filter, FilterExt, FilterParam, Footprint, ImageVisitor,
    Interpolator, OperatingSpace, ParamArray, ParamSource, Placed, ShaderEffect, ShapeInput,
    ShapeTextures, SpatialFilter, SpatialStage, StageCollector, WatchGuard, WorkingSpace, filters,
    kind,
};

// ============================================================================
// Test filters for the declarations no built-in uses yet
// ============================================================================

/// Scales the input by the clip shape's mask.
struct Masked;

impl Filter for Masked {
    type Kind = kind::Spatial;
    type Params = [f32; 0];

    fn params(&self) -> [f32; 0] {
        []
    }

    fn collect_stages<C: StageCollector>(&self, collector: &mut C) {
        const STAGE: SpatialStage = SpatialStage {
            name: "masked",
            source: include_str!("test_snippets/masked.wgsl"),
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

/// Declares a shape input its snippet does not take.
struct UndeclaredShape;

impl Filter for UndeclaredShape {
    type Kind = kind::Spatial;
    type Params = [f32; 0];

    fn params(&self) -> [f32; 0] {
        []
    }

    fn collect_stages<C: StageCollector>(&self, collector: &mut C) {
        const STAGE: SpatialStage = SpatialStage {
            name: "median",
            source: include_str!("../shaders/image/convolution/median3x3.wgsl"),
            params: &[],
            space: OperatingSpace::Working,
            shape: Some(ShapeInput::Sdf),
            aux: &[],
        };
        collector.spatial(Placed::new(&STAGE));
    }
}

impl SpatialFilter for UndeclaredShape {
    fn footprint_of(_params: &[f32; 0]) -> Footprint {
        Footprint::pixels(1.0)
    }
}

/// Inverts in sRGB, the way a CSS `invert()` does.
struct CssInvert;

impl Filter for CssInvert {
    type Kind = kind::Color;
    type Params = [f32; 0];

    fn params(&self) -> [f32; 0] {
        []
    }

    fn collect_stages<C: StageCollector>(&self, collector: &mut C) {
        const STAGE: ColorStage = ColorStage {
            name: "css_invert",
            source: include_str!("../shaders/color/transform/invert.wgsl"),
            params: &[],
            space: OperatingSpace::Srgb,
        };
        collector.color(Placed::new(&STAGE));
    }
}

impl ColorFilter for CssInvert {
    const LINEAR: bool = false;
}

/// One filtered sample at `uv * 0.5` — a pure bilinear read of the input.
struct SampleHalf;

const SAMPLE_HALF: SpatialStage = SpatialStage {
    name: "sample_half",
    source: include_str!("test_snippets/sample_half.wgsl"),
    params: &[],
    space: OperatingSpace::Working,
    shape: None,
    aux: &[],
};

impl Filter for SampleHalf {
    type Kind = kind::Spatial;
    type Params = [f32; 0];

    fn params(&self) -> [f32; 0] {
        []
    }

    fn collect_stages<C: StageCollector>(&self, collector: &mut C) {
        collector.spatial(Placed::new(&SAMPLE_HALF));
    }
}

impl SpatialFilter for SampleHalf {
    fn footprint_of(_params: &[f32; 0]) -> Footprint {
        Footprint::pixels(1.0)
    }
}

/// Passes the `aux0` texel at `uv` through — its precision is the output.
const READ_AUX: SpatialStage = SpatialStage {
    name: "read_aux",
    source: include_str!("test_snippets/read_aux.wgsl"),
    params: &[],
    space: OperatingSpace::Working,
    shape: None,
    aux: &[AuxSource::Image(0)],
};

/// `read_aux` with the image declared a caller-provided texture.
const READ_AUX_TEXTURE: SpatialStage = SpatialStage {
    aux: &[AuxSource::Texture(0)],
    ..READ_AUX
};

/// `apply` passes `aux0` through; the filter holds it as image 0.
struct ReadAux {
    image: crate::FilterImage,
}

impl Filter for ReadAux {
    type Kind = kind::Spatial;
    type Params = [f32; 0];
    const IMAGES: usize = 1;

    fn params(&self) -> [f32; 0] {
        []
    }

    fn collect_stages<C: StageCollector>(&self, collector: &mut C) {
        collector.spatial(Placed::new(&READ_AUX));
    }

    fn visit_images<V: ImageVisitor>(&self, visitor: &mut V) {
        visitor.visit(0, &self.image);
    }
}

impl SpatialFilter for ReadAux {
    fn footprint_of(_params: &[f32; 0]) -> Footprint {
        Footprint::ZERO
    }
}

/// `ReadAux` declaring its image a caller-provided GPU texture.
struct ReadAuxTexture {
    image: crate::FilterImage,
}

impl Filter for ReadAuxTexture {
    type Kind = kind::Spatial;
    type Params = [f32; 0];
    const IMAGES: usize = 1;

    fn params(&self) -> [f32; 0] {
        []
    }

    fn collect_stages<C: StageCollector>(&self, collector: &mut C) {
        collector.spatial(Placed::new(&READ_AUX_TEXTURE));
    }

    fn visit_images<V: ImageVisitor>(&self, visitor: &mut V) {
        visitor.visit(0, &self.image);
    }
}

impl SpatialFilter for ReadAuxTexture {
    fn footprint_of(_params: &[f32; 0]) -> Footprint {
        Footprint::ZERO
    }
}

// ============================================================================
// The reference program
// ============================================================================

/// Composes `filter` and wraps and validates every pass's entry point —
/// for the hardware-filtered plan and for the manual-bilinear one.
fn assert_composes<F: Filter>(name: &str, filter: &F) -> Plan {
    for filterable in [true, false] {
        let plan = Plan::new(filter, filterable, filterable, false).unwrap_or_else(|error| {
            panic!("{name} does not compose (filterable={filterable}): {error}")
        });
        for (index, pass) in plan.passes.iter().enumerate() {
            for resized in [false, true] {
                entry::pass_module(&plan.module, plan.capabilities, index, pass, resized)
                    .unwrap_or_else(|error| {
                        panic!("{name} (filterable={filterable}, resized={resized}): {error}")
                    });
            }
        }
    }
    Plan::new(filter, true, true, false)
        .unwrap_or_else(|error| panic!("{name} does not compose: {error}"))
}

fn blank_image() -> crate::FilterImage {
    crate::FilterImage::from_rgba8(2, 2, vec![128; 16])
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one composition check per built-in filter, in one list"
)]
fn every_builtin_composes_into_valid_passes() {
    use filters::*;

    assert_composes("brightness", &Brightness(0.2_f32));
    assert_composes("contrast", &Contrast(1.2_f32));
    assert_composes("exposure", &Exposure(0.5_f32));
    assert_composes("gamma", &Gamma(2.2_f32));
    assert_composes("highlights_shadows", &HighlightsShadows(0.4_f32, 0.3_f32));
    assert_composes("temperature_tint", &TemperatureTint(0.2_f32, -0.1_f32));
    assert_composes("white_point", &WhitePoint(1.1_f32, 1.0_f32, 0.9_f32));
    assert_composes("vignette", &Vignette(0.5_f32, 0.2_f32));
    assert_composes("color_matrix", &ColorMatrix([0.5_f32; 12]));
    assert_composes("grayscale", &Grayscale(1.0_f32));
    assert_composes("hue_rotation", &HueRotation(90.0_f32));
    assert_composes("invert", &Invert);
    assert_composes("saturation", &Saturation(1.3_f32));
    assert_composes("sepia", &Sepia(0.8_f32));
    assert_composes("vibrance", &Vibrance(0.5_f32));
    assert_composes("bump", &BumpDistortion([0.5_f32, 0.5, 0.3, 0.5]));
    assert_composes("kaleidoscope", &Kaleidoscope([6.0_f32, 0.0, 0.5, 0.5]));
    assert_composes(
        "perspective_correction",
        &PerspectiveCorrection([0.1_f32, 0.1, 0.9, 0.0, 1.0, 1.0, 0.0, 0.9]),
    );
    assert_composes(
        "perspective_transform",
        &PerspectiveTransform([0.1_f32, 0.1, 0.9, 0.0, 1.0, 1.0, 0.0, 0.9]),
    );
    assert_composes("pinch", &PinchDistortion([0.5_f32, 0.5, 0.3, 0.5]));
    assert_composes("pixellate", &Pixellate(8.0_f32));
    assert_composes("twirl", &TwirlDistortion([0.5_f32, 0.5, 0.3, 90.0]));
    assert_composes("vortex", &VortexDistortion([0.5_f32, 0.5, 0.3, 90.0]));
    assert_composes("blur", &Blur(3.0_f32));
    assert_composes("gaussian_blur", &GaussianBlur(2.0_f32));
    assert_composes("motion_blur", &MotionBlur(4.0_f32, 30.0_f32));
    assert_composes("zoom_blur", &ZoomBlur(0.2_f32, 0.5_f32, 0.5_f32));
    assert_composes("convolution3x3", &Convolution3x3([0.1_f32; 9]));
    assert_composes("convolution5x5", &Convolution5x5([0.04_f32; 25]));
    assert_composes("edge_work", &EdgeWork([2.0_f32, 1.0]));
    assert_composes("median3x3", &Median3x3);
    assert_composes("prewitt", &Prewitt);
    assert_composes("sharpen", &Sharpen(1.0_f32));
    assert_composes("sobel", &Sobel);
    assert_composes(
        "unsharp_mask",
        &UnsharpMask {
            radius: 3.0_f32,
            intensity: 1.0,
        },
    );
    assert_composes("morphology_min", &MorphologyMin);
    assert_composes("morphology_max", &MorphologyMax);
    assert_composes("morphology_gradient", &MorphologyGradient);
    assert_composes("dot_halftone", &DotHalftone([8.0_f32, 45.0, 0.5, 0.5]));
    assert_composes("line_halftone", &LineHalftone([8.0_f32, 45.0, 0.5, 0.5]));
    let glow = |radius| Bloom {
        radius,
        intensity: 1.0_f32,
        threshold: 0.5,
    };
    assert_composes("bloom", &glow(4.0));
    assert_composes(
        "gloom",
        &Gloom {
            radius: 4.0_f32,
            intensity: 1.0,
            threshold: 0.5,
        },
    );
    assert_composes("photo_effect_mono", &PhotoEffectMono);
    assert_composes("photo_effect_noir", &PhotoEffectNoir);
    assert_composes("photo_effect_chrome", &PhotoEffectChrome);
    assert_composes("photo_effect_instant", &PhotoEffectInstant);
    assert_composes("photo_effect_fade", &PhotoEffectFade);
    assert_composes("photo_effect_process", &PhotoEffectProcess);
    assert_composes("photo_effect_tonal", &PhotoEffectTonal);
    assert_composes("photo_effect_transfer", &PhotoEffectTransfer);
    assert_composes("crystallize", &Crystallize(12.0_f32));
    assert_composes("mirror_tile", &MirrorTile([2.0_f32, 2.0]));
    assert_composes(
        "blend_with_image",
        &BlendWithImage {
            image: blank_image(),
            amount: 0.5_f32,
            mode: BlendMode::Luminosity,
        },
    );
    assert_composes(
        "masked_blur",
        &MaskedBlur {
            mask: blank_image(),
            radius: 2.0_f32,
            strength: 1.0,
        },
    );
    assert_composes(
        "transition",
        &TransitionToImage {
            target: blank_image(),
            progress: 0.5_f32,
            softness: 0.1,
        },
    );
    assert_composes(
        "displacement_warp",
        &DisplacementWarp {
            map: blank_image(),
            scale_x: 4.0_f32,
            scale_y: 4.0,
        },
    );
    assert_composes(
        "guided_smooth",
        &GuidedSmooth {
            guide: blank_image(),
            radius: 2.0_f32,
            range_sigma: 0.1,
            amount: 1.0,
        },
    );
    assert_composes(
        "depth_aware_blur",
        &DepthAwareBlur {
            depth: blank_image(),
            focus_depth: 0.5_f32,
            aperture: 1.0,
            max_radius: 4.0,
        },
    );
    assert_composes(
        "temporal_denoise",
        &TemporalDenoise {
            history: blank_image(),
            motion: blank_image(),
            history_weight: 0.5_f32,
        },
    );
    assert_composes(
        "background_replace",
        &BackgroundReplace {
            matte: blank_image(),
            background: blank_image(),
            edge_softness: 0.1_f32,
        },
    );
    assert_composes(
        "lut_color_grade",
        &LutColorGrade {
            lut: crate::LutImage::from_rgba8(2, vec![255; 32]),
            intensity: 1.0_f32,
        },
    );
    assert_composes(
        "tone_curve",
        &ToneCurve {
            shadows: 0.1_f32,
            midtones: 0.0,
            highlights: -0.1,
            gamma: 1.0,
            amount: 1.0,
        },
    );
    assert_composes(
        "swipe_transition",
        &SwipeTransitionToImage {
            target: blank_image(),
            progress: 0.5_f32,
            softness: 0.1,
            direction: TransitionDirection::BottomToTop,
        },
    );
    assert_composes(
        "radial_transition",
        &RadialTransitionToImage {
            target: blank_image(),
            progress: 0.5_f32,
            softness: 0.1,
            center_x: 0.5,
            center_y: 0.5,
        },
    );
    assert_composes(
        "zoom_transition",
        &ZoomTransitionToImage {
            target: blank_image(),
            progress: 0.5_f32,
            amount: 0.5,
            center_x: 0.5,
            center_y: 0.5,
        },
    );
    assert_composes(
        "displacement_transition",
        &DisplacementTransitionToImage {
            target: blank_image(),
            map: blank_image(),
            progress: 0.5_f32,
            scale: 8.0,
        },
    );
    assert_composes(
        "chain",
        &PhotoEffectChrome
            .then(Blur(2.0_f32))
            .then(glow(3.0))
            .then(Brightness(0.1_f32)),
    );
}

#[test]
fn the_reference_program_materializes_every_spatial_stage() {
    use filters::{Blur, Brightness, Invert, Saturation};

    let plan = assert_composes(
        "chain",
        &Saturation(1.2_f32)
            .then(Blur(2.0_f32))
            .then(Brightness(0.1_f32))
            .then(Invert),
    );
    // Saturation alone, the blur's two passes, and the colour suffix fused
    // into one segment: the reference program never selects a fold.
    let stages: Vec<_> = plan
        .passes
        .iter()
        .map(|pass| (pass.segment.stages.clone(), pass.sampler))
        .collect();
    assert_eq!(
        stages,
        [
            (0..1, None),
            (1..2, Some(SamplerFilter::Point)),
            (2..3, Some(SamplerFilter::Point)),
            (3..5, None),
        ]
    );
}

#[test]
fn a_two_pass_filter_reads_its_first_pass_input() {
    use filters::{Bloom, Brightness};

    let plan = assert_composes(
        "bloom",
        &Brightness(0.1_f32).then(Bloom {
            radius: 3.0_f32,
            intensity: 1.0,
            threshold: 0.5,
        }),
    );
    // Brightness, extraction, composite: the composite's `aux0` is what the
    // extraction pass read — the brightened image, not the chain's input.
    assert_eq!(plan.passes.len(), 3);
    assert_eq!(plan.passes[2].aux, [PassAux::PassInput(1)]);
}

#[test]
fn srgb_stages_are_converted_around() {
    let plan = assert_composes("css_invert", &CssInvert);
    // The conversions are colour stages, so they share the stage's segment.
    assert_eq!(plan.passes.len(), 1);
    assert_eq!(plan.passes[0].segment.stages, 0..3);
}

#[test]
fn a_stage_declares_the_shape_input_its_snippet_takes() {
    let plan = assert_composes("masked", &Masked);
    assert_eq!(plan.passes[0].shape, Some(ShapeInput::Mask));
    assert!(matches!(
        Plan::new(&UndeclaredShape, true, true, false),
        Err(EffectSetupError::StageMismatch {
            stage: "median",
            ..
        })
    ));
}

// ============================================================================
// The colour-filter contract, checked on the CPU against the shaders
// ============================================================================

/// The only stage of a single-stage colour filter.
fn colour_stage<F: Filter>(filter: &F) -> &'static ColorStage {
    struct Only(Option<&'static ColorStage>);
    impl StageCollector for Only {
        fn color(&mut self, stage: Placed<ColorStage>) {
            assert!(self.0.replace(stage.stage).is_none(), "one stage");
        }
        fn spatial(&mut self, _: Placed<SpatialStage>) {
            panic!("a colour filter reported a spatial stage");
        }
    }
    let mut only = Only(None);
    filter.collect_stages(&mut only);
    only.0.expect("the filter reports a stage")
}

/// Runs a colour stage's snippet on one premultiplied colour.
fn evaluate(stage: &ColorStage, params: &[f32], colour: [f32; 4]) -> [f32; 4] {
    let module = naga::front::wgsl::parse_str(stage.source).expect("the built-in snippet parses");
    let eval = Eval::new(&module);
    let apply = eval.function("apply");
    let arguments = module.functions[apply]
        .arguments
        .iter()
        .map(|argument| match argument.name.as_deref() {
            Some("color") => Value::vec(&colour),
            Some("params") => Value::Struct(
                stage
                    .params
                    .iter()
                    .map(|source| match *source {
                        ParamSource::Param(index) => Value::Float(params[index]),
                        ParamSource::Constant(&[value]) => Value::Float(value),
                        ParamSource::Constant(values) => Value::vec(values),
                    })
                    .collect(),
            ),
            Some("space") => Value::Struct(vec![Value::vec(&WorkingSpace::LINEAR_DISPLAY_P3.luma)]),
            other => panic!("unexpected argument {other:?}"),
        })
        .collect();
    let result = eval.call(apply, arguments).components();
    [result[0], result[1], result[2], result[3]]
}

fn assert_close(actual: [f32; 4], expected: [f32; 4], what: &str) {
    for (got, want) in actual.iter().zip(expected) {
        assert!(
            (got - want).abs() <= 1e-5 * want.abs().max(1.0),
            "{what}: {actual:?} vs {expected:?}"
        );
    }
}

/// Premultiplied colours, some extended.
const COLOURS: [[f32; 4]; 4] = [
    [0.2, 0.4, 0.6, 1.0],
    [0.05, 0.3, 0.1, 0.5],
    [0.9, 0.2, 0.0, 0.8],
    [1.6, 0.7, -0.1, 0.9],
];

/// Checks a filter's stage is a linear map on premultiplied RGBA with an
/// identity alpha row: additive, homogeneous, and alpha-preserving.
fn assert_linear<F: ColorFilter>(name: &str, filter: &F) {
    const { assert!(F::LINEAR) };
    let stage = colour_stage(filter);
    let mut params = vec![0.0; <F::Params as ParamArray>::LEN];
    filter.params().write_to(&mut params);
    let apply = |colour| evaluate(stage, &params, colour);
    for pair in COLOURS.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let sum: [f32; 4] = core::array::from_fn(|i| a[i] + b[i]);
        let separately: [f32; 4] = core::array::from_fn(|i| apply(a)[i] + apply(b)[i]);
        assert_close(apply(sum), separately, &format!("{name} is not additive"));
        let scaled: [f32; 4] = core::array::from_fn(|i| a[i] * 0.37);
        let after: [f32; 4] = core::array::from_fn(|i| apply(a)[i] * 0.37);
        assert_close(apply(scaled), after, &format!("{name} is not homogeneous"));
        assert!((apply(a)[3] - a[3]).abs() <= 1e-6, "{name} changes alpha");
    }
}

#[test]
fn linear_colour_filters_are_linear_maps() {
    use filters::*;

    assert_linear("brightness", &Brightness(0.3_f32));
    assert_linear("contrast", &Contrast(1.4_f32));
    assert_linear("exposure", &Exposure(0.7_f32));
    assert_linear("white_point", &WhitePoint(1.1_f32, 0.9_f32, 0.8_f32));
    assert_linear(
        "color_matrix",
        &ColorMatrix([
            0.9_f32, 0.1, 0.0, 0.05, 0.1, 0.8, 0.1, -0.02, 0.0, 0.2, 0.7, 0.1,
        ]),
    );
    assert_linear("grayscale", &Grayscale(0.6_f32));
    assert_linear("hue_rotation", &HueRotation(120.0_f32));
    assert_linear("invert", &Invert);
    assert_linear("saturation", &Saturation(1.7_f32));
    assert_linear("sepia", &Sepia(0.8_f32));
    assert_linear("photo_effect_mono", &PhotoEffectMono);
    assert_linear("photo_effect_tonal", &PhotoEffectTonal);
}

/// Runs a filter's CPU kernel and its shader on the same pixels.
fn assert_kernel_matches_shader<F: CpuKernel>(name: &str, filter: &F) {
    let stage = colour_stage(filter);
    let params = filter.params();
    let mut flattened = vec![0.0; <F::Params as ParamArray>::LEN];
    params.write_to(&mut flattened);
    let mut pixels = COLOURS;
    F::apply_cpu(&params, &WorkingSpace::LINEAR_DISPLAY_P3, &mut pixels);
    for (cpu, colour) in pixels.iter().zip(COLOURS) {
        assert_close(
            *cpu,
            evaluate(stage, &flattened, colour),
            &format!("{name}'s CPU kernel disagrees with its shader"),
        );
    }
}

#[test]
fn cpu_kernels_match_their_shaders() {
    use filters::{Brightness, ColorMatrix, Grayscale, HueRotation, Saturation};

    assert_kernel_matches_shader("brightness", &Brightness(-0.2_f32));
    assert_kernel_matches_shader("saturation", &Saturation(1.6_f32));
    assert_kernel_matches_shader("grayscale", &Grayscale(0.7_f32));
    assert_kernel_matches_shader("hue_rotation", &HueRotation(120.0_f32));
    assert_kernel_matches_shader(
        "color_matrix",
        &ColorMatrix([
            0.9_f32, 0.1, 0.0, 0.05, 0.1, 0.8, 0.1, -0.02, 0.0, 0.2, 0.7, 0.1,
        ]),
    );
    // A chain's kernel runs its halves in order.
    let chain = Saturation(0.5_f32).then(Brightness(0.1_f32));
    let mut chained = COLOURS;
    <filters::Saturation<f32> as CpuKernel>::apply_cpu(
        &[0.5],
        &WorkingSpace::LINEAR_DISPLAY_P3,
        &mut chained,
    );
    <filters::Brightness<f32> as CpuKernel>::apply_cpu(
        &[0.1],
        &WorkingSpace::LINEAR_DISPLAY_P3,
        &mut chained,
    );
    let mut pixels = COLOURS;
    chain.apply_cpu_now(&WorkingSpace::LINEAR_DISPLAY_P3, &mut pixels);
    assert_eq!(pixels, chained);
}

#[test]
fn hue_rotation_matches_the_css_reference_matrices() {
    use filters::HueRotation;

    // The `feColorMatrix type="hueRotate"` coefficients at the spec's
    // canonical angles, rows of the (r', g', b') output in input (r, g, b)
    // order: the identity at 0°, at 90°, and its square at 180°.
    let matrices: [(f32, [[f32; 3]; 3]); 3] = [
        (0.0_f32, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
        (
            90.0,
            [
                [0.000, 0.000, 1.000],
                [0.356, 0.855, -0.211],
                [-0.574, 1.430, 0.144],
            ],
        ),
        (
            180.0,
            [
                [-0.574, 1.430, 0.144],
                [0.426, 0.430, 0.144],
                [0.426, 1.430, -0.856],
            ],
        ),
    ];
    let stage = colour_stage(&HueRotation(0.0_f32));
    for (angle, matrix) in matrices {
        for colour in COLOURS {
            let expected: [f32; 3] = core::array::from_fn(|channel| {
                matrix[channel][0].mul_add(
                    colour[0],
                    matrix[channel][1].mul_add(colour[1], matrix[channel][2] * colour[2]),
                )
            });
            assert_close(
                evaluate(stage, &[angle], colour),
                [expected[0], expected[1], expected[2], colour[3]],
                &format!("hue_rotation({angle}) differs from the CSS matrix"),
            );
        }
    }
}

#[test]
fn hue_rotation_commutes_with_source_over() {
    use filters::HueRotation;

    // A linear map on premultiplied colour commutes with src-over:
    // filtering the composite equals compositing the filtered layers.
    let filter = HueRotation(150.0_f32);
    let stage = colour_stage(&filter);
    let mut params = [0.0];
    filter.params().write_to(&mut params);
    let apply = |colour| evaluate(stage, &params, colour);
    let src_over = |src: [f32; 4], dst: [f32; 4]| -> [f32; 4] {
        core::array::from_fn(|channel| (1.0 - src[3]).mul_add(dst[channel], src[channel]))
    };
    for pair in COLOURS.windows(2) {
        let (src, dst) = (pair[0], pair[1]);
        assert_close(
            apply(src_over(src, dst)),
            src_over(apply(src), apply(dst)),
            "hue_rotation does not commute with src-over",
        );
    }
}

// ============================================================================
// Parameters and animation
// ============================================================================

/// A `FilterParam` whose watcher callback the test fires by hand.
struct ScriptedParam {
    initial: f32,
    callback: Arc<Mutex<Option<AnimatedCallback>>>,
}

impl ScriptedParam {
    fn constant(value: f32) -> Self {
        Self {
            initial: value,
            callback: Arc::default(),
        }
    }

    /// Fires the installed watcher with a new target.
    fn fire(callback: &Mutex<Option<AnimatedCallback>>, target: AnimatedTarget) {
        callback
            .lock()
            .expect("scripted param callback mutex poisoned")
            .as_ref()
            .expect("the executor installs a watcher on every parameter")(target);
    }
}

impl FilterParam for ScriptedParam {
    fn snapshot(&self) -> f32 {
        self.initial
    }

    fn watch_animated(&self, callback: AnimatedCallback) -> WatchGuard {
        *self
            .callback
            .lock()
            .expect("scripted param callback mutex poisoned") = Some(callback);
        WatchGuard::new(())
    }
}

/// Linear ramp over a fixed duration, for deterministic mid-flight sampling.
struct LinearRamp(Duration);

impl Interpolator for LinearRamp {
    fn duration(&self) -> Duration {
        self.0
    }

    fn interpolate(&self, from: f32, to: f32, elapsed: Duration) -> f32 {
        let progress = (elapsed.as_secs_f32() / self.0.as_secs_f32()).min(1.0);
        (to - from).mul_add(progress, from)
    }
}

#[test]
fn the_footprint_covers_every_running_animation() {
    let size = (256.0, 128.0);
    let radius = ScriptedParam::constant(2.0);
    let callback = radius.callback.clone();
    let mut executor = Executor::new(filters::Blur(radius));
    assert_eq!(executor.footprint(size), 2.0);

    // Growing: the bound is the target before a single frame ran.
    ScriptedParam::fire(
        &callback,
        AnimatedTarget {
            value: 10.0,
            interpolator: Some(Box::new(LinearRamp(Duration::from_millis(100)))),
        },
    );
    assert_eq!(executor.footprint(size), 10.0);
    executor.animator.update(Duration::from_millis(200));

    // Shrinking: the bound stays at the start until the animation ends.
    ScriptedParam::fire(
        &callback,
        AnimatedTarget {
            value: 1.0,
            interpolator: Some(Box::new(LinearRamp(Duration::from_millis(100)))),
        },
    );
    assert_eq!(executor.footprint(size), 10.0);
    executor.animator.update(Duration::from_millis(50));
    assert_eq!(executor.footprint(size), 10.0);
    executor.animator.update(Duration::from_millis(60));
    assert_eq!(executor.footprint(size), 1.0);

    // A relative footprint resolves against the input's larger dimension,
    // plus its absolute pixel component.
    let mut zoom = Executor::new(filters::ZoomBlur(0.2_f32, 0.5, 0.5));
    assert_eq!(
        zoom.footprint(size),
        (0.2_f32 * 1.5_f32.hypot(1.5)).mul_add(256.0, 1.0)
    );
}

#[test]
fn zoom_blur_footprint_covers_the_shader_reach() {
    // The shader samples at `uv + (center − uv)·amount·t` for `t` in [0, 1],
    // so the reach is `amount·|center − uv|`, worst at the image corner
    // farthest from `center` — on-centre, off-centre and out of range.
    // `footprint_of` sees the centre only as a magnitude `m`, standing for
    // any centre in `[-m, m]²`: the farthest corner is `m + 1` away per axis.
    let size = (256.0, 128.0);
    for (amount, center) in [(0.2_f32, [0.5, 0.5]), (0.2, [1.0, 1.0]), (0.2, [2.0, -1.0])] {
        let mut zoom = Executor::new(filters::ZoomBlur(amount, center[0], center[1]));
        let bound = (center[0].abs() + 1.0).hypot(center[1].abs() + 1.0);
        assert_eq!(
            zoom.footprint(size),
            (amount * bound).mul_add(256.0, 1.0),
            "footprint for amount {amount}, center {center:?}"
        );
        // … and the bound does cover the shader's true reach.
        let true_reach = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]]
            .into_iter()
            .map(|corner| (center[0] - corner[0]).hypot(center[1] - corner[1]))
            .fold(0.0_f32, f32::max);
        assert!(
            zoom.footprint(size) >= amount * true_reach * size.0,
            "bound below the reach for center {center:?}"
        );
    }
}

#[test]
fn a_divergent_distortion_scale_spans_the_extent() {
    // `scale` arrives at `footprint_of` as a magnitude, so a bound past 1
    // stands for a shader scale below −1, where the displacement diverges
    // and any texel can be reached — whichever sign the bound came from.
    let size = (256.0, 128.0);
    for scale in [2.0_f32, -2.0] {
        let mut pinch = Executor::new(filters::PinchDistortion([0.5, 0.5, 0.3, scale]));
        let mut bump = Executor::new(filters::BumpDistortion([0.5, 0.5, 0.3, scale]));
        assert_eq!(
            pinch.footprint(size),
            256.0_f32.mul_add(1.0, 1.0),
            "pinch scale {scale}"
        );
        assert_eq!(
            bump.footprint(size),
            256.0_f32.mul_add(1.0, 1.0),
            "bump scale {scale}"
        );
    }
    // While the magnitude stays within 1, the reach stays two radii.
    let mut pinch = Executor::new(filters::PinchDistortion([0.5, 0.5, 0.3, -0.5]));
    assert_eq!(pinch.footprint(size), (2.0_f32 * 0.3).mul_add(256.0, 1.0));
}

// ============================================================================
// GPU
// ============================================================================

struct TestGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn create_test_device() -> TestGpu {
    let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    instance_descriptor.backends = wgpu::Backends::all();
    let instance = wgpu::Instance::new(instance_descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .expect("filter GPU tests require a high-performance adapter");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("filter GPU tests require a working device");
    TestGpu { device, queue }
}

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn texture(gpu: &TestGpu, size: (u32, u32), usage: wgpu::TextureUsages) -> wgpu::Texture {
    texture_format(gpu, size, usage, FORMAT)
}

/// A texture in `format`.
fn texture_format(
    gpu: &TestGpu,
    (width, height): (u32, u32),
    usage: wgpu::TextureUsages,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("filtrate test texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// An RGBA8 texture holding `rgba`.
fn upload(gpu: &TestGpu, size: (u32, u32), rgba: &[u8]) -> wgpu::Texture {
    upload_bytes(gpu, size, FORMAT, 4, rgba)
}

/// A `format` texture holding `data`, `bytes_per_texel` per texel.
fn upload_bytes(
    gpu: &TestGpu,
    size: (u32, u32),
    format: wgpu::TextureFormat,
    bytes_per_texel: u32,
    data: &[u8],
) -> wgpu::Texture {
    let texture = texture_format(
        gpu,
        size,
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        format,
    );
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size.0 * bytes_per_texel),
            rows_per_image: Some(size.1),
        },
        wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
    );
    texture
}

/// An `Rgba32Float` texture holding `texels`.
fn upload_f32(gpu: &TestGpu, size: (u32, u32), texels: &[[f32; 4]]) -> wgpu::Texture {
    upload_bytes(
        gpu,
        size,
        wgpu::TextureFormat::Rgba32Float,
        16,
        bytemuck::cast_slice(texels),
    )
}

fn readback_rgba8_image(gpu: &TestGpu, texture: &wgpu::Texture, size: (u32, u32)) -> Vec<u8> {
    readback_bytes(gpu, texture, size, 4)
}

fn readback_bytes(
    gpu: &TestGpu,
    texture: &wgpu::Texture,
    (width, height): (u32, u32),
    bytes_per_pixel: u32,
) -> Vec<u8> {
    let unpadded_bpr = width * bytes_per_pixel;
    let padded_bpr = unpadded_bpr.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("filtrate test readback"),
        size: u64::from(padded_bpr) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("filtrate test readback"),
        });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bpr),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv()
        .expect("map callback should return a completion result")
        .expect("buffer mapping should succeed");

    let mapped = slice
        .get_mapped_range()
        .expect("buffer range is mapped and not overlapping");
    let mut out = Vec::with_capacity((unpadded_bpr * height) as usize);
    for row in 0..height as usize {
        let start = row * padded_bpr as usize;
        out.extend_from_slice(&mapped[start..start + unpadded_bpr as usize]);
    }
    drop(mapped);
    buffer.unmap();
    out
}

fn readback_f32_image(gpu: &TestGpu, texture: &wgpu::Texture, size: (u32, u32)) -> Vec<[f32; 4]> {
    bytemuck::cast_slice(&readback_bytes(gpu, texture, size, 16)).to_vec()
}

fn frame_input<'a>(
    gpu: &'a TestGpu,
    texture: &'a wgpu::Texture,
    size: (u32, u32),
    delta: Duration,
    shape: ShapeTextures,
) -> EffectInput<'a> {
    frame_input_format(gpu, texture, size, delta, shape, FORMAT)
}

fn frame_input_format<'a>(
    gpu: &'a TestGpu,
    texture: &'a wgpu::Texture,
    size: (u32, u32),
    delta: Duration,
    shape: ShapeTextures,
    format: wgpu::TextureFormat,
) -> EffectInput<'a> {
    EffectInput {
        device: &gpu.device,
        queue: &gpu.queue,
        texture,
        view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        format,
        width: size.0,
        height: size.1,
        timing: EffectFrameTiming::new(Duration::ZERO, delta, 0),
        shape,
    }
}

fn frame_output<'a>(
    gpu: &'a TestGpu,
    texture: &'a wgpu::Texture,
    size: (u32, u32),
) -> EffectOutput<'a> {
    frame_output_format(gpu, texture, size, FORMAT)
}

fn frame_output_format<'a>(
    gpu: &'a TestGpu,
    texture: &'a wgpu::Texture,
    size: (u32, u32),
    format: wgpu::TextureFormat,
) -> EffectOutput<'a> {
    EffectOutput {
        device: &gpu.device,
        queue: &gpu.queue,
        texture,
        view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        format,
        width: size.0,
        height: size.1,
    }
}

fn setup<E: Effect>(gpu: &TestGpu, executor: &mut E) {
    setup_format(gpu, executor, FORMAT, FORMAT);
}

fn setup_format<E: Effect>(
    gpu: &TestGpu,
    executor: &mut E,
    input_format: wgpu::TextureFormat,
    output_format: wgpu::TextureFormat,
) {
    let ctx = EffectContext {
        device: &gpu.device,
        queue: &gpu.queue,
        input_format,
        output_format,
    };
    pollster::block_on(executor.setup(&ctx)).expect("test filter setup should succeed");
}

/// `setup_format` forcing every format unfilterable — the manual-bilinear
/// plan on a device that could filter.
fn setup_unfilterable<F: Filter, S: Fn(u32, u32) -> (u32, u32) + 'static>(
    gpu: &TestGpu,
    executor: &mut Executor<F, S>,
    input_format: wgpu::TextureFormat,
    output_format: wgpu::TextureFormat,
) {
    let ctx = EffectContext {
        device: &gpu.device,
        queue: &gpu.queue,
        input_format,
        output_format,
    };
    pollster::block_on(executor.setup_unfilterable(&ctx))
        .expect("test filter setup should succeed");
}

/// `setup` on the composer's folded program, for comparing it against the
/// plain program the executor normally runs.
fn setup_folded<F: Filter>(gpu: &TestGpu, executor: &mut Executor<F>) {
    let ctx = EffectContext {
        device: &gpu.device,
        queue: &gpu.queue,
        input_format: FORMAT,
        output_format: FORMAT,
    };
    pollster::block_on(executor.setup_folded(&ctx)).expect("test filter setup should succeed");
}

/// Renders `rgba` through `executor` once and reads the RGBA8 output back.
fn render_rgba8<F: Filter>(
    gpu: &TestGpu,
    executor: &mut Executor<F>,
    size: (u32, u32),
    rgba: &[u8],
    shape: ShapeTextures,
) -> Vec<u8> {
    let input = upload(gpu, size, rgba);
    let output = texture(
        gpu,
        size,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    executor
        .render(
            &frame_input(gpu, &input, size, Duration::ZERO, shape),
            &frame_output(gpu, &output, size),
        )
        .expect("test render should succeed");
    readback_rgba8_image(gpu, &output, size)
}

/// Runs `filter` once on `rgba` and reads the result back.
fn run<F: Filter>(
    gpu: &TestGpu,
    filter: F,
    size: (u32, u32),
    rgba: &[u8],
    shape: ShapeTextures,
) -> Vec<u8> {
    let mut executor = Executor::new(filter);
    setup(gpu, &mut executor);
    render_rgba8(gpu, &mut executor, size, rgba, shape)
}

fn to_unorm(value: f32) -> u8 {
    let scaled = (value.clamp(0.0, 1.0) * 255.0).round();
    // The clamp bounds the value to [0, 255].
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is clamped to the u8 range"
    )]
    let byte = scaled as u8;
    byte
}

fn from_unorm(byte: u8) -> f32 {
    f32::from(byte) / 255.0
}

/// Asserts two RGBA8 images agree within `tolerance` steps per channel.
fn assert_rgba8_close(actual: &[u8], expected: &[u8], tolerance: u8, what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}: sizes differ");
    for (index, (got, want)) in actual.iter().zip(expected).enumerate() {
        assert!(
            got.abs_diff(*want) <= tolerance,
            "{what}: channel {index} is {got}, expected {want}"
        );
    }
}

/// Opaque colours spread over the unit cube.
fn test_pixels(count: u32) -> Vec<u8> {
    (0..count)
        .flat_map(|index| {
            let step = |shift: u32| u8::try_from(((index * 37) >> shift) % 256).expect("fits u8");
            [step(0), step(1), step(2), 255]
        })
        .collect()
}

#[test]
fn gpu_color_filter_executes_and_writes_output() {
    let gpu = create_test_device();
    // Opaque black: brightness lifts it by the amount.
    let black: Vec<u8> = core::iter::repeat_n([0u8, 0, 0, 255], 64)
        .flatten()
        .collect();
    let output = run(
        &gpu,
        filters::Brightness(0.25_f32),
        (8, 8),
        &black,
        ShapeTextures::default(),
    );
    assert_rgba8_close(&output[..4], &[64, 64, 64, 255], 1, "brightness");
}

#[test]
fn gpu_chains_match_their_cpu_kernels() {
    let gpu = create_test_device();
    let size = (16, 4);
    let rgba = test_pixels(size.0 * size.1);
    let chain = filters::Saturation(1.4_f32)
        .then(filters::Grayscale(0.3_f32))
        .then(filters::HueRotation(60.0_f32))
        .then(filters::Brightness(-0.1_f32));
    let mut pixels: Vec<[f32; 4]> = rgba
        .chunks(4)
        .map(|texel| core::array::from_fn(|i| from_unorm(texel[i])))
        .collect();
    chain.apply_cpu_now(&WorkingSpace::LINEAR_DISPLAY_P3, &mut pixels);
    let expected: Vec<u8> = pixels
        .iter()
        .flatten()
        .map(|&value| to_unorm(value))
        .collect();
    let output = run(&gpu, chain, size, &rgba, ShapeTextures::default());
    assert_rgba8_close(
        &output,
        &expected,
        1,
        "saturation, grayscale, hue rotation, brightness",
    );
}

#[test]
fn gpu_shape_mask_reaches_the_stage() {
    let gpu = create_test_device();
    let size = (4, 1);
    let white = [255u8; 16];
    let mask = upload(
        &gpu,
        size,
        &[0, 0, 0, 255, 64, 0, 0, 255, 128, 0, 0, 255, 255, 0, 0, 255],
    );
    let shape = ShapeTextures {
        sdf: None,
        mask: Some(mask.create_view(&wgpu::TextureViewDescriptor::default())),
    };
    let output = run(&gpu, Masked, size, &white, shape);
    assert_rgba8_close(
        &output,
        &[
            0, 0, 0, 0, 64, 64, 64, 64, 128, 128, 128, 128, 255, 255, 255, 255,
        ],
        1,
        "masked",
    );

    // Without the mask the frame fails instead of reading nothing.
    let mut executor = Executor::new(Masked);
    setup(&gpu, &mut executor);
    let input = upload(&gpu, size, &white);
    let target = texture(&gpu, size, wgpu::TextureUsages::RENDER_ATTACHMENT);
    assert_eq!(
        executor.render(
            &frame_input(&gpu, &input, size, Duration::ZERO, ShapeTextures::default()),
            &frame_output(&gpu, &target, size),
        ),
        Err(EffectRenderError::MissingShape(ShapeInput::Mask))
    );
}

/// The sRGB transfer, mirrored through zero.
fn srgb_encode(linear: f32) -> f32 {
    let magnitude = linear.abs();
    let curve = if magnitude > 0.003_130_8 {
        1.055f32.mul_add(magnitude.powf(1.0 / 2.4), -0.055)
    } else {
        magnitude * 12.92
    };
    curve.copysign(linear)
}

fn srgb_decode(encoded: f32) -> f32 {
    let magnitude = encoded.abs();
    let curve = if magnitude > 0.040_45 {
        ((magnitude + 0.055) / 1.055).powf(2.4)
    } else {
        magnitude / 12.92
    };
    curve.copysign(encoded)
}

fn transform(matrix: [[f32; 3]; 3], rgb: [f32; 3]) -> [f32; 3] {
    core::array::from_fn(|row| {
        matrix[row][0].mul_add(
            rgb[0],
            matrix[row][1].mul_add(rgb[1], matrix[row][2] * rgb[2]),
        )
    })
}

#[test]
fn gpu_srgb_stages_run_in_srgb() {
    const P3_TO_SRGB: [[f32; 3]; 3] = [
        [1.224_94, -0.224_94, 0.0],
        [-0.042_057, 1.042_057, 0.0],
        [-0.019_637_6, -0.078_636, 1.098_274],
    ];
    const SRGB_TO_P3: [[f32; 3]; 3] = [
        [0.822_462, 0.177_538, 0.0],
        [0.033_194_2, 0.966_805_8, 0.0],
        [0.017_082_6, 0.072_397_4, 0.910_519_9],
    ];
    let gpu = create_test_device();
    let size = (16, 4);
    let rgba = test_pixels(size.0 * size.1);
    // Opaque: invert in sRGB, then back to the working space.
    let expected: Vec<u8> = rgba
        .chunks(4)
        .flat_map(|texel| {
            let linear = core::array::from_fn(|i| from_unorm(texel[i]));
            let encoded = transform(P3_TO_SRGB, linear).map(srgb_encode);
            let inverted = encoded.map(|value| 1.0 - value);
            let [r, g, b] = transform(SRGB_TO_P3, inverted.map(srgb_decode)).map(to_unorm);
            [r, g, b, 255]
        })
        .collect();
    let output = run(&gpu, CssInvert, size, &rgba, ShapeTextures::default());
    assert_rgba8_close(&output, &expected, 2, "invert in sRGB");
}

/// The executor's manual bilinear, as a CPU reference over f32 texels.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "test coordinates fit every type they are cast to"
)]
fn manual_bilinear(texels: &[[f32; 4]], (width, height): (u32, u32), uv: [f32; 2]) -> [f32; 4] {
    let position = [
        uv[0].mul_add(width as f32, -0.5),
        uv[1].mul_add(height as f32, -0.5),
    ];
    let base = [position[0].floor(), position[1].floor()];
    let t = [position[0] - base[0], position[1] - base[1]];
    let corner = |offset: [i32; 2]| {
        let x = (base[0] as i32 + offset[0]).clamp(0, width as i32 - 1);
        let y = (base[1] as i32 + offset[1]).clamp(0, height as i32 - 1);
        texels[(y * width as i32 + x) as usize]
    };
    let mix = |a: [f32; 4], b: [f32; 4], t: f32| {
        core::array::from_fn(|i| a[i].mul_add(1.0 - t, b[i] * t))
    };
    mix(
        mix(corner([0, 0]), corner([1, 0]), t[0]),
        mix(corner([0, 1]), corner([1, 1]), t[0]),
        t[1],
    )
}

/// `uv * 0.5` for output texel `(x, y)` of `size`, the `uv` a stage sees.
#[expect(clippy::cast_precision_loss, reason = "test coordinates fit in f32")]
fn stage_uv((width, height): (u32, u32), x: u32, y: u32) -> [f32; 2] {
    [
        (x as f32 + 0.5) / width as f32,
        (y as f32 + 0.5) / height as f32,
    ]
}

/// Texels that give every bilinear tap a distinct value, beyond unorm range.
fn float_test_texels(count: u8) -> Vec<[f32; 4]> {
    (0..count)
        .map(|i| {
            let f = f32::from(i) / f32::from(count - 1);
            [f.mul_add(4.0, -1.0), 1.0 / (f + 1.0), f * f, 0.5 + f]
        })
        .collect()
}

#[test]
fn gpu_manual_bilinear_matches_hardware_filtering() {
    let gpu = create_test_device();
    let size = (16, 16);
    let rgba = create_test_input_rgba(size.0, size.1);
    let params = [0.5_f32, 0.5, 0.4, 180.0];
    let hardware = run(
        &gpu,
        filters::TwirlDistortion(params),
        size,
        &rgba,
        ShapeTextures::default(),
    );
    let mut executor = Executor::new(filters::TwirlDistortion(params));
    setup_unfilterable(&gpu, &mut executor, FORMAT, FORMAT);
    let manual = render_rgba8(&gpu, &mut executor, size, &rgba, ShapeTextures::default());
    assert_rgba8_close(
        &manual,
        &hardware,
        1,
        "manual bilinear vs hardware filtering",
    );
}

#[test]
fn gpu_folded_chain_matches_the_unfolded_chain() {
    let gpu = create_test_device();
    let size = (16, 16);
    let rgba = create_test_input_rgba(size.0, size.1);
    // A colour prefix followed by a spatial stage: the composer folds the
    // prefix into the stage's samples — here through the `load` helper box
    // blur samples with — so the folded program is one pass where the plain
    // one is two.
    let unfolded = run(
        &gpu,
        filters::Brightness(0.1_f32).then(filters::Blur(1.0_f32)),
        size,
        &rgba,
        ShapeTextures::default(),
    );
    let mut executor = Executor::new(filters::Brightness(0.1_f32).then(filters::Blur(1.0_f32)));
    setup_folded(&gpu, &mut executor);
    // Box blur is separable (two spatial stages): the colour prefix folds
    // into the first; the second stays a plain segment.
    assert_eq!(
        executor
            .gpu
            .as_ref()
            .expect("setup succeeded")
            .segment_stages(),
        [0..2, 2..3],
        "the colour prefix folded into the blur's samples"
    );
    let folded = render_rgba8(&gpu, &mut executor, size, &rgba, ShapeTextures::default());
    assert_rgba8_close(&folded, &unfolded, 1, "folded chain vs unfolded chain");
}

#[test]
fn gpu_folded_segment_reads_the_size_uniform() {
    let gpu = create_test_device();
    let size = (16, 16);
    let rgba = create_test_input_rgba(size.0, size.1);
    // Vignette is size-dependent: folding brightness into it proves the
    // `size` uniform flows through the folded segment, not just the plain.
    let unfolded = run(
        &gpu,
        filters::Brightness(0.1_f32).then(filters::Vignette(0.8_f32, 0.3_f32)),
        size,
        &rgba,
        ShapeTextures::default(),
    );
    let mut executor =
        Executor::new(filters::Brightness(0.1_f32).then(filters::Vignette(0.8_f32, 0.3_f32)));
    setup_folded(&gpu, &mut executor);
    let folded = executor
        .gpu
        .as_ref()
        .expect("setup succeeded")
        .segment_stages();
    assert_eq!(folded.len(), 1);
    assert_eq!(
        folded[0],
        0..2,
        "the colour prefix folded into the vignette's samples"
    );
    let folded = render_rgba8(&gpu, &mut executor, size, &rgba, ShapeTextures::default());
    assert_rgba8_close(
        &folded,
        &unfolded,
        1,
        "folded size-dependent chain vs unfolded chain",
    );
}

#[test]
fn gpu_filtered_samples_work_on_rgba32f_input() {
    let gpu = create_test_device();
    let size = (8, 8);
    let texels = float_test_texels(64);
    let mut executor = Executor::new(SampleHalf);
    setup_unfilterable(
        &gpu,
        &mut executor,
        wgpu::TextureFormat::Rgba32Float,
        wgpu::TextureFormat::Rgba32Float,
    );
    let input = upload_f32(&gpu, size, &texels);
    let output = texture_format(
        &gpu,
        size,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        wgpu::TextureFormat::Rgba32Float,
    );
    executor
        .render(
            &frame_input_format(
                &gpu,
                &input,
                size,
                Duration::ZERO,
                ShapeTextures::default(),
                wgpu::TextureFormat::Rgba32Float,
            ),
            &frame_output_format(&gpu, &output, size, wgpu::TextureFormat::Rgba32Float),
        )
        .expect("test render should succeed");
    let readback = readback_f32_image(&gpu, &output, size);
    for y in 0..size.1 {
        for x in 0..size.0 {
            let uv = stage_uv(size, x, y);
            let expected = manual_bilinear(&texels, size, [uv[0] * 0.5, uv[1] * 0.5]);
            let got = readback[usize::try_from(y * size.0 + x).expect("fits usize")];
            for channel in 0..4 {
                assert!(
                    (got[channel] - expected[channel]).abs() < 0.0001,
                    "pixel ({x}, {y}) channel {channel}: got {}, expected {}",
                    got[channel],
                    expected[channel]
                );
            }
        }
    }
}

#[test]
fn gpu_float_aux_upload_keeps_its_precision() {
    let gpu = create_test_device();
    let size = (8, 4);
    let aux_texels = float_test_texels(32);
    let filter = ReadAux {
        image: crate::FilterImage::from_rgba32f(size.0, size.1, bytemuck::cast_slice(&aux_texels)),
    };
    let mut executor = Executor::new(filter);
    setup_format(
        &gpu,
        &mut executor,
        FORMAT,
        wgpu::TextureFormat::Rgba32Float,
    );
    let input = upload(&gpu, size, &[0; 128]);
    let output = texture_format(
        &gpu,
        size,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        wgpu::TextureFormat::Rgba32Float,
    );
    executor
        .render(
            &frame_input(&gpu, &input, size, Duration::ZERO, ShapeTextures::default()),
            &frame_output_format(&gpu, &output, size, wgpu::TextureFormat::Rgba32Float),
        )
        .expect("test render should succeed");
    assert_eq!(
        readback_f32_image(&gpu, &output, size),
        aux_texels,
        "a textureLoad pass-through returns the uploaded f32 texels exactly"
    );
}

#[test]
fn gpu_texture_aux_binds_at_native_format() {
    let gpu = create_test_device();
    let size = (8, 4);
    let aux_texels = float_test_texels(32);
    let filter = ReadAuxTexture {
        image: crate::FilterImage::from_texture(crate::TextureImage::new(upload_f32(
            &gpu,
            size,
            &aux_texels,
        ))),
    };
    let mut executor = Executor::new(filter);
    setup_format(
        &gpu,
        &mut executor,
        FORMAT,
        wgpu::TextureFormat::Rgba32Float,
    );
    let input = upload(&gpu, size, &[0; 128]);
    let output = texture_format(
        &gpu,
        size,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        wgpu::TextureFormat::Rgba32Float,
    );
    executor
        .render(
            &frame_input(&gpu, &input, size, Duration::ZERO, ShapeTextures::default()),
            &frame_output_format(&gpu, &output, size, wgpu::TextureFormat::Rgba32Float),
        )
        .expect("test render should succeed");
    assert_eq!(
        readback_f32_image(&gpu, &output, size),
        aux_texels,
        "a bound texture reads back at its native precision"
    );
}

/// `EffectContext` for an RGBA8 input and output.
fn setup_error<F: Filter>(gpu: &TestGpu, executor: &mut Executor<F>) -> EffectSetupError {
    let ctx = EffectContext {
        device: &gpu.device,
        queue: &gpu.queue,
        input_format: FORMAT,
        output_format: FORMAT,
    };
    pollster::block_on(executor.setup(&ctx)).expect_err("setup should fail")
}

#[test]
fn gpu_texture_aux_rejects_cpu_images() {
    let gpu = create_test_device();
    let mut executor = Executor::new(ReadAuxTexture {
        image: crate::FilterImage::from_rgba8(1, 1, vec![0; 4]),
    });
    assert!(
        matches!(
            setup_error(&gpu, &mut executor),
            EffectSetupError::StageMismatch {
                stage: "read_aux",
                ..
            }
        ),
        "a CPU image must not satisfy a `Texture` aux"
    );
}

#[test]
fn gpu_bound_aux_rejects_non_float_formats() {
    let gpu = create_test_device();
    // An `Image` aux may bind a texture too — the format check applies either
    // way.
    let uint = texture_format(
        &gpu,
        (1, 1),
        wgpu::TextureUsages::TEXTURE_BINDING,
        wgpu::TextureFormat::R8Uint,
    );
    let mut executor = Executor::new(ReadAux {
        image: crate::FilterImage::from_texture(crate::TextureImage::new(uint)),
    });
    assert!(
        matches!(
            setup_error(&gpu, &mut executor),
            EffectSetupError::StageMismatch {
                stage: "read_aux",
                ..
            }
        ),
        "a uint texture must not bind as `texture_2d<f32>`"
    );
}

#[test]
fn gpu_mismatched_sizes_fail_the_frame() {
    let gpu = create_test_device();
    let mut executor = Executor::new(filters::Blur(1.0_f32));
    setup(&gpu, &mut executor);
    let input = upload(&gpu, (6, 4), &[255; 96]);
    let target = texture(&gpu, (11, 7), wgpu::TextureUsages::RENDER_ATTACHMENT);
    assert_eq!(
        executor.render(
            &frame_input(
                &gpu,
                &input,
                (6, 4),
                Duration::ZERO,
                ShapeTextures::default()
            ),
            &frame_output(&gpu, &target, (11, 7)),
        ),
        Err(EffectRenderError::SizeMismatch {
            input: (6, 4),
            expected: (6, 4),
            output: (11, 7),
        })
    );
}

#[test]
fn output_size_defaults_and_chain_policy() {
    let executor = Executor::new(filters::Brightness(0.0_f32));
    assert_eq!(executor.output_size(6, 4), (6, 4));
    let executor = executor
        .with_output_size(|width, height| (width * 2, height * 3))
        .then(filters::Contrast(1.0_f32));
    assert_eq!(executor.output_size(6, 4), (12, 12));
}

#[test]
#[should_panic(expected = "effect declared a zero output dimension")]
fn output_size_rejects_zero_dimensions() {
    Executor::new(filters::Brightness(0.0_f32))
        .with_output_size(|_, height| (0, height))
        .output_size(6, 4);
}

#[test]
fn gpu_declared_output_size_maps_the_whole_input() {
    let gpu = create_test_device();
    let mut executor = Executor::new(filters::Brightness(0.0_f32))
        .with_output_size(|width, height| (width * 2, height * 3));
    setup(&gpu, &mut executor);
    let pixels = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let input = upload(&gpu, (2, 2), &pixels);
    let size = executor.output_size(2, 2);
    let target = texture(
        &gpu,
        size,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    executor
        .render(
            &frame_input(
                &gpu,
                &input,
                (2, 2),
                Duration::ZERO,
                ShapeTextures::default(),
            ),
            &frame_output(&gpu, &target, size),
        )
        .expect("declared output size renders");
    let actual = readback_rgba8_image(&gpu, &target, size);
    let mut expected = Vec::new();
    for y in 0..6 {
        for x in 0..4 {
            let start = ((y / 3) * 2 + x / 2) * 4;
            expected.extend_from_slice(&pixels[start..start + 4]);
        }
    }
    assert_eq!(
        actual, expected,
        "every output pixel maps into the captured input"
    );
    let wrong = texture(&gpu, (2, 2), wgpu::TextureUsages::RENDER_ATTACHMENT);
    assert_eq!(
        executor.render(
            &frame_input(
                &gpu,
                &input,
                (2, 2),
                Duration::ZERO,
                ShapeTextures::default()
            ),
            &frame_output(&gpu, &wrong, (2, 2)),
        ),
        Err(EffectRenderError::SizeMismatch {
            input: (2, 2),
            expected: size,
            output: (2, 2),
        })
    );
}

#[test]
fn gpu_declared_output_size_preserves_spatial_sampling() {
    let gpu = create_test_device();
    let input = upload(
        &gpu,
        (4, 1),
        &[
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ],
    );
    for filterable in [true, false] {
        let mut executor = Executor::new(filters::Brightness(0.0_f32).then(SampleHalf))
            .with_output_size(|width, height| (width / 2, height));
        if filterable {
            setup(&gpu, &mut executor);
        } else {
            setup_unfilterable(&gpu, &mut executor, FORMAT, FORMAT);
        }
        let size = executor.output_size(4, 1);
        let target = texture(
            &gpu,
            size,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        executor
            .render(
                &frame_input(
                    &gpu,
                    &input,
                    (4, 1),
                    Duration::ZERO,
                    ShapeTextures::default(),
                ),
                &frame_output(&gpu, &target, size),
            )
            .expect("resized spatial pass renders");
        // SampleHalf reads the left half at output pixel centres: exactly the
        // first two input texel centres after the output's 2:1 downscale.
        assert_eq!(
            readback_rgba8_image(&gpu, &target, size),
            [255, 0, 0, 255, 0, 255, 0, 255]
        );
    }
}

#[test]
fn gpu_reactive_output_sizes_keep_each_encodes_uniforms() {
    use std::{cell::Cell, rc::Rc};
    let gpu = create_test_device();
    let size = Rc::new(Cell::new((4, 2)));
    let policy = Rc::clone(&size);
    // Spatial passes materialize at input resolution. A final colour pass
    // resizes that result, preserving each encode's uniforms before submit.
    let mut executor = Executor::new(filters::GaussianBlur(0.0_f32))
        .with_output_size(move |_, _| policy.get())
        .then(filters::Brightness(0.0_f32));
    setup(&gpu, &mut executor);
    let pixels = [255, 0, 0, 255, 0, 255, 0, 255];
    let input = upload(&gpu, (2, 1), &pixels);
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let targets: Vec<_> = [(4, 2), (1, 1), (2, 1)]
        .into_iter()
        .map(|dimensions| {
            size.set(dimensions);
            let declared = executor.output_size(2, 1);
            assert_eq!(declared, dimensions);
            let target = texture(
                &gpu,
                declared,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            );
            executor
                .encode_render(
                    &frame_input(
                        &gpu,
                        &input,
                        (2, 1),
                        Duration::ZERO,
                        ShapeTextures::default(),
                    ),
                    &frame_output(&gpu, &target, declared),
                    &mut encoder,
                )
                .expect("each declared size encodes");
            (target, dimensions)
        })
        .collect();
    gpu.queue.submit([encoder.finish()]);
    for (target, dimensions) in targets {
        let actual = readback_rgba8_image(&gpu, &target, dimensions);
        let mut expected = Vec::new();
        for _ in 0..dimensions.1 {
            for x in 0..dimensions.0 {
                let index = usize::try_from((2 * x + 1) / dimensions.0).expect("source index");
                expected.extend_from_slice(&pixels[index * 4..index * 4 + 4]);
            }
        }
        assert_rgba8_close(&actual, &expected, 1, "declared size with shared encoder");
    }
    assert!(
        executor
            .gpu
            .as_ref()
            .expect("set up")
            .intermediates
            .iter()
            .all(|slots| slots.size == (2, 1))
    );
}

#[test]
fn gpu_params_animate_between_frames() {
    let gpu = create_test_device();
    let amount = ScriptedParam::constant(0.0);
    let amount_callback = amount.callback.clone();
    let mut executor = Executor::new(filters::ToneCurve {
        shadows: ScriptedParam::constant(0.0),
        midtones: ScriptedParam::constant(0.0),
        highlights: ScriptedParam::constant(0.0),
        gamma: ScriptedParam::constant(1.0),
        amount,
    });
    setup(&gpu, &mut executor);

    let size = (4, 4);
    let input = upload(&gpu, size, &[128; 64]);
    let output = texture(&gpu, size, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let render = |executor: &mut Executor<_>, delta| {
        executor
            .render(
                &frame_input(&gpu, &input, size, delta, ShapeTextures::default()),
                &frame_output(&gpu, &output, size),
            )
            .expect("render should succeed")
    };

    // Setup snapped every parameter to its snapshot; a static frame is settled.
    assert!(!render(&mut executor, Duration::ZERO));
    assert!(!executor.redraw_hint());

    // A watcher event with an interpolator starts an animation and raises the
    // redraw hint before any frame runs.
    ScriptedParam::fire(
        &amount_callback,
        AnimatedTarget {
            value: 1.0,
            interpolator: Some(Box::new(LinearRamp(Duration::from_millis(100)))),
        },
    );
    assert!(executor.redraw_hint());

    // Mid-flight: half the ramp has elapsed, so the sampled value is halfway
    // and the executor keeps asking for frames.
    assert!(render(&mut executor, Duration::from_millis(50)));
    let mid = executor.animated_values()[4];
    assert!(
        (mid - 0.5).abs() < 0.001,
        "expected the amount halfway through its ramp, got {mid}"
    );
    assert!(executor.redraw_hint());

    // Past the end: the value settles on the target and the demand stops.
    assert!(!render(&mut executor, Duration::from_millis(60)));
    let done = executor.animated_values()[4];
    assert!(
        (done - 1.0).abs() < f32::EPSILON,
        "expected the amount settled on its target, got {done}"
    );
    assert!(!executor.redraw_hint());
}

fn upload_f16(gpu: &TestGpu, size: (u32, u32), pixels: &[[f32; 4]]) -> wgpu::Texture {
    let bytes: Vec<u8> = pixels
        .iter()
        .flat_map(|pixel| {
            pixel
                .iter()
                .flat_map(|channel| half::f16::from_f32(*channel).to_bits().to_le_bytes())
        })
        .collect();
    upload_bytes(gpu, size, wgpu::TextureFormat::Rgba16Float, 8, &bytes)
}

fn readback_f16_image(gpu: &TestGpu, texture: &wgpu::Texture, size: (u32, u32)) -> Vec<[f32; 4]> {
    readback_bytes(gpu, texture, size, 8)
        .as_chunks::<8>()
        .0
        .iter()
        .map(|pixel| {
            std::array::from_fn(|channel| {
                let offset = channel * 2;
                half::f16::from_bits(u16::from_le_bytes([pixel[offset], pixel[offset + 1]]))
                    .to_f32()
            })
        })
        .collect()
}

#[expect(
    clippy::cast_precision_loss,
    reason = "the deterministic test input uses indices smaller than 391"
)]
fn premultiplied_input(size: (usize, usize)) -> Vec<[f32; 4]> {
    (0..size.0 * size.1)
        .map(|index| {
            let alpha = ((index * 29 + 53) % 251) as f32 / 250.0;
            let channel = |salt| ((index * salt + 17) % 239) as f32 / 238.0 * alpha;
            [channel(31), channel(43), channel(67), alpha]
        })
        .collect()
}

fn assert_f16_matches_cpu<F: Filter + CpuFilter>(
    gpu: &TestGpu,
    filter: F,
    size: (u32, u32),
    input: &[[f32; 4]],
    label: &str,
) {
    let params = filter.params();
    let mut expected = input
        .iter()
        .map(|pixel| pixel.map(|channel| half::f16::from_f32(channel).to_f32()))
        .collect::<Vec<_>>();
    filter
        .apply_cpu_image(
            &params,
            &WorkingSpace::LINEAR_DISPLAY_P3,
            &mut CpuImage {
                pixels: &mut expected,
                top: 0,
                size: (
                    usize::try_from(size.0).expect("width fits usize"),
                    usize::try_from(size.1).expect("height fits usize"),
                ),
            },
        )
        .expect("CPU filter should succeed");
    let input_texture = upload_f16(gpu, size, input);
    let output = texture_format(
        gpu,
        size,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        wgpu::TextureFormat::Rgba16Float,
    );
    let mut executor = Executor::new(filter);
    setup_format(
        gpu,
        &mut executor,
        wgpu::TextureFormat::Rgba16Float,
        wgpu::TextureFormat::Rgba16Float,
    );
    executor
        .render(
            &frame_input_format(
                gpu,
                &input_texture,
                size,
                Duration::ZERO,
                ShapeTextures::default(),
                wgpu::TextureFormat::Rgba16Float,
            ),
            &frame_output_format(gpu, &output, size, wgpu::TextureFormat::Rgba16Float),
        )
        .expect("f16 filter render should succeed");
    let actual = readback_f16_image(gpu, &output, size);
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        for channel in 0..4 {
            assert!(
                (actual[channel] - expected[channel]).abs()
                    <= 2.0e-3 * expected[channel].abs().max(1.0),
                "{label}, pixel {index}, channel {channel}: GPU {}, CPU {}",
                actual[channel],
                expected[channel]
            );
        }
    }
}

#[test]
fn cpu_blurs_match_the_wgpu_executor_and_window_aprons() {
    let gpu = create_test_device();
    let size = (23_usize, 17_usize);
    let input = premultiplied_input(size);
    let gaussian = filters::GaussianBlur(1.7_f32);
    assert_f16_matches_cpu(&gpu, gaussian, (23, 17), &input, "GaussianBlur");
    let box_blur = filters::Blur(2.0_f32);
    assert_f16_matches_cpu(&gpu, box_blur, (23, 17), &input, "Blur");

    assert_window_matches_full(&filters::GaussianBlur(1.0_f32), &input, size, 4);
    assert_window_matches_full(&filters::Blur(2.0_f32), &input, size, 2);
}

fn assert_window_matches_full<F: Filter + CpuFilter>(
    filter: &F,
    input: &[[f32; 4]],
    size: (usize, usize),
    apron: usize,
) {
    let params = filter.params();
    let mut full = input.to_vec();
    filter
        .apply_cpu_image(
            &params,
            &WorkingSpace::LINEAR_DISPLAY_P3,
            &mut CpuImage {
                pixels: &mut full,
                top: 0,
                size,
            },
        )
        .expect("full CPU filter should succeed");
    let (top, bottom) = (2, size.1 - 2);
    let mut window = input[top * size.0..bottom * size.0].to_vec();
    filter
        .apply_cpu_image(
            &params,
            &WorkingSpace::LINEAR_DISPLAY_P3,
            &mut CpuImage {
                pixels: &mut window,
                top,
                size,
            },
        )
        .expect("window CPU filter should succeed");
    for y in top + apron..bottom - apron {
        assert_eq!(
            &full[y * size.0..(y + 1) * size.0],
            &window[(y - top) * size.0..(y - top + 1) * size.0],
            "row {y} differs from full-image application"
        );
    }
}

#[test]
fn cpu_image_blend_modes_match_the_wgpu_executor() {
    let gpu = create_test_device();
    let size = (23_usize, 17_usize);
    let input = premultiplied_input(size);
    let auxiliary: Vec<u8> = (0..7 * 5 * 4)
        .map(|index| u8::try_from((index * 37 + 19) % 256).expect("byte is bounded"))
        .collect();
    for mode in [
        filters::BlendMode::Normal,
        filters::BlendMode::Multiply,
        filters::BlendMode::Screen,
        filters::BlendMode::Overlay,
        filters::BlendMode::Darken,
        filters::BlendMode::Lighten,
        filters::BlendMode::SoftLight,
        filters::BlendMode::HardLight,
        filters::BlendMode::Difference,
        filters::BlendMode::Exclusion,
        filters::BlendMode::ColorDodge,
        filters::BlendMode::ColorBurn,
        filters::BlendMode::Hue,
        filters::BlendMode::Saturation,
        filters::BlendMode::Color,
        filters::BlendMode::Luminosity,
    ] {
        let filter = filters::BlendWithImage {
            image: crate::FilterImage::from_rgba8(7, 5, auxiliary.clone()),
            amount: 0.73_f32,
            mode,
        };
        let label = format!("BlendWithImage::{mode:?}");
        assert_f16_matches_cpu(&gpu, filter, (23, 17), &input, &label);
    }

    let auxiliary_f16: Vec<half::f16> = (0..7 * 5 * 4)
        .map(|index| {
            let value = u16::try_from((index * 37 + 19) % 256).expect("texel channel is bounded");
            half::f16::from_f32(f32::from(value) / 256.0)
        })
        .collect();
    let auxiliary_f32: Vec<f32> = (0..7 * 5 * 4)
        .map(|index| {
            let value = u16::try_from((index * 37 + 19) % 256).expect("texel channel is bounded");
            f32::from(value) / 256.0
        })
        .collect();
    for (label, image) in [
        (
            "BlendWithImage::Rgba16Float",
            crate::FilterImage::from_rgba16f(7, 5, &auxiliary_f16),
        ),
        (
            "BlendWithImage::Rgba32Float",
            crate::FilterImage::from_rgba32f(7, 5, &auxiliary_f32),
        ),
    ] {
        let filter = filters::BlendWithImage {
            image,
            amount: 0.73_f32,
            mode: filters::BlendMode::Multiply,
        };
        assert_f16_matches_cpu(&gpu, filter, (23, 17), &input, label);
    }
}

#[test]
fn image_blend_hsl_and_screen_match_gpu_at_alpha_edges() {
    let gpu = create_test_device();
    let input = [
        [0.0, 0.0, 0.0, 0.0],
        [0.6e-6, 0.1e-6, 0.1e-6, 1.0e-6],
        [0.3, 0.05, 0.05, 0.5],
        [0.6, 0.1, 0.1, 1.0],
    ];

    for mode in [filters::BlendMode::Luminosity, filters::BlendMode::Screen] {
        let filter = filters::BlendWithImage {
            image: crate::FilterImage::from_rgba8(1, 1, vec![240, 36, 48, 255]),
            amount: 1.0_f32,
            mode,
        };
        let label = format!("BlendWithImage::{mode:?}, alpha edges");
        assert_f16_matches_cpu(&gpu, filter, (4, 1), &input, &label);
    }
}

#[test]
fn derived_colour_filters_apply_through_cpu_filter() {
    let filter = filters::Brightness(0.25_f32);
    let mut pixels = [[0.2, 0.3, 0.4, 0.5]];
    filter
        .apply_cpu_image(
            &filter.params(),
            &WorkingSpace::LINEAR_DISPLAY_P3,
            &mut CpuImage {
                pixels: &mut pixels,
                top: 0,
                size: (1, 1),
            },
        )
        .expect("derived CPU kernel should succeed");
    assert_eq!(pixels, [[0.325, 0.425, 0.525, 0.5]]);
}

/// Parameter values that change every frame must not accumulate one
/// uniform buffer per frame: a stale buffer — one no encode of the
/// current sequence has bound — is rewritten in place.
#[test]
fn gpu_params_rewrite_buffers_across_frames() {
    let gpu = create_test_device();
    let amount = ScriptedParam::constant(0.0);
    let amount_callback = amount.callback.clone();
    let mut executor = Executor::new(filters::ToneCurve {
        shadows: ScriptedParam::constant(0.0),
        midtones: ScriptedParam::constant(0.0),
        highlights: ScriptedParam::constant(0.0),
        gamma: ScriptedParam::constant(1.0),
        amount,
    });
    setup(&gpu, &mut executor);

    let size = (16, 16);
    let rgba = test_pixels(size.0 * size.1);
    let input = upload(&gpu, size, &rgba);
    let values = [0.2_f32, 0.6, 1.0];
    for (frame, value) in values.iter().enumerate() {
        ScriptedParam::fire(
            &amount_callback,
            AnimatedTarget {
                value: *value,
                interpolator: None,
            },
        );
        let output = texture(
            &gpu,
            size,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        executor
            .encode_render(
                &EffectInput {
                    timing: EffectFrameTiming::new(
                        Duration::ZERO,
                        Duration::ZERO,
                        frame as u64 + 1,
                    ),
                    ..frame_input(&gpu, &input, size, Duration::ZERO, ShapeTextures::default())
                },
                &frame_output(&gpu, &output, size),
                &mut encoder,
            )
            .expect("encode should succeed");
        gpu.queue.submit([encoder.finish()]);
        // A fresh executor rendering the same value is the reference.
        let want = run(
            &gpu,
            filters::ToneCurve {
                shadows: ScriptedParam::constant(0.0),
                midtones: ScriptedParam::constant(0.0),
                highlights: ScriptedParam::constant(0.0),
                gamma: ScriptedParam::constant(1.0),
                amount: ScriptedParam::constant(*value),
            },
            size,
            &rgba,
            ShapeTextures::default(),
        );
        assert_rgba8_close(
            &readback_rgba8_image(&gpu, &output, size),
            &want,
            1,
            "animated frame",
        );
    }
    let buffers: usize = executor
        .gpu
        .as_ref()
        .expect("executor set up")
        .passes
        .iter()
        .map(|pass| pass.params.buffer_count())
        .sum();
    assert_eq!(buffers, 1, "changing params must reuse the stale buffer");
}

// ============================================================================
// Gallery
// ============================================================================

fn write_png(path: &Path, (width, height): (u32, u32), rgba: &[u8]) {
    RgbaImage::from_raw(width, height, rgba.to_vec())
        .expect("rgba buffer length should match dimensions")
        .save(path)
        .expect("failed to save png");
}

fn scale_to_u8(numerator: u32, denominator: u32) -> u8 {
    u8::try_from(u64::from(numerator) * 255 / u64::from(denominator.max(1)))
        .expect("scaled channel fits u8")
}

fn clamp_i32_to_u8(value: i32) -> u8 {
    u8::try_from(value.clamp(0, i32::from(u8::MAX))).expect("clamped channel fits u8")
}

/// Gradients, a checkerboard and a ring: something every filter visibly
/// changes.
fn create_test_input_rgba(width: u32, height: u32) -> Vec<u8> {
    let max_x = width.saturating_sub(1).max(1);
    let max_y = height.saturating_sub(1).max(1);
    let min_dimension = i64::from(width.min(height));
    let inner_edge_radius = min_dimension * 28 / 100;
    let outer_edge_radius = min_dimension * 32 / 100;
    let center_x = i64::from(width) / 2;
    let center_y = i64::from(height) / 2;

    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let checker = if ((x / 16) + (y / 16)) % 2 == 0 {
                32
            } else {
                -32
            };
            let dx = i64::from(x) - center_x;
            let dy = i64::from(y) - center_y;
            let ring_sq = dx * dx + dy * dy;
            let edge = if ring_sq > inner_edge_radius * inner_edge_radius
                && ring_sq < outer_edge_radius * outer_edge_radius
            {
                80
            } else {
                0
            };
            let inverse_gradient = u8::try_from(
                u64::from(max_x - x) * u64::from(max_y - y) * 255
                    / (u64::from(max_x) * u64::from(max_y)),
            )
            .expect("inverse channel fits u8");
            data.extend([
                clamp_i32_to_u8(i32::from(scale_to_u8(x, max_x)) + checker + edge),
                clamp_i32_to_u8(i32::from(scale_to_u8(y, max_y)) - checker + edge),
                clamp_i32_to_u8(i32::from(inverse_gradient) + edge),
                255,
            ]);
        }
    }
    data
}

/// Renders every built-in filter into `/tmp/waterui_filter_gallery/`, for
/// reading the images.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the gallery enumerates every filter case in one visual artifact generator"
)]
fn gpu_export_filter_gallery_images() {
    use filters::*;

    let gpu = create_test_device();
    let size = (256, 256);
    let output_dir = PathBuf::from("/tmp/waterui_filter_gallery");
    fs::create_dir_all(&output_dir).expect("failed to create output directory");
    let input = create_test_input_rgba(size.0, size.1);
    write_png(&output_dir.join("input.png"), size, &input);

    macro_rules! export_filter {
        ($name:literal, $filter:expr) => {{
            let result = run(&gpu, $filter, size, &input, ShapeTextures::default());
            write_png(&output_dir.join($name), size, &result);
        }};
    }

    export_filter!("brightness.png", Brightness(0.2_f32));
    export_filter!("contrast.png", Contrast(1.4_f32));
    export_filter!("saturation.png", Saturation(1.8_f32));
    export_filter!("grayscale.png", Grayscale(1.0_f32));
    export_filter!("hue_rotation.png", HueRotation(120.0_f32));
    export_filter!("sepia.png", Sepia(1.0_f32));
    export_filter!("invert.png", Invert);
    export_filter!("blur.png", Blur(3.0_f32));
    export_filter!("gaussian_blur.png", GaussianBlur(2.0_f32));
    export_filter!("motion_blur.png", MotionBlur(6.0_f32, 30.0_f32));
    export_filter!("zoom_blur.png", ZoomBlur(0.2_f32, 0.5_f32, 0.5_f32));
    export_filter!("sharpen.png", Sharpen(1.5_f32));
    export_filter!(
        "chain_blur_brightness.png",
        Blur(2.0_f32)
            .then(Brightness(0.15_f32))
            .then(Contrast(1.2_f32))
    );
    export_filter!("sobel.png", Sobel);
    export_filter!("prewitt.png", Prewitt);
    export_filter!("edge_work.png", EdgeWork([2.0_f32, 1.5]));
    export_filter!("median3x3.png", Median3x3);
    export_filter!("morphology_min.png", MorphologyMin);
    export_filter!("morphology_max.png", MorphologyMax);
    export_filter!("morphology_gradient.png", MorphologyGradient);
    // 3x3 sharpen kernel: identity * 5 minus the four neighbours.
    export_filter!(
        "convolution3x3_sharpen.png",
        Convolution3x3([0.0_f32, -1.0, 0.0, -1.0, 5.0, -1.0, 0.0, -1.0, 0.0])
    );
    // 5x5 identity (centre = 1, rest = 0). Output should match input.
    export_filter!("convolution5x5_identity.png", {
        let mut kernel = [0.0_f32; 25];
        kernel[12] = 1.0;
        Convolution5x5(kernel)
    });
    export_filter!("photo_effect_mono.png", PhotoEffectMono);
    export_filter!("photo_effect_noir.png", PhotoEffectNoir);
    export_filter!("photo_effect_chrome.png", PhotoEffectChrome);
    export_filter!("photo_effect_instant.png", PhotoEffectInstant);
    export_filter!("photo_effect_fade.png", PhotoEffectFade);
    export_filter!("photo_effect_process.png", PhotoEffectProcess);
    export_filter!("photo_effect_tonal.png", PhotoEffectTonal);
    export_filter!("photo_effect_transfer.png", PhotoEffectTransfer);
    export_filter!(
        "chain_chrome_brightness_contrast.png",
        PhotoEffectChrome
            .then(Brightness(0.05_f32))
            .then(Contrast(1.1_f32))
    );
    export_filter!("chain_sobel_then_tonal.png", Sobel.then(PhotoEffectTonal));
    export_filter!("vibrance.png", Vibrance(0.8_f32));
    export_filter!("vignette.png", Vignette(0.55_f32, 0.35_f32));
    export_filter!(
        "bloom.png",
        Bloom {
            radius: 8.0_f32,
            intensity: 1.2,
            threshold: 0.6,
        }
    );
    export_filter!(
        "gloom.png",
        Gloom {
            radius: 8.0_f32,
            intensity: 0.8,
            threshold: 0.4,
        }
    );
    export_filter!(
        "unsharp_mask.png",
        UnsharpMask {
            radius: 4.0_f32,
            intensity: 1.5,
        }
    );
    // A spatial pass ahead of bloom: its original is the blurred
    // intermediate, not the chain's input.
    export_filter!(
        "chain_blur_then_bloom.png",
        Blur(2.0_f32).then(Bloom {
            radius: 8.0_f32,
            intensity: 1.2,
            threshold: 0.6,
        })
    );
    export_filter!("twirl.png", TwirlDistortion([0.5_f32, 0.5, 0.4, 180.0]));
    export_filter!("bump.png", BumpDistortion([0.5_f32, 0.5, 0.4, 0.8]));
    export_filter!("pixellate.png", Pixellate(12.0_f32));
    export_filter!("crystallize.png", Crystallize(16.0_f32));
    export_filter!("dot_halftone.png", DotHalftone([8.0_f32, 45.0, 0.5, 0.5]));
    export_filter!(
        "perspective_transform.png",
        PerspectiveTransform([0.1_f32, 0.1, 0.9, 0.0, 1.0, 1.0, 0.0, 0.9])
    );
    export_filter!(
        "blend_with_image.png",
        BlendWithImage {
            image: crate::FilterImage::from_rgba8(1, 1, vec![255, 128, 0, 255]),
            amount: 0.6_f32,
            mode: BlendMode::Overlay,
        }
    );

    // An application-supplied post-process shader: scanlines whose strength
    // is its first parameter, plus a slight edge vignette.
    let mut scanlines = ShaderEffect::new(
        "@fragment
        fn main(in: VertexOutput) -> @location(0) vec4<f32> {
            let color = textureSample(input_texture, input_sampler, in.uv);
            let row = u32(in.position.y);
            let line = select(1.0, 0.0, (row / 2u) % 2u == 1u);
            let centered = in.uv - vec2<f32>(0.5);
            let vignette = 1.0 - dot(centered, centered);
            let shade = mix(1.0, line, effect_param(0u)) * vignette;
            return vec4<f32>(color.rgb * shade, color.a);
        }",
    )
    .expect("the gallery scanline shader is valid WGSL")
    .param(0.6);
    setup_shader_effect(&gpu, &mut scanlines, FORMAT);
    let input_texture = upload(&gpu, size, &input);
    let result = render_shader_effect(
        &gpu,
        &mut scanlines,
        &input_texture,
        size,
        EffectFrameTiming::new(Duration::ZERO, Duration::ZERO, 0),
    );
    write_png(
        &output_dir.join("shader_effect_scanlines.png"),
        size,
        &result,
    );
}

/// Two encodes of one effect at different sizes in a single encoder must
/// stay independent: parameters used to be a queue write into one buffer,
/// so both recorded passes ran with the last encode's `size`.
#[test]
fn gpu_one_effect_two_sizes_one_encoder() {
    use filters::GaussianBlur;
    let gpu = create_test_device();
    let mut executor = Executor::new(GaussianBlur(8.0_f32));
    setup(&gpu, &mut executor);
    let size_a = (64, 64);
    let size_b = (64, 32);
    let rgba_a = test_pixels(size_a.0 * size_a.1);
    let rgba_b = test_pixels(size_b.0 * size_b.1);
    let input_a = upload(&gpu, size_a, &rgba_a);
    let input_b = upload(&gpu, size_b, &rgba_b);
    let output_a = texture(
        &gpu,
        size_a,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let output_b = texture(
        &gpu,
        size_b,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("two-size encode"),
        });
    executor
        .encode_render(
            &frame_input(
                &gpu,
                &input_a,
                size_a,
                Duration::ZERO,
                ShapeTextures::default(),
            ),
            &frame_output(&gpu, &output_a, size_a),
            &mut encoder,
        )
        .expect("first encode should succeed");
    executor
        .encode_render(
            &frame_input(
                &gpu,
                &input_b,
                size_b,
                Duration::ZERO,
                ShapeTextures::default(),
            ),
            &frame_output(&gpu, &output_b, size_b),
            &mut encoder,
        )
        .expect("second encode should succeed");
    gpu.queue.submit([encoder.finish()]);
    // References: the same effect, one encode per submission.
    let want_a = run(
        &gpu,
        GaussianBlur(8.0_f32),
        size_a,
        &rgba_a,
        ShapeTextures::default(),
    );
    let want_b = run(
        &gpu,
        GaussianBlur(8.0_f32),
        size_b,
        &rgba_b,
        ShapeTextures::default(),
    );
    assert_rgba8_close(
        &readback_rgba8_image(&gpu, &output_a, size_a),
        &want_a,
        0,
        "first encode",
    );
    assert_rgba8_close(
        &readback_rgba8_image(&gpu, &output_b, size_b),
        &want_b,
        0,
        "second encode",
    );
}

/// Intermediate slots for a size a frame does not encode against are
/// dropped; sizes encoded in the same frame all stay.
#[test]
fn gpu_intermediates_evicted_when_size_unused() {
    use filters::GaussianBlur;
    fn encode<F: Filter>(
        gpu: &TestGpu,
        executor: &mut Executor<F>,
        size: (u32, u32),
        sequence: u64,
    ) {
        let input = upload(gpu, size, &test_pixels(size.0 * size.1));
        let output = texture(
            gpu,
            size,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        executor
            .encode_render(
                &EffectInput {
                    timing: EffectFrameTiming::new(Duration::ZERO, Duration::ZERO, sequence),
                    ..frame_input(gpu, &input, size, Duration::ZERO, ShapeTextures::default())
                },
                &frame_output(gpu, &output, size),
                &mut encoder,
            )
            .expect("encode should succeed");
        gpu.queue.submit([encoder.finish()]);
    }
    fn sizes<F: Filter>(executor: &Executor<F>) -> Vec<(u32, u32)> {
        executor
            .gpu
            .as_ref()
            .expect("executor set up")
            .intermediates
            .iter()
            .map(|intermediates| intermediates.size)
            .collect()
    }
    fn slots_of<F: Filter>(executor: &Executor<F>, size: (u32, u32)) -> wgpu::TextureView {
        executor
            .gpu
            .as_ref()
            .expect("executor set up")
            .intermediates
            .iter()
            .find(|intermediates| intermediates.size == size)
            .expect("size's intermediates exist")
            .views[0]
            .clone()
    }
    let gpu = create_test_device();
    let mut executor = Executor::new(GaussianBlur(4.0_f32));
    setup(&gpu, &mut executor);
    let size_a = (64, 64);
    let size_b = (64, 32);
    encode(&gpu, &mut executor, size_a, 1);
    assert_eq!(sizes(&executor), vec![size_a]);
    // A size the previous frame used is still live.
    encode(&gpu, &mut executor, size_b, 2);
    let mut got = sizes(&executor);
    got.sort_unstable();
    assert_eq!(got, vec![size_b, size_a]);
    // A size unused for a whole frame is dropped.
    encode(&gpu, &mut executor, size_b, 3);
    assert_eq!(sizes(&executor), vec![size_b]);

    // Steady state with two sizes alternating every frame: both sizes'
    // intermediates stay allocated and are reused, not dropped and
    // reallocated on each encode.
    encode(&gpu, &mut executor, size_a, 4);
    encode(&gpu, &mut executor, size_b, 4);
    assert_eq!(sizes(&executor).len(), 2);
    let frame4 = (slots_of(&executor, size_a), slots_of(&executor, size_b));
    encode(&gpu, &mut executor, size_a, 5);
    encode(&gpu, &mut executor, size_b, 5);
    assert_eq!(sizes(&executor).len(), 2);
    let frame5 = (slots_of(&executor, size_a), slots_of(&executor, size_b));
    encode(&gpu, &mut executor, size_a, 6);
    encode(&gpu, &mut executor, size_b, 6);
    assert_eq!(sizes(&executor).len(), 2);
    let frame6 = (slots_of(&executor, size_a), slots_of(&executor, size_b));
    assert!(
        frame4.0 == frame5.0 && frame5.0 == frame6.0,
        "size A encoded every frame must keep its intermediates"
    );
    assert!(
        frame4.1 == frame5.1 && frame5.1 == frame6.1,
        "size B encoded every frame must keep its intermediates"
    );
}

// ============================================================================
// ShaderEffect
// ============================================================================

fn setup_shader_effect(
    gpu: &TestGpu,
    effect: &mut ShaderEffect,
    input_format: wgpu::TextureFormat,
) {
    let ctx = EffectContext {
        device: &gpu.device,
        queue: &gpu.queue,
        input_format,
        output_format: FORMAT,
    };
    pollster::block_on(effect.setup(&ctx)).expect("shader effect setup should succeed");
}

/// Encodes one frame of `effect` at `timing` into a fresh RGBA8 output of
/// `size` and reads it back.
fn render_shader_effect(
    gpu: &TestGpu,
    effect: &mut ShaderEffect,
    input: &wgpu::Texture,
    size: (u32, u32),
    timing: EffectFrameTiming,
) -> Vec<u8> {
    let output = texture(
        gpu,
        size,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    effect
        .render(
            &EffectInput {
                timing,
                ..frame_input(gpu, input, size, Duration::ZERO, ShapeTextures::default())
            },
            &frame_output(gpu, &output, size),
        )
        .expect("shader effect render should succeed");
    readback_rgba8_image(gpu, &output, size)
}

/// A user shader samples the input, reads the frame time from the host's
/// timeline, and reads a reactive parameter at the value its latest change
/// delivered — and, being animated, asks for the next frame.
#[test]
fn gpu_shader_effect_reads_input_time_and_params() {
    let gpu = create_test_device();
    let size = (4, 4);
    let input = upload(&gpu, size, &[10, 20, 200, 255].repeat(16));
    let strength = ScriptedParam::constant(0.25);
    let strength_callback = strength.callback.clone();
    let (mut effect, _subscription) = ShaderEffect::new(
        "@fragment
        fn main(in: VertexOutput) -> @location(0) vec4<f32> {
            let source = textureSample(input_texture, input_sampler, in.uv);
            return vec4<f32>(effect_param(0u), fract(uniforms.time), source.b, 1.0);
        }",
    )
    .expect("the test shader is valid WGSL")
    .animated()
    .watch_param(&strength);
    setup_shader_effect(&gpu, &mut effect, FORMAT);

    ScriptedParam::fire(
        &strength_callback,
        AnimatedTarget {
            value: 0.75,
            interpolator: None,
        },
    );
    assert!(effect.redraw_hint(), "a parameter change wants a frame");

    let output = texture(
        &gpu,
        size,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let needs_redraw = effect
        .render(
            &EffectInput {
                timing: EffectFrameTiming::new(
                    Duration::from_millis(1500),
                    Duration::from_millis(16),
                    90,
                ),
                ..frame_input(&gpu, &input, size, Duration::ZERO, ShapeTextures::default())
            },
            &frame_output(&gpu, &output, size),
        )
        .expect("shader effect render should succeed");
    assert!(needs_redraw, "an animated shader asks for the next frame");

    let pixels = readback_rgba8_image(&gpu, &output, size);
    for &[red, green, blue, alpha] in pixels.as_chunks::<4>().0 {
        assert!(red.abs_diff(191) <= 1, "param 0.75 -> red {red}");
        assert!(green.abs_diff(128) <= 1, "time 1.5 s -> green {green}");
        assert_eq!(blue, 200, "the input's blue channel passes through");
        assert_eq!(alpha, 255);
    }
}

/// Two encodes of one shader effect at different sizes in one encoder keep
/// their own uniforms: a queue write into a single buffer would land before
/// the submit and give both passes the second encode's resolution. On the
/// next frame, with new uniform values, both encodes reuse their bind
/// groups.
#[test]
fn gpu_shader_effect_two_sizes_one_encoder() {
    let gpu = create_test_device();
    let mut effect = ShaderEffect::new(
        "@fragment
        fn main(in: VertexOutput) -> @location(0) vec4<f32> {
            return vec4<f32>(uniforms.resolution / 255.0, fract(uniforms.time), 1.0);
        }",
    )
    .expect("the test shader is valid WGSL");
    setup_shader_effect(&gpu, &mut effect, FORMAT);

    let sizes = [(8, 8), (16, 4)];
    // The host keeps one input view per size across frames.
    let inputs = sizes.map(|size| {
        let texture = upload(&gpu, size, &test_pixels(size.0 * size.1));
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    });
    let outputs = sizes.map(|size| {
        texture(
            &gpu,
            size,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        )
    });
    let mut frame = |sequence: u64, time: Duration, blue: u8| {
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("two-size shader effect encode"),
            });
        for ((size, (texture, view)), output) in sizes.iter().zip(&inputs).zip(&outputs) {
            effect
                .encode_render(
                    &EffectInput {
                        view: view.clone(),
                        timing: EffectFrameTiming::new(time, Duration::ZERO, sequence),
                        ..frame_input(
                            &gpu,
                            texture,
                            *size,
                            Duration::ZERO,
                            ShapeTextures::default(),
                        )
                    },
                    &frame_output(&gpu, output, *size),
                    &mut encoder,
                )
                .expect("encode should succeed");
        }
        gpu.queue.submit([encoder.finish()]);
        for (size, output) in sizes.iter().zip(&outputs) {
            let pixels = readback_rgba8_image(&gpu, output, *size);
            let width = u8::try_from(size.0).expect("test width fits u8");
            let height = u8::try_from(size.1).expect("test height fits u8");
            assert_rgba8_close(
                &pixels,
                &[width, height, blue, 255].repeat(pixels.len() / 4),
                1,
                "each encode reads its own uniforms",
            );
        }
        effect.cached_bind_groups()
    };
    let first = frame(1, Duration::ZERO, 0);
    let second = frame(2, Duration::from_millis(500), 128);
    assert_eq!(first.len(), 2, "one bind group per encode");
    assert_eq!(second, first, "the next frame reuses both bind groups");
}

/// A shader that reads `input_sampler` needs a filterable input; one that
/// only loads texels runs on any float input.
#[test]
fn gpu_shader_effect_sampling_requires_a_filterable_input() {
    let gpu = create_test_device();
    let unfilterable = wgpu::TextureFormat::Rgba32Float;
    assert!(
        !super::filterable(unfilterable, gpu.device.features()),
        "the test device does not filter Rgba32Float"
    );

    let mut sampling = ShaderEffect::new(
        "@fragment
        fn main(in: VertexOutput) -> @location(0) vec4<f32> {
            return textureSample(input_texture, input_sampler, in.uv);
        }",
    )
    .expect("the sampling shader is valid WGSL");
    let ctx = EffectContext {
        device: &gpu.device,
        queue: &gpu.queue,
        input_format: unfilterable,
        output_format: FORMAT,
    };
    assert_eq!(
        pollster::block_on(sampling.setup(&ctx)),
        Err(EffectSetupError::InputNotFilterable {
            format: unfilterable
        })
    );

    let mut loading = ShaderEffect::new(
        "@fragment
        fn main(in: VertexOutput) -> @location(0) vec4<f32> {
            return textureLoad(input_texture, vec2<i32>(in.position.xy), 0);
        }",
    )
    .expect("the loading shader is valid WGSL");
    setup_shader_effect(&gpu, &mut loading, unfilterable);
    let size = (2, 2);
    let input = upload_f32(&gpu, size, &[[0.25, 0.5, 0.75, 1.0]; 4]);
    let output = texture(
        &gpu,
        size,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    loading
        .render(
            &frame_input_format(
                &gpu,
                &input,
                size,
                Duration::ZERO,
                ShapeTextures::default(),
                unfilterable,
            ),
            &frame_output(&gpu, &output, size),
        )
        .expect("a texel-loading shader renders an unfilterable input");
    assert_rgba8_close(
        &readback_rgba8_image(&gpu, &output, size),
        &[64, 128, 191, 255].repeat(4),
        1,
        "loaded texels",
    );
}
