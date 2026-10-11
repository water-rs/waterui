//! Filters with auxiliary images: blends, masks, maps, LUTs and transitions.
//!
//! Each is one spatial stage whose snippet reads the images the filter holds
//! through its `aux` arguments. Discrete selectors (blend modes,
//! directions, LUT sizes) are parameters the filter never animates.

use crate::{
    AuxSource, Filter, FilterImage, FilterParam, Footprint, ImageVisitor, LutImage, OperatingSpace,
    ParamSource, Placed, SignalVisitor, SpatialFilter, SpatialStage, StageCollector,
    filters::footprint, kind,
};

/// A spatial stage reading the filter's images `0..aux.len()`.
const fn stage(
    name: &'static str,
    source: &'static str,
    params: &'static [ParamSource],
    aux: &'static [AuxSource],
) -> SpatialStage {
    SpatialStage {
        name,
        source,
        params,
        space: OperatingSpace::Working,
        shape: None,
        aux,
    }
}

/// Blend operators for combining the input with an auxiliary image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    /// Alpha compositing with the auxiliary image.
    Normal,
    /// Multiply input and auxiliary colors.
    Multiply,
    /// Screen the auxiliary image over the input.
    Screen,
    /// Overlay the auxiliary image onto the input.
    Overlay,
    /// Keep the darker channel from each source.
    Darken,
    /// Keep the lighter channel from each source.
    Lighten,
    /// Apply a soft light blend.
    SoftLight,
    /// Apply a hard light blend.
    HardLight,
    /// Subtract shared luminance to emphasize differences.
    Difference,
    /// Blend using the exclusion operator.
    Exclusion,
    /// Brighten the input with color dodge.
    ColorDodge,
    /// Darken the input with color burn.
    ColorBurn,
    /// Hue from the auxiliary, saturation and luminance from the input.
    Hue,
    /// Saturation from the auxiliary, hue and luminance from the input.
    Saturation,
    /// Hue and saturation from the auxiliary, luminance from the input.
    Color,
    /// Luminance from the auxiliary, hue and saturation from the input.
    Luminosity,
}

impl BlendMode {
    /// The selector the blend snippet switches on.
    const fn token(self) -> f32 {
        match self {
            Self::Normal => 0.0,
            Self::Multiply => 1.0,
            Self::Screen => 2.0,
            Self::Overlay => 3.0,
            Self::Darken => 4.0,
            Self::Lighten => 5.0,
            Self::SoftLight => 6.0,
            Self::HardLight => 7.0,
            Self::Difference => 8.0,
            Self::Exclusion => 9.0,
            Self::ColorDodge => 10.0,
            Self::ColorBurn => 11.0,
            Self::Hue => 12.0,
            Self::Saturation => 13.0,
            Self::Color => 14.0,
            Self::Luminosity => 15.0,
        }
    }
}

/// Swipe direction for [`SwipeTransitionToImage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionDirection {
    /// Reveal from left to right.
    LeftToRight,
    /// Reveal from right to left.
    RightToLeft,
    /// Reveal from top to bottom.
    TopToBottom,
    /// Reveal from bottom to top.
    BottomToTop,
}

impl TransitionDirection {
    /// The selector the swipe snippet switches on.
    const fn token(self) -> f32 {
        match self {
            Self::LeftToRight => 0.0,
            Self::RightToLeft => 1.0,
            Self::TopToBottom => 2.0,
            Self::BottomToTop => 3.0,
        }
    }
}

/// The LUT cube size as the parameter the LUT snippet reads.
fn lut_size(lut: &LutImage) -> f32 {
    f32::from(u16::try_from(lut.size()).expect("a LUT cube size fits in u16"))
}

const BLEND_WITH_IMAGE: SpatialStage = stage(
    "blend_with_image",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/composite/blend_with_image.wgsl"
    )),
    &[ParamSource::Param(0), ParamSource::Param(1)],
    &[AuxSource::Image(0)],
);

/// Blends the input's unpremultiplied colour with the sampled auxiliary
/// texel, mixes by `amount`, then re-premultiplies with the input's alpha.
#[derive(Debug, Clone)]
pub struct BlendWithImage<A: FilterParam = f32> {
    /// Image blended over the input.
    pub image: FilterImage,
    /// Blend strength, clamped to [0, 1].
    pub amount: A,
    /// Blend operator to apply.
    pub mode: BlendMode,
}

impl<A: FilterParam> Filter for BlendWithImage<A> {
    type Kind = kind::Spatial;
    type Params = [f32; 2];
    const IMAGES: usize = 1;

    fn params(&self) -> [f32; 2] {
        [self.amount.snapshot(), self.mode.token()]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial(Placed::new(&BLEND_WITH_IMAGE));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.amount);
    }

    fn visit_images<V: ImageVisitor>(&self, v: &mut V) {
        v.visit(0, &self.image);
    }
}

impl<A: FilterParam> SpatialFilter for BlendWithImage<A> {
    fn footprint_of(_params: &[f32; 2]) -> Footprint {
        Footprint::ZERO
    }
}

const MASKED_BLUR: SpatialStage = stage(
    "masked_blur",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/composite/masked_blur.wgsl"
    )),
    &[ParamSource::Param(0), ParamSource::Param(1)],
    &[AuxSource::Image(0)],
);

/// Box-blurs the input where a mask image (red channel) is set.
#[derive(Debug, Clone)]
pub struct MaskedBlur<P: FilterParam = f32> {
    /// Blur mask image.
    pub mask: FilterImage,
    /// Blur radius in pixels.
    pub radius: P,
    /// Blur strength multiplier, clamped to [0, 1].
    pub strength: P,
}

impl<P: FilterParam> Filter for MaskedBlur<P> {
    type Kind = kind::Spatial;
    type Params = [f32; 2];
    const IMAGES: usize = 1;

    fn params(&self) -> [f32; 2] {
        [self.radius.snapshot(), self.strength.snapshot()]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial(Placed::new(&MASKED_BLUR));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.radius);
        v.visit(1, &self.strength);
    }

    fn visit_images<V: ImageVisitor>(&self, v: &mut V) {
        v.visit(0, &self.mask);
    }
}

impl<P: FilterParam> SpatialFilter for MaskedBlur<P> {
    fn footprint_of(params: &[f32; 2]) -> Footprint {
        Footprint::pixels(footprint::rounded(params[0]))
    }
}

const TRANSITION_TO_IMAGE: SpatialStage = stage(
    "transition",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/composite/transition.wgsl"
    )),
    &[ParamSource::Param(0), ParamSource::Param(1)],
    &[AuxSource::Image(0)],
);

/// Reveals a target image left to right as `progress` goes from 0 to 1.
#[derive(Debug, Clone)]
pub struct TransitionToImage<P: FilterParam = f32> {
    /// Target image revealed by the transition.
    pub target: FilterImage,
    /// Transition progress from `0.0` to `1.0`.
    pub progress: P,
    /// Feathering amount around the transition boundary.
    pub softness: P,
}

impl<P: FilterParam> Filter for TransitionToImage<P> {
    type Kind = kind::Spatial;
    type Params = [f32; 2];
    const IMAGES: usize = 1;

    fn params(&self) -> [f32; 2] {
        [self.progress.snapshot(), self.softness.snapshot()]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial(Placed::new(&TRANSITION_TO_IMAGE));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.progress);
        v.visit(1, &self.softness);
    }

    fn visit_images<V: ImageVisitor>(&self, v: &mut V) {
        v.visit(0, &self.target);
    }
}

impl<P: FilterParam> SpatialFilter for TransitionToImage<P> {
    fn footprint_of(_params: &[f32; 2]) -> Footprint {
        Footprint::ZERO
    }
}

const DISPLACEMENT_WARP: SpatialStage = stage(
    "displacement_warp",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/composite/displacement_warp.wgsl"
    )),
    &[ParamSource::Param(0), ParamSource::Param(1)],
    &[AuxSource::Image(0)],
);

/// Warps the input by a displacement map (red and green mapped from
/// [0, 1] to [-1, 1]), scaled in pixels.
#[derive(Debug, Clone)]
pub struct DisplacementWarp<P: FilterParam = f32> {
    /// Displacement map image.
    pub map: FilterImage,
    /// Horizontal displacement scale, in pixels.
    pub scale_x: P,
    /// Vertical displacement scale, in pixels.
    pub scale_y: P,
}

impl<P: FilterParam> Filter for DisplacementWarp<P> {
    type Kind = kind::Spatial;
    type Params = [f32; 2];
    const IMAGES: usize = 1;

    fn params(&self) -> [f32; 2] {
        [self.scale_x.snapshot(), self.scale_y.snapshot()]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial(Placed::new(&DISPLACEMENT_WARP));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.scale_x);
        v.visit(1, &self.scale_y);
    }

    fn visit_images<V: ImageVisitor>(&self, v: &mut V) {
        v.visit(0, &self.map);
    }
}

impl<P: FilterParam> SpatialFilter for DisplacementWarp<P> {
    fn footprint_of(params: &[f32; 2]) -> Footprint {
        Footprint::pixels(footprint::offset(params[0]).max(footprint::offset(params[1])))
    }
}

const GUIDED_SMOOTH: SpatialStage = stage(
    "guided_smooth",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/composite/guided_smooth.wgsl"
    )),
    &[
        ParamSource::Param(0),
        ParamSource::Param(1),
        ParamSource::Param(2),
    ],
    &[AuxSource::Image(0)],
);

/// Smooths the input with an edge-preserving blur guided by another image.
#[derive(Debug, Clone)]
pub struct GuidedSmooth<P: FilterParam = f32> {
    /// Guide image that preserves major edges.
    pub guide: FilterImage,
    /// Filter radius in pixels.
    pub radius: P,
    /// Range sensitivity.
    pub range_sigma: P,
    /// Blend amount for the smoothed result.
    pub amount: P,
}

impl<P: FilterParam> Filter for GuidedSmooth<P> {
    type Kind = kind::Spatial;
    type Params = [f32; 3];
    const IMAGES: usize = 1;

    fn params(&self) -> [f32; 3] {
        [
            self.radius.snapshot(),
            self.range_sigma.snapshot(),
            self.amount.snapshot(),
        ]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial(Placed::new(&GUIDED_SMOOTH));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.radius);
        v.visit(1, &self.range_sigma);
        v.visit(2, &self.amount);
    }

    fn visit_images<V: ImageVisitor>(&self, v: &mut V) {
        v.visit(0, &self.guide);
    }
}

impl<P: FilterParam> SpatialFilter for GuidedSmooth<P> {
    fn footprint_of(params: &[f32; 3]) -> Footprint {
        Footprint::pixels(footprint::rounded(params[0]))
    }
}

const DEPTH_AWARE_BLUR: SpatialStage = stage(
    "depth_aware_blur",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/composite/depth_aware_blur.wgsl"
    )),
    &[
        ParamSource::Param(0),
        ParamSource::Param(1),
        ParamSource::Param(2),
    ],
    &[AuxSource::Image(0)],
);

/// Blurs the input by a depth map (red channel), like a camera's depth of
/// field.
#[derive(Debug, Clone)]
pub struct DepthAwareBlur<P: FilterParam = f32> {
    /// Depth map that drives blur strength.
    pub depth: FilterImage,
    /// Depth plane that remains in focus.
    pub focus_depth: P,
    /// Simulated aperture size.
    pub aperture: P,
    /// Maximum blur radius, in pixels.
    pub max_radius: P,
}

impl<P: FilterParam> Filter for DepthAwareBlur<P> {
    type Kind = kind::Spatial;
    type Params = [f32; 3];
    const IMAGES: usize = 1;

    fn params(&self) -> [f32; 3] {
        [
            self.focus_depth.snapshot(),
            self.aperture.snapshot(),
            self.max_radius.snapshot(),
        ]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial(Placed::new(&DEPTH_AWARE_BLUR));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.focus_depth);
        v.visit(1, &self.aperture);
        v.visit(2, &self.max_radius);
    }

    fn visit_images<V: ImageVisitor>(&self, v: &mut V) {
        v.visit(0, &self.depth);
    }
}

impl<P: FilterParam> SpatialFilter for DepthAwareBlur<P> {
    fn footprint_of(params: &[f32; 3]) -> Footprint {
        Footprint::pixels((params[1].max(0.0) * params[2].max(0.0)).round())
    }
}

const TEMPORAL_DENOISE: SpatialStage = stage(
    "temporal_denoise",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/composite/temporal_denoise.wgsl"
    )),
    &[ParamSource::Param(0)],
    &[AuxSource::Image(0), AuxSource::Image(1)],
);

/// Denoises the current frame by mixing in the previous one, reprojected
/// through motion vectors.
#[derive(Debug, Clone)]
pub struct TemporalDenoise<P: FilterParam = f32> {
    /// Previous filtered frame.
    pub history: FilterImage,
    /// Motion vectors (red and green mapped from [0, 1] to [-1, 1], in pixels).
    pub motion: FilterImage,
    /// Weight assigned to history data.
    pub history_weight: P,
}

impl<P: FilterParam> Filter for TemporalDenoise<P> {
    type Kind = kind::Spatial;
    type Params = [f32; 1];
    const IMAGES: usize = 2;

    fn params(&self) -> [f32; 1] {
        [self.history_weight.snapshot()]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial(Placed::new(&TEMPORAL_DENOISE));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.history_weight);
    }

    fn visit_images<V: ImageVisitor>(&self, v: &mut V) {
        v.visit(0, &self.history);
        v.visit(1, &self.motion);
    }
}

impl<P: FilterParam> SpatialFilter for TemporalDenoise<P> {
    fn footprint_of(_params: &[f32; 1]) -> Footprint {
        Footprint::ZERO
    }
}

const BACKGROUND_REPLACE: SpatialStage = stage(
    "background_replace",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/composite/background_replace.wgsl"
    )),
    &[ParamSource::Param(0)],
    &[AuxSource::Image(0), AuxSource::Image(1)],
);

/// Composites the input over a replacement background through a matte.
#[derive(Debug, Clone)]
pub struct BackgroundReplace<P: FilterParam = f32> {
    /// Foreground matte image (red channel).
    pub matte: FilterImage,
    /// Replacement background image.
    pub background: FilterImage,
    /// Softening factor for matte edges.
    pub edge_softness: P,
}

impl<P: FilterParam> Filter for BackgroundReplace<P> {
    type Kind = kind::Spatial;
    type Params = [f32; 1];
    const IMAGES: usize = 2;

    fn params(&self) -> [f32; 1] {
        [self.edge_softness.snapshot()]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial(Placed::new(&BACKGROUND_REPLACE));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.edge_softness);
    }

    fn visit_images<V: ImageVisitor>(&self, v: &mut V) {
        v.visit(0, &self.matte);
        v.visit(1, &self.background);
    }
}

impl<P: FilterParam> SpatialFilter for BackgroundReplace<P> {
    fn footprint_of(_params: &[f32; 1]) -> Footprint {
        Footprint::ZERO
    }
}

const LUT_COLOR_GRADE: SpatialStage = stage(
    "lut_color_grade",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/composite/lut_color_grade.wgsl"
    )),
    &[ParamSource::Param(0), ParamSource::Param(1)],
    &[AuxSource::Image(0)],
);

/// Grades colour through a 3D LUT.
#[derive(Debug, Clone)]
pub struct LutColorGrade<P: FilterParam = f32> {
    /// LUT texture encoded as a 2D strip.
    pub lut: LutImage,
    /// Grade intensity multiplier.
    pub intensity: P,
}

impl<P: FilterParam> Filter for LutColorGrade<P> {
    type Kind = kind::Spatial;
    type Params = [f32; 2];
    const IMAGES: usize = 1;

    fn params(&self) -> [f32; 2] {
        [self.intensity.snapshot(), lut_size(&self.lut)]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial(Placed::new(&LUT_COLOR_GRADE));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.intensity);
    }

    fn visit_images<V: ImageVisitor>(&self, v: &mut V) {
        v.visit(0, self.lut.image());
    }
}

impl<P: FilterParam> SpatialFilter for LutColorGrade<P> {
    fn footprint_of(_params: &[f32; 2]) -> Footprint {
        Footprint::ZERO
    }
}

const SWIPE_TRANSITION_TO_IMAGE: SpatialStage = stage(
    "swipe_transition",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/composite/swipe_transition.wgsl"
    )),
    &[
        ParamSource::Param(0),
        ParamSource::Param(1),
        ParamSource::Param(2),
    ],
    &[AuxSource::Image(0)],
);

/// Reveals a target image along a direction as `progress` goes from 0 to 1.
#[derive(Debug, Clone)]
pub struct SwipeTransitionToImage<P: FilterParam = f32> {
    /// Target image revealed by the transition.
    pub target: FilterImage,
    /// Transition progress from `0.0` to `1.0`.
    pub progress: P,
    /// Feathering amount around the swipe edge.
    pub softness: P,
    /// Swipe direction.
    pub direction: TransitionDirection,
}

impl<P: FilterParam> Filter for SwipeTransitionToImage<P> {
    type Kind = kind::Spatial;
    type Params = [f32; 3];
    const IMAGES: usize = 1;

    fn params(&self) -> [f32; 3] {
        [
            self.progress.snapshot(),
            self.softness.snapshot(),
            self.direction.token(),
        ]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial(Placed::new(&SWIPE_TRANSITION_TO_IMAGE));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.progress);
        v.visit(1, &self.softness);
    }

    fn visit_images<V: ImageVisitor>(&self, v: &mut V) {
        v.visit(0, &self.target);
    }
}

impl<P: FilterParam> SpatialFilter for SwipeTransitionToImage<P> {
    fn footprint_of(_params: &[f32; 3]) -> Footprint {
        Footprint::ZERO
    }
}

const RADIAL_TRANSITION_TO_IMAGE: SpatialStage = stage(
    "radial_transition",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/composite/radial_transition.wgsl"
    )),
    &[
        ParamSource::Param(0),
        ParamSource::Param(1),
        ParamSource::Param(2),
        ParamSource::Param(3),
    ],
    &[AuxSource::Image(0)],
);

/// Reveals a target image outward from a centre point.
#[derive(Debug, Clone)]
pub struct RadialTransitionToImage<P: FilterParam = f32> {
    /// Target image revealed by the transition.
    pub target: FilterImage,
    /// Transition progress from `0.0` to `1.0`.
    pub progress: P,
    /// Feathering amount around the radial edge.
    pub softness: P,
    /// Horizontal transition centre in normalized coordinates.
    pub center_x: P,
    /// Vertical transition centre in normalized coordinates.
    pub center_y: P,
}

impl<P: FilterParam> Filter for RadialTransitionToImage<P> {
    type Kind = kind::Spatial;
    type Params = [f32; 4];
    const IMAGES: usize = 1;

    fn params(&self) -> [f32; 4] {
        [
            self.progress.snapshot(),
            self.softness.snapshot(),
            self.center_x.snapshot(),
            self.center_y.snapshot(),
        ]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial(Placed::new(&RADIAL_TRANSITION_TO_IMAGE));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.progress);
        v.visit(1, &self.softness);
        v.visit(2, &self.center_x);
        v.visit(3, &self.center_y);
    }

    fn visit_images<V: ImageVisitor>(&self, v: &mut V) {
        v.visit(0, &self.target);
    }
}

impl<P: FilterParam> SpatialFilter for RadialTransitionToImage<P> {
    fn footprint_of(_params: &[f32; 4]) -> Footprint {
        Footprint::ZERO
    }
}

const ZOOM_TRANSITION_TO_IMAGE: SpatialStage = stage(
    "zoom_transition",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/composite/zoom_transition.wgsl"
    )),
    &[
        ParamSource::Param(0),
        ParamSource::Param(1),
        ParamSource::Param(2),
        ParamSource::Param(3),
    ],
    &[AuxSource::Image(0)],
);

/// Cross-fades to a target image while zooming through a centre point.
///
/// The zoom displaces samples by `amount` times the image extent, so the
/// footprint is relative.
#[derive(Debug, Clone)]
pub struct ZoomTransitionToImage<P: FilterParam = f32> {
    /// Target image revealed by the transition.
    pub target: FilterImage,
    /// Transition progress from `0.0` to `1.0`.
    pub progress: P,
    /// Zoom magnitude applied during the transition.
    pub amount: P,
    /// Horizontal zoom centre in normalized coordinates.
    pub center_x: P,
    /// Vertical zoom centre in normalized coordinates.
    pub center_y: P,
}

impl<P: FilterParam> Filter for ZoomTransitionToImage<P> {
    type Kind = kind::Spatial;
    type Params = [f32; 4];
    const IMAGES: usize = 1;

    fn params(&self) -> [f32; 4] {
        [
            self.progress.snapshot(),
            self.amount.snapshot(),
            self.center_x.snapshot(),
            self.center_y.snapshot(),
        ]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial(Placed::new(&ZOOM_TRANSITION_TO_IMAGE));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.progress);
        v.visit(1, &self.amount);
        v.visit(2, &self.center_x);
        v.visit(3, &self.center_y);
    }

    fn visit_images<V: ImageVisitor>(&self, v: &mut V) {
        v.visit(0, &self.target);
    }
}

impl<P: FilterParam> SpatialFilter for ZoomTransitionToImage<P> {
    fn footprint_of(params: &[f32; 4]) -> Footprint {
        Footprint::extent(params[1].max(0.0))
    }
}

const DISPLACEMENT_TRANSITION_TO_IMAGE: SpatialStage = stage(
    "displacement_transition",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/shaders/composite/displacement_transition.wgsl"
    )),
    &[ParamSource::Param(0), ParamSource::Param(1)],
    &[AuxSource::Image(0), AuxSource::Image(1)],
);

/// Cross-fades to a target image while a displacement map pushes the input
/// out and pulls the target in.
#[derive(Debug, Clone)]
pub struct DisplacementTransitionToImage<P: FilterParam = f32> {
    /// Target image revealed by the transition.
    pub target: FilterImage,
    /// Displacement map (red and green mapped from [0, 1] to [-1, 1]).
    pub map: FilterImage,
    /// Transition progress from `0.0` to `1.0`.
    pub progress: P,
    /// Displacement strength, in pixels.
    pub scale: P,
}

impl<P: FilterParam> Filter for DisplacementTransitionToImage<P> {
    type Kind = kind::Spatial;
    type Params = [f32; 2];
    const IMAGES: usize = 2;

    fn params(&self) -> [f32; 2] {
        [self.progress.snapshot(), self.scale.snapshot()]
    }

    fn collect_stages<C: StageCollector>(&self, c: &mut C) {
        c.spatial(Placed::new(&DISPLACEMENT_TRANSITION_TO_IMAGE));
    }

    fn visit_signals<V: SignalVisitor>(&self, v: &mut V) {
        v.visit(0, &self.progress);
        v.visit(1, &self.scale);
    }

    fn visit_images<V: ImageVisitor>(&self, v: &mut V) {
        v.visit(0, &self.target);
        v.visit(1, &self.map);
    }
}

impl<P: FilterParam> SpatialFilter for DisplacementTransitionToImage<P> {
    fn footprint_of(params: &[f32; 2]) -> Footprint {
        Footprint::pixels(footprint::offset(params[1].max(0.0)))
    }
}

/// Shapes tonal regions of the colour as sampled: a gamma curve plus
/// shadow, midtone and highlight lifts, clamped to [0, 1] and mixed in by
/// `amount`.
#[derive(Debug, Clone, crate::Filter)]
#[filter(color, shader = "composite/tone_curve.wgsl", linear = false)]
pub struct ToneCurve<P = f32> {
    /// Shadow adjustment.
    pub shadows: P,
    /// Midtone adjustment.
    pub midtones: P,
    /// Highlight adjustment.
    pub highlights: P,
    /// Gamma adjustment.
    pub gamma: P,
    /// Overall blend amount.
    pub amount: P,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_mode_tokens_are_stable() {
        assert_eq!(BlendMode::Normal.token(), 0.0);
        assert_eq!(BlendMode::Multiply.token(), 1.0);
        assert_eq!(BlendMode::Screen.token(), 2.0);
        assert_eq!(BlendMode::Overlay.token(), 3.0);
        assert_eq!(BlendMode::Darken.token(), 4.0);
        assert_eq!(BlendMode::Lighten.token(), 5.0);
        assert_eq!(BlendMode::SoftLight.token(), 6.0);
        assert_eq!(BlendMode::HardLight.token(), 7.0);
        assert_eq!(BlendMode::Difference.token(), 8.0);
        assert_eq!(BlendMode::Exclusion.token(), 9.0);
        assert_eq!(BlendMode::ColorDodge.token(), 10.0);
        assert_eq!(BlendMode::ColorBurn.token(), 11.0);
        assert_eq!(BlendMode::Hue.token(), 12.0);
        assert_eq!(BlendMode::Saturation.token(), 13.0);
        assert_eq!(BlendMode::Color.token(), 14.0);
        assert_eq!(BlendMode::Luminosity.token(), 15.0);
    }

    #[test]
    fn transition_direction_tokens_are_stable() {
        assert_eq!(TransitionDirection::LeftToRight.token(), 0.0);
        assert_eq!(TransitionDirection::RightToLeft.token(), 1.0);
        assert_eq!(TransitionDirection::TopToBottom.token(), 2.0);
        assert_eq!(TransitionDirection::BottomToTop.token(), 3.0);
    }

    #[test]
    fn selectors_follow_the_animated_parameters() {
        let image = FilterImage::from_rgba8(1, 1, vec![0, 0, 0, 255]);
        let blend = BlendWithImage {
            image,
            amount: 0.5_f32,
            mode: BlendMode::Screen,
        };
        assert_eq!(blend.params(), [0.5, 2.0]);
        assert_eq!(blend.footprint(), Footprint::ZERO);
        let warp = DisplacementWarp {
            map: FilterImage::from_rgba8(1, 1, vec![0, 0, 0, 255]),
            scale_x: -3.5_f32,
            scale_y: 2.0,
        };
        assert_eq!(warp.footprint(), Footprint::pixels(4.0));
    }
}
