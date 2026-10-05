//! The AppKit-native realization of portable filter chains
//! (`Native<FilteredView>`) — a mounted, visible content view whose host
//! layer presents through `CALayer.filters` backed by ordered `CIFilter`s
//! on `layerUsesCoreImageFilters` (#1748).
//!
//! Selection is static and all-or-nothing per fused chain, planned before
//! `AnyEffect::build`, before any capture resource or `GpuRuntime` is
//! asked for: every effect must carry a portable description, declare no
//! output-size policy, and every stage must map to a `CIFilter` — the
//! slightest miss sends the whole chain down the existing capture path.
//! Identity is the concrete filter type: `FilterDescription::visit_links`
//! reports each `FilterLink` — the erased concrete filter, its
//! `param_base`/`image_base` offsets into the flattened arrays, in
//! application order — and `downcast_ref` on the canonical
//! `filter_view` aliases claims the link for the matching `CIFilter`.
//! No stage-text comparison survives: a custom `Filter` or an alias type
//! this leaf does not claim fails `map_link` and routes the whole chain
//! to capture.
//!
//! After attachment, inputs change only through `CALayer`
//! `setValue:forKeyPath:` — never by mutating an attached `CIFilter`'s
//! inputs, which the SDK declares undefined. Animated parameter updates
//! submit a `CAKeyframeAnimation` sampled from the change's
//! [`Interpolator`] at the attached display's cadence, interrupting from
//! the parameter's own pending timeline and preserving every unaffected
//! bound component's curve; an update while the view has no screen keeps
//! its pending timeline instead of installing a final value.
//!
//! Captures go through the owned seam: the mount's `CapturableSurface`
//! external render captures the mounted content through the existing
//! `ViewCapture` pipeline (native + resolved GPU surfaces included) and
//! then runs a dedicated `CIFilter` set — synced to the current
//! presentation and in-flight parameter timelines — into the
//! compositor's texture through an issue-owned `CIContext` with a
//! real Metal command submission; `Ok(())` is settled only after that
//! command buffer reports completion.
//!
//! #1683 prerequisite: on this branch the capturable registration and the
//! resolver that lets *outer* captures resolve a native-filtered host are
//! the `CaptureRegistry` calls documented in the implementation report —
//! this branch predates the env-installed registry, so the seam is
//! authored to its interface without duplicating it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use crate::main_queue_owned::Shared;
use cocoa_ui::capture::{CapturableSurface, CaptureDeferred, SurfaceCaptureCompletion};
use cocoa_ui::{PlatformView, Retained};
use std::ptr::NonNull;

use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AnyThread, MainThreadMarker, Message};
use objc2_core_foundation::{CFRetained, CGRect};
use objc2_core_graphics::{CGColorSpace, kCGColorSpaceExtendedLinearDisplayP3};
use objc2_core_image::{
    CIContext, CIFilter, CIImage, CIVector, kCIContextOutputColorSpace,
    kCIContextWorkingColorSpace, kCIImageColorSpace,
};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSObjectNSKeyValueCoding, NSString};
use objc2_metal::{
    MTLCommandBuffer as _, MTLCommandQueue as _, MTLDevice as _, MTLResource as _, MTLTexture as _,
};
use objc2_quartz_core::{
    CACurrentMediaTime, CAKeyframeAnimation, CALayer, CAMediaTiming, CAMediaTimingFunction,
    CATransaction, kCAAnimationLinear, kCAMediaTimingFunctionLinear,
};
use waterui_backend_core::{AnyView, Environment};
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};
use waterui_graphics::filter_view::{AnyEffect, ParamGuards};
use waterui_graphics::filtrate::{FilterLink, Interpolator, WatchGuard, WorkingSpace};

use crate::contract::{Mounted, NativeLeaf, RenderContext};

type MetalTexture = ProtocolObject<dyn objc2_metal::MTLTexture>;
type MetalDevice = ProtocolObject<dyn objc2_metal::MTLDevice>;
type MetalQueue = ProtocolObject<dyn objc2_metal::MTLCommandQueue>;
type MetalCommandBuffer = ProtocolObject<dyn objc2_metal::MTLCommandBuffer>;

/// The primaries the compositor evaluates `CALayer.filters` in — the
/// linearized gamut of the screen the layer renders on. Filtrate
/// evaluates every stage in linear Display P3, so on an sRGB screen the
/// native matrix is the conjugated `P3→sRGB·M·sRGB→P3` while on a P3
/// screen it is `M` verbatim; `Bound`s carry both resolutions, so a
/// window crossing displays resamples instead of re-planning.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Gamut {
    /// sRGB primaries, linearized — conjugated coefficients.
    Srgb = 0,
    /// Display P3 primaries — filtrate's own working space.
    DisplayP3 = 1,
}

impl Gamut {
    /// The gamut the screen `view` renders on can represent —
    /// `DisplayP3` when it covers P3, sRGB otherwise. No window means
    /// no screen sample yet: sRGB — the layout pass corrects it on
    /// attach.
    fn of_view(view: &PlatformView) -> Self {
        cocoa_ui::view::window(view)
            .and_then(|window| window.screen())
            .map_or(Self::Srgb, |screen| {
                if screen.canRepresentDisplayGamut(cocoa_ui::objc2_app_kit::NSDisplayGamut::P3) {
                    Self::DisplayP3
                } else {
                    Self::Srgb
                }
            })
    }
}

/// How a raw flat parameter value becomes a `CIFilter` input value.
#[derive(Clone, Copy)]
enum Conv {
    /// The value is passed through.
    Direct,
    /// `mul·v + add` — a matrix coefficient affine in the parameter,
    /// resolved per evaluation gamut (`gamut as usize`).
    Affine { mul: [f32; 2], add: [f32; 2] },
}

impl Conv {
    const fn map(self, value: f32, gamut: Gamut) -> f32 {
        match self {
            Self::Direct => value,
            Self::Affine { mul, add } => value.mul_add(mul[gamut as usize], add[gamut as usize]),
        }
    }
}

/// A 3×3 matrix over a linear RGB space, row-major.
type Mat3 = [[f32; 3]; 3];

/// Row-major matrix product `a·b`.
fn mat3_mul(a: Mat3, b: Mat3) -> Mat3 {
    std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            a[i][2].mul_add(b[2][j], a[i][1].mul_add(b[1][j], a[i][0] * b[0][j]))
        })
    })
}

/// Row-major `m·v`.
fn mat3_mul_vec(m: Mat3, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| m[i][2].mul_add(v[2], m[i][1].mul_add(v[1], m[i][0] * v[0])))
}

/// Component-wise `a - b`.
fn mat3_sub(a: Mat3, b: Mat3) -> Mat3 {
    std::array::from_fn(|i| std::array::from_fn(|j| a[i][j] - b[i][j]))
}

/// Component-wise `a + b`.
fn mat3_add(a: Mat3, b: Mat3) -> Mat3 {
    std::array::from_fn(|i| std::array::from_fn(|j| a[i][j] + b[i][j]))
}

/// Component-wise `k·m`.
fn mat3_scale(m: Mat3, k: f32) -> Mat3 {
    std::array::from_fn(|i| std::array::from_fn(|j| m[i][j] * k))
}

/// The linear sRGB → linear Display P3 transform (D65), row-major —
/// `graphics/filtrate/src/shaders/space/from_srgb.wgsl`'s `SRGB_TO_P3`.
const SRGB_TO_P3: Mat3 = [
    [0.822_462, 0.177_538, 0.0],
    [0.033_194_2, 0.966_805_8, 0.0],
    [0.017_082_6, 0.072_397_4, 0.910_519_9],
];

/// The linear Display P3 → linear sRGB transform (D65), row-major —
/// `graphics/filtrate/src/shaders/space/to_srgb.wgsl`'s `P3_TO_SRGB`.
const P3_TO_SRGB: Mat3 = [
    [1.224_940_2, -0.224_940_2, 0.0],
    [-0.042_057, 1.042_057, 0.0],
    [-0.019_637_6, -0.078_636, 1.098_273_6],
];

/// `P3→sRGB · m · sRGB→P3` — `m` evaluated in the sRGB gamut's linear
/// space.
fn conjugate(m: Mat3) -> Mat3 {
    mat3_mul(P3_TO_SRGB, mat3_mul(m, SRGB_TO_P3))
}

const MAT_IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
const MAT_ZERO: Mat3 = [[0.0; 3]; 3];
const ZERO3: [f32; 3] = [0.0; 3];
const ONE3: [f32; 3] = [1.0; 3];

/// The working space's luma row — `WorkingSpace::LINEAR_DISPLAY_P3.luma`.
const LUMA_P3: [f32; 3] = WorkingSpace::LINEAR_DISPLAY_P3.luma;

/// Every row the working-space luma — the replication matrix.
const LUMA_MAT: Mat3 = [LUMA_P3; 3];

/// Filtrate's sepia matrix `S` — `color/transform/sepia.wgsl`.
const SEPIA_MAT: Mat3 = [
    [0.393, 0.769, 0.189],
    [0.349, 0.686, 0.168],
    [0.272, 0.534, 0.131],
];

/// A `CIFilter` input bound to a flat parameter of the owning effect, a
/// weighted sum of flat parameters, or a constant.
#[derive(Clone)]
enum Bound {
    /// Flat parameter index inside the owning effect's `params()`.
    Param(usize, Conv),
    /// A constant per evaluation gamut — identity slots on shared
    /// `CIFilter`s carry identical resolutions.
    Const([f32; 2]),
    /// `add` plus a weighted sum of flat parameters — a coefficient
    /// mixing several parameters, as the conjugated user colour
    /// matrix's are. Each referenced parameter keeps its own timeline;
    /// a resample evaluates every term at one timestamp, the GPU path's
    /// own semantics.
    Combo(ComboBound),
}

/// The term list of a [`Bound::Combo`] per evaluation gamut.
#[derive(Clone)]
struct ComboBound {
    /// The constant term per gamut.
    add: [f32; 2],
    /// `(flat index, weight)` under [`Gamut::Srgb`].
    srgb: Box<[(usize, f32)]>,
    /// `(flat index, weight)` under [`Gamut::DisplayP3`].
    p3: Box<[(usize, f32)]>,
}

impl ComboBound {
    fn terms(&self, gamut: Gamut) -> &[(usize, f32)] {
        match gamut {
            Gamut::Srgb => &self.srgb,
            Gamut::DisplayP3 => &self.p3,
        }
    }

    /// `add` plus the weighted sum over `param`.
    fn value(&self, param: impl Fn(usize) -> f32, gamut: Gamut) -> f32 {
        self.terms(gamut)
            .iter()
            .fold(self.add[gamut as usize], |sum, (index, weight)| {
                sum + weight * param(*index)
            })
    }
}

impl Bound {
    /// Whether the bound reads the flat parameter `index`.
    fn binds(&self, index: usize) -> bool {
        match self {
            Self::Param(flat, _) => *flat == index,
            Self::Const(_) => false,
            Self::Combo(combo) => combo
                .srgb
                .iter()
                .chain(combo.p3.iter())
                .any(|(flat, _)| *flat == index),
        }
    }

    /// The flat parameter indices the bound reads.
    fn params(&self) -> impl Iterator<Item = usize> + '_ {
        let (one, combo) = match self {
            Self::Param(flat, _) => (Some(*flat), None),
            Self::Const(_) => (None, None),
            Self::Combo(combo) => (None, Some(combo)),
        };
        one.into_iter().chain(combo.into_iter().flat_map(|combo| {
            combo
                .srgb
                .iter()
                .chain(combo.p3.iter())
                .map(|(flat, _)| *flat)
        }))
    }

    /// The current value from `model`.
    fn value(&self, model: &[f32], gamut: Gamut) -> f32 {
        match self {
            Self::Param(index, conv) => conv.map(model[*index], gamut),
            Self::Const(value) => value[gamut as usize],
            Self::Combo(combo) => combo.value(|index| model[index], gamut),
        }
    }

    /// The bound's value at `now` — each parameter's pending timeline
    /// evaluated when one exists, its model value otherwise.
    fn value_at(
        &self,
        animations: &HashMap<AnimKey, ComponentAnim>,
        effect: usize,
        model: &[f32],
        now: f64,
        gamut: Gamut,
    ) -> f32 {
        let param_at = |index: usize| {
            animations
                .get(&(effect, index))
                .map_or(model[index], |anim| anim.raw_at(now))
        };
        match self {
            Self::Param(index, conv) => conv.map(param_at(*index), gamut),
            Self::Const(value) => value[gamut as usize],
            Self::Combo(combo) => combo.value(param_at, gamut),
        }
    }
}

/// A `CIFilter` input value.
#[derive(Clone)]
enum BoundValue {
    /// An `NSNumber` input.
    Scalar(Bound),
    /// A `CIVector` input of four components.
    Vec4([Bound; 4]),
}

impl BoundValue {
    /// Whether this input reads the flat parameter `index`.
    fn binds(&self, index: usize) -> bool {
        match self {
            Self::Scalar(bound) => bound.binds(index),
            Self::Vec4(components) => components.iter().any(|bound| bound.binds(index)),
        }
    }

    /// The flat parameter indices this input reads.
    fn params(&self) -> impl Iterator<Item = usize> + '_ {
        let (one, components) = match self {
            Self::Scalar(bound) => (Some(bound), None),
            Self::Vec4(components) => (None, Some(components.as_slice())),
        };
        one.into_iter().flat_map(Bound::params).chain(
            components
                .into_iter()
                .flat_map(|c| c.iter().flat_map(Bound::params)),
        )
    }
}

/// One mapped `CIFilter` — class name plus its input bindings.
struct NativeStage {
    class: &'static str,
    inputs: Vec<(&'static str, BoundValue)>,
}

const fn param(index: usize, conv: Conv) -> BoundValue {
    BoundValue::Scalar(Bound::Param(index, conv))
}

/// `CALayer.filters` evaluates `CIColorMatrix` on unpremultiplied
/// pixels — `out = rows·(r,g,b,a) + bias`, then repremultiplies by the
/// input alpha — so a filtrate premultiplied additive term `u·a` rides
/// the bias vector (`bias·a = u·a`), not the alpha column, and the
/// alpha column stays zero on every matrix stage.
const ALPHA_IDENTITY: [Bound; 4] = [
    Bound::Const([0.0; 2]),
    Bound::Const([0.0; 2]),
    Bound::Const([0.0; 2]),
    Bound::Const([1.0; 2]),
];
const ZERO4: Bound = Bound::Const([0.0; 2]);

/// A `CIColorMatrix` stage from the three `R`/`G`/`B` row vectors plus
/// the bias vector carrying filtrate's alpha-scaled additive term.
fn matrix_stage(r: BoundValue, g: BoundValue, b: BoundValue, bias: BoundValue) -> NativeStage {
    NativeStage {
        class: "CIColorMatrix",
        inputs: vec![
            ("inputRVector", r),
            ("inputGVector", g),
            ("inputBVector", b),
            ("inputAVector", BoundValue::Vec4(ALPHA_IDENTITY)),
            ("inputBiasVector", bias),
        ],
    }
}

/// `m0 + p·m1` on premultiplied RGB plus the additive `u0 + p·u1`
/// through the bias vector — one `CIColorMatrix`. Every coefficient is
/// affine in the parameter, so the key-path vectors interpolate the
/// filtrate parameter timeline exactly.
fn affine_matrix_stage(
    param: usize,
    m0: Mat3,
    m1: Mat3,
    u0: [f32; 3],
    u1: [f32; 3],
) -> NativeStage {
    // `gamut as usize`: sRGB takes the conjugated matrix, P3 the
    // verbatim working-space one.
    let c0 = [conjugate(m0), m0];
    let c1 = [conjugate(m1), m1];
    let a0 = [mat3_mul_vec(P3_TO_SRGB, u0), u0];
    let a1 = [mat3_mul_vec(P3_TO_SRGB, u1), u1];
    let component = |i: usize, j: usize| {
        Bound::Param(
            param,
            Conv::Affine {
                mul: [c1[0][i][j], c1[1][i][j]],
                add: [c0[0][i][j], c0[1][i][j]],
            },
        )
    };
    let row =
        |i: usize| BoundValue::Vec4([component(i, 0), component(i, 1), component(i, 2), ZERO4]);
    let bias = |i: usize| {
        Bound::Param(
            param,
            Conv::Affine {
                mul: [a1[0][i], a1[1][i]],
                add: [a0[0][i], a0[1][i]],
            },
        )
    };
    matrix_stage(
        row(0),
        row(1),
        row(2),
        BoundValue::Vec4([bias(0), bias(1), bias(2), ZERO4]),
    )
}

/// A constant `CIColorMatrix` — `m` on premultiplied RGB, `u` through
/// the bias vector.
fn const_matrix_stage(m: Mat3, u: [f32; 3]) -> NativeStage {
    let c = conjugate(m);
    let a = mat3_mul_vec(P3_TO_SRGB, u);
    let row = |i: usize| {
        BoundValue::Vec4([
            Bound::Const([c[i][0], m[i][0]]),
            Bound::Const([c[i][1], m[i][1]]),
            Bound::Const([c[i][2], m[i][2]]),
            ZERO4,
        ])
    };
    let bias = |i: usize| Bound::Const([a[i], u[i]]);
    matrix_stage(
        row(0),
        row(1),
        row(2),
        BoundValue::Vec4([bias(0), bias(1), bias(2), ZERO4]),
    )
}

/// The approved per-type native realizations. Each builder takes the
/// concrete link's flat parameter offset inside the owning effect and
/// emits the `CIFilter` class plus its input bindings. No stage-text
/// identity matching survives: the concrete `FilterLink` type is the
/// identity.
///
/// `Brightness` — `rgb + amount·a`: identity RGB, `amount` through the
/// bias vector.
fn brightness(base: usize) -> NativeStage {
    affine_matrix_stage(base, MAT_IDENTITY, MAT_ZERO, ZERO3, ONE3)
}

/// `Contrast` — `(rgb − 0.5a)·k + 0.5a`: `k·I` plus `0.5(1−k)`
/// through the bias vector.
fn contrast(base: usize) -> NativeStage {
    affine_matrix_stage(base, MAT_ZERO, MAT_IDENTITY, [0.5; 3], [-0.5; 3])
}

/// `Saturation` — `s·rgb + (1−s)·luma·1` with the working-space luma.
fn saturation(base: usize) -> NativeStage {
    affine_matrix_stage(
        base,
        LUMA_MAT,
        mat3_sub(MAT_IDENTITY, LUMA_MAT),
        ZERO3,
        ZERO3,
    )
}

/// `Grayscale` — `(1−i)·rgb + i·luma·1`.
fn grayscale(base: usize) -> NativeStage {
    affine_matrix_stage(
        base,
        MAT_IDENTITY,
        mat3_sub(LUMA_MAT, MAT_IDENTITY),
        ZERO3,
        ZERO3,
    )
}

/// `Sepia` — `(1−i)·rgb + i·S·rgb`, `S` filtrate's sepia matrix.
fn sepia(base: usize) -> NativeStage {
    affine_matrix_stage(
        base,
        MAT_IDENTITY,
        mat3_sub(SEPIA_MAT, MAT_IDENTITY),
        ZERO3,
        ZERO3,
    )
}

/// `Invert` — `a − rgb`.
fn invert() -> NativeStage {
    const_matrix_stage([[-1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]], ONE3)
}

/// `ColorMatrix` — the user's 3×4 rows: the 3×3 block conjugates as a
/// matrix; the fourth column multiplies alpha, and under the
/// unpremultiplied evaluation that additive `m·a` term rides the bias
/// vector, not the alpha column.
fn color_matrix(base: usize) -> NativeStage {
    let row = |i: usize| {
        BoundValue::Vec4(std::array::from_fn(|j| {
            if j < 3 {
                let srgb = (0..3)
                    .flat_map(|k| {
                        (0..3).map(move |l| (base + k * 4 + l, P3_TO_SRGB[i][k] * SRGB_TO_P3[l][j]))
                    })
                    .collect();
                Bound::Combo(ComboBound {
                    add: [0.0; 2],
                    srgb,
                    p3: Box::new([(base + i * 4 + j, 1.0)]),
                })
            } else {
                ZERO4
            }
        }))
    };
    let bias = |i: usize| {
        let srgb = (0..3)
            .map(|k| (base + k * 4 + 3, P3_TO_SRGB[i][k]))
            .collect();
        Bound::Combo(ComboBound {
            add: [0.0; 2],
            srgb,
            p3: Box::new([(base + i * 4 + 3, 1.0)]),
        })
    };
    matrix_stage(
        row(0),
        row(1),
        row(2),
        BoundValue::Vec4([bias(0), bias(1), bias(2), ZERO4]),
    )
}

/// `Exposure` → `CIExposureAdjust.inputEV` — `rgb·2^ev` commutes with
/// premultiplication.
fn exposure(base: usize) -> NativeStage {
    NativeStage {
        class: "CIExposureAdjust",
        inputs: vec![("inputEV", param(base, Conv::Direct))],
    }
}

/// `PhotoEffectMono` — `luma·1` on the working-space luma, a constant
/// replication matrix.
fn photo_effect_mono() -> NativeStage {
    const_matrix_stage(LUMA_MAT, ZERO3)
}

/// `PhotoEffectNoir` — `clamp((luma−0.5)·1.6+0.5, 0, F16_MAX)·1` on
/// straight-alpha colour: `1.6·L` rows with `−0.3` through the bias
/// vector. Filtrate's own lower clamp binds only where the affine map
/// leaves the display gamut; the compositor's own output saturation
/// produces the same pixels.
fn photo_effect_noir() -> NativeStage {
    const_matrix_stage(mat3_scale(LUMA_MAT, 1.6), [-0.3; 3])
}

/// `PhotoEffectChrome` — `D·(1.45·s − 0.45·luma)` on straight-alpha
/// colour with `D = diag(1.05, 1.0, 0.95)`.
fn photo_effect_chrome() -> NativeStage {
    let boosted = mat3_sub(mat3_scale(MAT_IDENTITY, 1.45), mat3_scale(LUMA_MAT, 0.45));
    const_matrix_stage(
        mat3_mul(
            [[1.05, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 0.95]],
            boosted,
        ),
        ZERO3,
    )
}

/// `PhotoEffectInstant` — `0.3·luma(warmed)·1 + 0.7·warmed` with
/// `warmed = D·s + b`, `D = diag(1.10, 1.02, 0.85)`, `b = (0.05, 0.04, 0)`
/// on straight-alpha colour.
fn photo_effect_instant() -> NativeStage {
    let mix = mat3_add(mat3_scale(LUMA_MAT, 0.3), mat3_scale(MAT_IDENTITY, 0.7));
    let warmed = [[1.10, 0.0, 0.0], [0.0, 1.02, 0.0], [0.0, 0.0, 0.85]];
    const_matrix_stage(mat3_mul(mix, warmed), mat3_mul_vec(mix, [0.05, 0.04, 0.0]))
}

/// `PhotoEffectFade` — `0.25·luma(lifted)·1 + 0.75·lifted` with
/// `lifted = 0.85·s + 0.015·1` on straight-alpha colour.
fn photo_effect_fade() -> NativeStage {
    let mix = mat3_add(mat3_scale(LUMA_MAT, 0.25), mat3_scale(MAT_IDENTITY, 0.75));
    const_matrix_stage(mat3_scale(mix, 0.85), [0.015; 3])
}

/// `PhotoEffectTonal` — `0.6·luma·1 + 0.4·rgb` on the premultiplied
/// working space.
fn photo_effect_tonal() -> NativeStage {
    const_matrix_stage(
        mat3_add(mat3_scale(LUMA_MAT, 0.6), mat3_scale(MAT_IDENTITY, 0.4)),
        ZERO3,
    )
}

/// `PhotoEffectTransfer` — `0.45·s + 0.55·(1.07, 0.95, 0.78)·luma` on
/// straight-alpha colour — `0.45·I` plus `0.55·c⊗L`.
fn photo_effect_transfer() -> NativeStage {
    let warm: Mat3 =
        std::array::from_fn(|i| std::array::from_fn(|j| 0.55 * [1.07, 0.95, 0.78][i] * LUMA_P3[j]));
    const_matrix_stage(mat3_add(mat3_scale(MAT_IDENTITY, 0.45), warm), ZERO3)
}

/// The static native plan for one effect: the ordered `CIFilter`s plus
/// the flat parameter model the bindings read, and `source` — the
/// effect's index in `chained` — so model, signals and filter instances
/// share one identity.
pub struct EffectPlan {
    source: usize,
    stages: Vec<NativeStage>,
    params: Vec<f32>,
}

/// Maps one concrete `FilterLink` — the link's own type IS the identity;
/// `param_base` places its parameters in the owning effect's flattened
/// array. The map covers every parameter type the public filter API
/// accepts — the static `f32` instantiation and the reactive
/// `filter_view` alias — and only filters whose filtrate function a
/// `CIFilter` set computes identically: the affine colour ops through
/// one `CIColorMatrix` each, plus `CIExposureAdjust`, whose scalar
/// input is the parameter itself with identical math. The photo
/// effects whose filtrate WGSL is affine — `Mono`, `Noir`, `Chrome`,
/// `Instant`, `Fade`, `Tonal`, `Transfer` — realize through one
/// `CIColorMatrix` each; `PhotoEffectProcess`'s `min()` is not affine.
/// `Gamma` — filtrate unpremultiplies, `CIGammaAdjust` does not —
/// `HueRotation` — `CIHueAdjust` is a different matrix, and the angle
/// is not affine — the blur family —
/// `CIGaussianBlur`/`CIBoxBlur`/`CIMotionBlur` use different kernels,
/// radius conventions, and edge handling — Apple's `CIPhotoEffect*`
/// presets — tone curves, not filtrate's functions — and everything
/// else (`ZoomBlur`, `Vibrance`, the compound
/// `Bloom`/`Gloom`/`UnsharpMask`, `TemperatureTint`, `WhitePoint`,
/// `HighlightsShadows`, `Vignette`, `Pixellate`, `Median`,
/// `Convolution*`, halftone variants, distortions, every custom
/// `Filter`) realize through capture.
fn map_link(link: &FilterLink<'_>) -> Option<NativeStage> {
    use waterui_graphics::filter_view as fv;
    use waterui_graphics::filtrate::filters;
    let base = link.param_base;
    if link.downcast_ref::<fv::Brightness>().is_some()
        || link.downcast_ref::<filters::Brightness<f32>>().is_some()
    {
        return Some(brightness(base));
    }
    if link.downcast_ref::<fv::Contrast>().is_some()
        || link.downcast_ref::<filters::Contrast<f32>>().is_some()
    {
        return Some(contrast(base));
    }
    if link.downcast_ref::<fv::Saturation>().is_some()
        || link.downcast_ref::<filters::Saturation<f32>>().is_some()
    {
        return Some(saturation(base));
    }
    if link.downcast_ref::<fv::Grayscale>().is_some()
        || link.downcast_ref::<filters::Grayscale<f32>>().is_some()
    {
        return Some(grayscale(base));
    }
    if link.downcast_ref::<fv::Exposure>().is_some()
        || link.downcast_ref::<filters::Exposure<f32>>().is_some()
    {
        return Some(exposure(base));
    }
    if link.downcast_ref::<fv::Sepia>().is_some()
        || link.downcast_ref::<filters::Sepia<f32>>().is_some()
    {
        return Some(sepia(base));
    }
    if link.downcast_ref::<fv::Invert>().is_some() {
        return Some(invert());
    }
    if link.downcast_ref::<fv::ColorMatrix>().is_some()
        || link
            .downcast_ref::<filters::ColorMatrix<fv::Reactive>>()
            .is_some()
    {
        return Some(color_matrix(base));
    }
    // The photo effects below are filtrate's own affine WGSL — constant
    // matrices, not Apple's `CIPhotoEffect*` presets.
    if link.downcast_ref::<filters::PhotoEffectMono>().is_some() {
        return Some(photo_effect_mono());
    }
    if link.downcast_ref::<filters::PhotoEffectNoir>().is_some() {
        return Some(photo_effect_noir());
    }
    if link.downcast_ref::<filters::PhotoEffectChrome>().is_some() {
        return Some(photo_effect_chrome());
    }
    if link.downcast_ref::<filters::PhotoEffectInstant>().is_some() {
        return Some(photo_effect_instant());
    }
    if link.downcast_ref::<filters::PhotoEffectFade>().is_some() {
        return Some(photo_effect_fade());
    }
    if link.downcast_ref::<filters::PhotoEffectTonal>().is_some() {
        return Some(photo_effect_tonal());
    }
    if link
        .downcast_ref::<filters::PhotoEffectTransfer>()
        .is_some()
    {
        return Some(photo_effect_transfer());
    }
    None
}

/// Attempts the static native plan for the fused chain — all-or-nothing.
/// Effects arrive outer-to-content; `CALayer.filters` applies
/// content-adjacent first, so the plan walks `chained` in reverse and
/// carries each effect's `source` index back. A link's parameter
/// bindings address `description.params()` through `param_base`, and its
/// reactive parameters address the same flat indices through
/// `visit_signals`.
#[must_use]
pub fn plan(chained: &[(AnyEffect, ParamGuards)]) -> Option<Vec<EffectPlan>> {
    let mut effects = Vec::with_capacity(chained.len());
    for (source, (effect, _)) in chained.iter().enumerate().rev() {
        // Any declared output-size policy is unmapped — including one that
        // could later change, since choosing by its current value would be
        // a runtime realization switch.
        if effect.output_size().is_some() {
            return None;
        }
        // An arbitrary GPU effect has no portable description.
        let description = effect.description()?;
        let mut stages = Vec::new();
        let mut unmapped = false;
        description.visit_links(|link| match map_link(&link) {
            Some(stage) => stages.push(stage),
            None => unmapped = true,
        });
        if unmapped {
            return None;
        }
        if stages.is_empty() {
            // A filter with no stages is a no-op realization; keep it on
            // the capture path rather than owning an empty native chain.
            return None;
        }
        effects.push(EffectPlan {
            source,
            stages,
            params: description.params(),
        });
    }
    Some(effects)
}

/// One bound `CIFilter`'s live state — the filter, its key-path root and
/// the inputs rebuilt per parameter update.
struct BoundFilter {
    filter: Retained<CIFilter>,
    /// The registered `CIFilter` class — used to instantiate fresh
    /// per-frame capture filters.
    class: &'static str,
    /// Its name in `layer.filters` — the `setValue:forKeyPath:` root.
    name: String,
    inputs: Vec<(&'static str, BoundValue)>,
    /// Index into `model` — the owning effect in plan order.
    effect: usize,
}

/// One parameter's pending timeline — the single source of truth for
/// its animation state, on or off screen, live until it settles or the
/// next update replaces it. Materializing submits it to CA without
/// discarding it: `applied` records that submission so a repeated layout
/// pass never restarts the on-screen tween and an interrupting update
/// still reads the raw curve.
struct ComponentAnim {
    /// The raw (pre-`Conv`) value the tween started from.
    from: f32,
    /// The raw value it heads to.
    to: f32,
    /// The curve the change arrived with — installed once per parameter
    /// and evaluated by every bound input.
    interpolator: Box<dyn Interpolator>,
    /// The `CACurrentMediaTime` the update arrived at — the timeline's
    /// epoch on the shared CA media clock, the same axis the
    /// #1683 `PresentationTime` anchor maps; at integration the
    /// evaluation timestamp becomes `PresentationTime::capture_time`.
    started: f64,
    /// Whether the current timeline has been sampled into a live CA
    /// animation; `false` is pending materialization.
    applied: bool,
}

impl ComponentAnim {
    /// The raw interpolated value at `now`, before `Conv`.
    fn raw_at(&self, now: f64) -> f32 {
        self.interpolator.interpolate(
            self.from,
            self.to,
            Duration::from_secs_f64((now - self.started).max(0.0)),
        )
    }

    /// Whether the timeline has settled — its remaining value is `to`.
    fn is_settled(&self, now: f64) -> bool {
        self.interpolator
            .is_complete(Duration::from_secs_f64((now - self.started).max(0.0)))
    }

    /// The time still on the curve at `now`.
    fn remaining(&self, now: f64) -> Duration {
        self.interpolator
            .duration()
            .saturating_sub(Duration::from_secs_f64((now - self.started).max(0.0)))
    }
}

/// The animation key — the owning effect and flat parameter index. A
/// parameter has one timeline, however many inputs it feeds; bound
/// inputs evaluate the parameter's timeline at the frame's timestamp.
type AnimKey = (usize, usize);

/// A context-owned Core Image context and command queue — created on the
/// exact `SharedGpuContext` issue that produced the compositor's
/// textures and rebuilt whenever a different context instance issues a
/// frame. The `Arc` is the identity: a generation number or a device
/// pointer alone cannot tell two contexts of the same generation apart.
struct CaptureGpu {
    context: Arc<waterui_graphics::gpu::SharedGpuContext>,
    ci_context: Retained<CIContext>,
    queue: Retained<MetalQueue>,
}

/// The immutable issue-time snapshot of one external capture — the
/// exact resources, GPU context and parameter/filter/image state the
/// preparation produced. In-flight work reads this, never the owner's
/// mutable slots, so an overlapping prepare or parameter update cannot
/// rebind an old frame onto new state.
struct Prepared {
    /// The compositor-owned destination.
    target: Retained<MetalTexture>,
    /// The private texture the content captures into.
    input: Retained<MetalTexture>,
    /// The exact `SharedGpuContext` that issued this frame's textures —
    /// its device backs the `CIContext` and command queue, and its `Arc`
    /// identity is what staleness checks compare.
    context: std::sync::Arc<waterui_graphics::gpu::SharedGpuContext>,
    /// Fresh, unattached `CIFilter` instances with inputs already synced
    /// to the issue timestamp — presentation values plus in-flight
    /// timelines.
    filters: Vec<Retained<CIFilter>>,
    width: u32,
    height: u32,
}

/// Everything a capture completion needs after the content capture's
/// callback crosses back to the main thread — wrapped in
/// `MainQueueOwned` so the hop is the sanctioned mechanism, a last drop
/// off-main cannot run the synchronous `MainThreadBound` teardown, and
/// no manual `Send` assertion is needed.
struct CaptureWork {
    owner: Weak<NativeFilterOwner>,
    prepared: Prepared,
    /// Taken exactly once — the capture callback and the command-buffer
    /// handler can both arrive; only the first settles.
    completion: RefCell<Option<SurfaceCaptureCompletion>>,
}

/// The mount-time owner: the host view, presentation layer, attached
/// screen filters, the capture-side filter instances, the parameter
/// model and the signal watchers. Native callbacks hold it `Weak`
/// through `MainQueueOwned`; teardown clears `layer.filters` and drops
/// the watchers — no cycles.
struct NativeFilterOwner {
    view: Retained<PlatformView>,
    layer: Retained<CALayer>,
    env: Environment,
    /// The filters attached to `layer` — their inputs are only ever
    /// written through `setValue:forKeyPath:`.
    filters: Vec<BoundFilter>,
    /// A second, unattached `CIFilter` set for the capture seam — the
    /// SDK forbids mutating an attached filter's inputs, so the capture
    /// renders through its own instances synced from the presentation
    /// values and in-flight timelines each frame.
    capture_filters: Vec<BoundFilter>,
    /// The owned-content capture pipeline — the same `ViewCapture` the
    /// filtered leaf uses, resolving GPU children alike.
    capture: Rc<cocoa_ui::capture::ViewCapture>,
    /// The per-effect flat parameter model — raw values, before `Conv`.
    model: RefCell<Vec<Vec<f32>>>,
    /// Per-component pending timelines — evaluated for capture and
    /// materialized to CA on attach; never a source of silent jumps.
    animations: RefCell<HashMap<AnimKey, ComponentAnim>>,
    /// The evaluation gamut the screen bindings resolve under — the
    /// display's; the capture seam always evaluates under `DisplayP3`
    /// (its context is pinned to extended-linear-P3).
    gamut: Cell<Gamut>,
    /// The issued-context-owned capture context — rebuilt when a
    /// different `SharedGpuContext` instance issues a frame.
    capture_gpu: RefCell<Option<CaptureGpu>>,
    /// The immutable snapshot of the active external render — one
    /// prepare/render pair at a time.
    external_prepared: RefCell<Option<Prepared>>,
    /// Nested native-pass suppression scopes.
    capture_suppression: Cell<usize>,
    /// The `hidden` state the outermost suppression found — restored on
    /// release instead of unconditionally unhiding.
    suppressed_hidden: Cell<bool>,
    /// External-render scopes — redirecting redraw to the capture hook.
    external_count: Cell<usize>,
    /// While external, parameter updates notify this capture hook.
    external_redraw: RefCell<Option<Rc<dyn Fn()>>>,
    /// Subscription guards — dropping them cancels the watches.
    guards: RefCell<Vec<WatchGuard>>,
}

impl std::fmt::Debug for NativeFilterOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeFilterOwner")
            .field("filters", &self.filters.len())
            .finish_non_exhaustive()
    }
}

impl Drop for NativeFilterOwner {
    /// Clears the layer's filters and shuts the capture down; the
    /// watchers cancel with the guards.
    fn drop(&mut self) {
        // SAFETY: `filters` accepts nil to clear the list; main-thread use
        // by the leaf contract.
        unsafe { self.layer.setFilters(None) };
        self.capture.shutdown();
    }
}

/// A `CIVector` of four `f32` components.
fn ci_vec4(value: [f32; 4]) -> Retained<CIVector> {
    // SAFETY: `initWithValues:count:` reads `count` doubles; the buffer
    // outlives the call.
    unsafe {
        CIVector::initWithValues_count(
            CIVector::alloc(),
            std::ptr::NonNull::new(
                [
                    f64::from(value[0]),
                    f64::from(value[1]),
                    f64::from(value[2]),
                    f64::from(value[3]),
                ]
                .as_ptr()
                .cast_mut(),
            )
            .expect("a stack buffer pointer is never null"),
            4,
        )
    }
}

/// Runs `body` inside a `CATransaction` with implicit actions disabled —
/// every model write through `setValue:forKeyPath:` then lands
/// immediately instead of picking up CA's default tween on top of the
/// explicit keyframe stream.
fn with_implicit_actions_disabled(body: impl FnOnce()) {
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    body();
    CATransaction::commit();
}

/// Writes one scalar input through the layer key path — the only legal
/// post-attach mutation route — with implicit actions disabled.
fn set_scalar(layer: &CALayer, key_path: &str, value: f32) {
    let path = NSString::from_str(key_path);
    let number = NSNumber::new_f32(value);
    // SAFETY: `filters.<name>.<input>` accepts an `NSNumber` for scalar
    // filter inputs; the object is the correct KVC type.
    with_implicit_actions_disabled(|| unsafe {
        layer.setValue_forKeyPath(
            Some(&*(std::ptr::from_ref::<NSNumber>(&number)).cast::<AnyObject>()),
            &path,
        );
    });
}

/// Writes one `CIVector` input through the layer key path, implicit
/// actions disabled.
fn set_vec4(layer: &CALayer, key_path: &str, value: [f32; 4]) {
    let path = NSString::from_str(key_path);
    let vector = ci_vec4(value);
    // SAFETY: `filters.<name>.<input>` accepts a `CIVector` for vector
    // filter inputs.
    with_implicit_actions_disabled(|| unsafe {
        layer.setValue_forKeyPath(
            Some(&*(std::ptr::from_ref::<CIVector>(&vector)).cast::<AnyObject>()),
            &path,
        );
    });
}

/// Writes one bound input through the layer key path from `model`,
/// resolved under `gamut`.
fn set_input(layer: &CALayer, key_path: &str, value: &BoundValue, model: &[f32], gamut: Gamut) {
    match value {
        BoundValue::Scalar(bound) => set_scalar(layer, key_path, bound.value(model, gamut)),
        BoundValue::Vec4(components) => set_vec4(
            layer,
            key_path,
            components
                .each_ref()
                .map(|component| component.value(model, gamut)),
        ),
    }
}

/// The `filters.<name>.<key>` key path for one input.
fn key_path(filter: &BoundFilter, key: &str) -> String {
    format!("filters.{}.{}", filter.name, key)
}

/// Removes the explicit `CAKeyframeAnimation` on `path` — an immediate
/// update's presentation lands on the written value instead of finishing
/// a stale submitted stream.
fn remove_animation(layer: &CALayer, key_path: &str) {
    let path = NSString::from_str(key_path);
    // `removeAnimationForKey:` accepts an absent key.
    layer.removeAnimationForKey(&path);
}

/// The scalar `filters.<name>.<key>` currently presents — the
/// presentation layer's value when it exists, so an interrupting
/// animation starts from what is on screen, never a stale model value.
fn presentation_scalar(layer: &CALayer, key_path: &str, model_value: f32) -> f32 {
    let path = NSString::from_str(key_path);
    // SAFETY: `presentationLayer` is a main-thread read on a live layer.
    let Some(presentation) = (unsafe { layer.presentationLayer() }) else {
        return model_value;
    };
    presentation
        .valueForKeyPath(&path)
        .and_then(|object| object.downcast::<NSNumber>().ok())
        .map_or(model_value, |number| number.floatValue())
}

/// The display cadence `view`'s attached screen reports — `None` when
/// the view is not in a window, in which case there is nothing to pace
/// a tween for.
fn display_fps(view: &PlatformView) -> Option<f32> {
    cocoa_ui::view::window(view)
        .and_then(|window| window.screen())
        .map(|screen| {
            f32::from(u16::try_from(screen.maximumFramesPerSecond().max(1)).unwrap_or(u16::MAX))
        })
}

/// Writes one bound input through the layer key path with every
/// component evaluated at `at` under `gamut` — the resting value CA
/// presents where no submitted stream is running, and never a snap to
/// the parameter's end model while a sibling component's own timeline
/// is still moving.
#[allow(clippy::too_many_arguments)]
fn write_bound_input_at(
    layer: &CALayer,
    key_path: &str,
    bound: &BoundValue,
    animations: &HashMap<AnimKey, ComponentAnim>,
    effect: usize,
    model: &[f32],
    at: f64,
    gamut: Gamut,
) {
    match bound {
        BoundValue::Scalar(bound) => set_scalar(
            layer,
            key_path,
            bound.value_at(animations, effect, model, at, gamut),
        ),
        BoundValue::Vec4(components) => set_vec4(
            layer,
            key_path,
            components
                .each_ref()
                .map(|component| component.value_at(animations, effect, model, at, gamut)),
        ),
    }
}

/// Re-submits one bound input's CA stream so every bound parameter's own
/// raw timeline stays authoritative: drops settled timelines, then —
/// when any live curve remains — samples the whole input at `now` plus
/// each display frame for the longest remaining duration, writes the
/// resting value at `now + longest`, and marks every included timeline
/// applied only when the submit actually happens. A parameter with no
/// timeline contributes its constant current value through the
/// resubmitted siblings' stream. With no live curves the explicit stream
/// is removed and the current value written immediately.
///
/// `applied` is per-parameter — the current mapping binds each parameter
/// to a single input; if that ever widens to several inputs, this flag
/// must become per-(parameter, input) rather than redesigned now.
///
/// `force` re-samples even when every live timeline is already applied —
/// the update path requires it: an immediate or replaced timeline must
/// leave the live stream, and only a fresh submit excises the stale
/// curve. Materialization passes `false` so repeated layout never
/// restarts a stream already covering its timelines.
#[allow(clippy::too_many_arguments)]
fn resample_bound_stream(
    owner: &NativeFilterOwner,
    key_path: &str,
    bound: &BoundValue,
    animations: &mut HashMap<AnimKey, ComponentAnim>,
    effect: usize,
    model: &[f32],
    now: f64,
    force: bool,
    gamut: Gamut,
) {
    let mut live = Vec::new();
    let mut expired = Vec::new();
    for flat in bound.params() {
        match animations.get(&(effect, flat)) {
            Some(anim) if anim.is_settled(now) => expired.push(flat),
            Some(_) => live.push(flat),
            None => {}
        }
    }
    for flat in expired {
        animations.remove(&(effect, flat));
    }
    if live.is_empty() {
        remove_animation(&owner.layer, key_path);
        write_bound_input_at(
            &owner.layer,
            key_path,
            bound,
            animations,
            effect,
            model,
            now,
            gamut,
        );
        return;
    }
    if !force && live.iter().all(|flat| animations[&(effect, *flat)].applied) {
        return;
    }
    let stream = live
        .iter()
        .map(|flat| animations[&(effect, *flat)].remaining(now))
        .max()
        .unwrap_or_default();
    let submitted = match bound {
        BoundValue::Scalar(bound) => submit_animation(owner, key_path, stream, |elapsed| {
            NSNumber::new_f32(bound.value_at(
                animations,
                effect,
                model,
                now + elapsed.as_secs_f64(),
                gamut,
            ))
        }),
        BoundValue::Vec4(components) => submit_animation(owner, key_path, stream, |elapsed| {
            let mut value = [0.0f32; 4];
            for (index, b) in components.iter().enumerate() {
                value[index] = b.value_at(
                    animations,
                    effect,
                    model,
                    now + elapsed.as_secs_f64(),
                    gamut,
                );
            }
            ci_vec4(value)
        }),
    };
    if submitted {
        for flat in &live {
            if let Some(anim) = animations.get_mut(&(effect, *flat)) {
                anim.applied = true;
            }
        }
        write_bound_input_at(
            &owner.layer,
            key_path,
            bound,
            animations,
            effect,
            model,
            now + stream.as_secs_f64(),
            gamut,
        );
    } else {
        write_bound_input_at(
            &owner.layer,
            key_path,
            bound,
            animations,
            effect,
            model,
            now,
            gamut,
        );
    }
}

/// Submits a `CAKeyframeAnimation` on `key_path` — one native value per
/// display frame of `duration` — at the attached screen's cadence, no
/// synthetic cap. Returns `false` when the view is not on a screen: no
/// cadence exists to sample at and the pending timeline stands.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn submit_animation<T: Message>(
    owner: &NativeFilterOwner,
    key_path: &str,
    duration: Duration,
    frames: impl Fn(Duration) -> Retained<T>,
) -> bool {
    let Some(fps) = display_fps(&owner.view) else {
        return false;
    };
    if duration.is_zero() {
        return false;
    }
    // Every frame of the attached display gets a sample — the cadence is
    // the screen's, not an invented ceiling.
    let count = ((duration.as_secs_f64() * f64::from(fps)).ceil() as usize).max(2);
    let step = duration / (count - 1) as u32;
    let values: Vec<Retained<T>> = (0..count).map(|i| frames(step * i as u32)).collect();
    let path = NSString::from_str(key_path);
    let animation = CAKeyframeAnimation::animationWithKeyPath(Some(&path));
    let array = NSArray::from_retained_slice(&values);
    // SAFETY: `values` holds the property's value type and `NSArray`'s
    // element type is erased at runtime.
    unsafe {
        animation.setValues(Some(&*(std::ptr::from_ref(&*array)).cast::<NSArray>()));
    }
    animation.setDuration(duration.as_secs_f64());
    // Explicit linear calculation and pacing — `values` already carry
    // the interpolator's curve, so CA must not pace them a second time.
    // SAFETY: the `kCA…` externs are immutable well-known constant
    // objects owned by Core Animation.
    unsafe {
        animation.setCalculationMode(kCAAnimationLinear);
        animation.setTimingFunction(Some(&CAMediaTimingFunction::functionWithName(
            kCAMediaTimingFunctionLinear,
        )));
    }
    animation.setRemovedOnCompletion(false);
    owner.layer.addAnimation_forKey(&animation, Some(&path));
    true
}

/// Applies one parameter update — the new timeline is installed once for
/// the parameter and every bound input evaluates that same timeline.
/// The raw model write happens only after the timeline's start is read:
/// `from` is the previous timeline's raw value at `now`, else the prior
/// raw model value — never a presentation read and never the freshly
/// written target. An unanimated or zero-duration update clears any
/// explicit CA animation on the input and writes the end value
/// immediately. When an external capture owns this output the update
/// also notifies its redraw hook.
#[allow(clippy::too_many_lines)]
fn apply_animated(
    owner: &NativeFilterOwner,
    effect: usize,
    flat: usize,
    target: waterui_graphics::filtrate::AnimatedTarget,
) {
    let now = CACurrentMediaTime();
    let mut target = target;
    let interpolator = target.interpolator.take();
    let from = owner.animations.borrow().get(&(effect, flat)).map_or_else(
        || owner.model.borrow()[effect][flat],
        |prior| prior.raw_at(now),
    );
    let to = target.value;
    owner.model.borrow_mut()[effect][flat] = to;
    match interpolator {
        Some(interpolator) if !interpolator.duration().is_zero() => {
            owner.animations.borrow_mut().insert(
                (effect, flat),
                ComponentAnim {
                    from,
                    to,
                    interpolator,
                    started: now,
                    applied: false,
                },
            );
        }
        // An immediate update retires any pending timeline so the input
        // loop takes the write-now path rather than the stale curve.
        _ => {
            owner.animations.borrow_mut().remove(&(effect, flat));
        }
    }
    {
        let model = owner.model.borrow();
        let mut animations = owner.animations.borrow_mut();
        for filter in &owner.filters {
            if filter.effect != effect {
                continue;
            }
            for (key, bound) in &filter.inputs {
                if !bound.binds(flat) {
                    continue;
                }
                resample_bound_stream(
                    owner,
                    &key_path(filter, key),
                    bound,
                    &mut animations,
                    filter.effect,
                    &model[filter.effect],
                    now,
                    true,
                    owner.gamut.get(),
                );
            }
        }
    }
    if owner.external_count.get() > 0
        && let Some(on_redraw) = owner.external_redraw.borrow().as_ref()
    {
        on_redraw();
    }
}

/// Materializes every pending timeline as a `CAKeyframeAnimation` over
/// its *remaining* duration — the same resample the update path uses, in
/// non-forced mode so a stream already covering its timelines is never
/// restarted by a repeated layout pass. Live timelines stay in the map
/// (capture samples them; an interrupt reads `from` there); settled ones
/// are dropped and their end state written.
fn materialize_animations(owner: &NativeFilterOwner) {
    resample_all(owner, false);
}

/// Resamples every bound input's CA stream. `force` re-bakes the
/// submitted values — the evaluation-gamut change requires it since the
/// live streams carry the old resolution's samples.
fn resample_all(owner: &NativeFilterOwner, force: bool) {
    let now = CACurrentMediaTime();
    let model = owner.model.borrow();
    let mut animations = owner.animations.borrow_mut();
    for filter in &owner.filters {
        for (key, bound) in &filter.inputs {
            resample_bound_stream(
                owner,
                &key_path(filter, key),
                bound,
                &mut animations,
                filter.effect,
                &model[filter.effect],
                now,
                force,
                owner.gamut.get(),
            );
        }
    }
}

/// The capture seam's own `CIFilter` instance — inputs synced per frame
/// from the in-flight timelines, then the presentation layer's current
/// values, then the model, so a capture renders the same state the
/// timeline describes at the capture timestamp.
fn build_capture_filter(owner: &NativeFilterOwner, index: usize, now: f64) -> Retained<CIFilter> {
    let screen = &owner.filters[index];
    let capture = &owner.capture_filters[index];
    let model = owner.model.borrow();
    let animations = owner.animations.borrow();
    // SAFETY: `filterWithName:` on an unattached lookup is a legal
    // constructor — the class comes from the approved mapping table.
    let filter = unsafe { CIFilter::filterWithName(&NSString::from_str(capture.class)) }
        .unwrap_or_else(|| panic!("CIFilter class {} is unavailable", capture.class));
    for (key, bound) in &capture.inputs {
        let path = key_path(screen, key);
        let name = NSString::from_str(key);
        match bound {
            BoundValue::Scalar(bound) => {
                let fallback = bound.value(&model[capture.effect], Gamut::DisplayP3);
                let value = match bound {
                    Bound::Param(index, conv) => {
                        animations.get(&(capture.effect, *index)).map_or_else(
                            || presentation_scalar(&owner.layer, &path, fallback),
                            |anim| conv.map(anim.raw_at(now), Gamut::DisplayP3),
                        )
                    }
                    // A scalar bound never holds a `Combo` — combos exist
                    // only as matrix-vector components.
                    Bound::Const(_) | Bound::Combo(_) => {
                        presentation_scalar(&owner.layer, &path, fallback)
                    }
                };
                let number = NSNumber::new_f32(value);
                // SAFETY: `setValue:forKey:` on an unattached `CIFilter`
                // is a legal input write — the SDK's undefined-behaviour
                // rule covers only layer-attached filters. `NSNumber` is
                // the scalar input's type.
                unsafe {
                    filter.setValue_forKey(
                        Some(&*(std::ptr::from_ref(&number)).cast::<AnyObject>()),
                        &name,
                    );
                }
            }
            BoundValue::Vec4(components) => {
                let mut value = [0.0f32; 4];
                for (component, bound) in components.iter().enumerate() {
                    value[component] = bound.value_at(
                        &animations,
                        capture.effect,
                        &model[capture.effect],
                        now,
                        Gamut::DisplayP3,
                    );
                }
                let vector = ci_vec4(value);
                // SAFETY: same unattached-filter write for a `CIVector`.
                unsafe {
                    filter.setValue_forKey(
                        Some(&*(std::ptr::from_ref(&vector)).cast::<AnyObject>()),
                        &name,
                    );
                }
            }
        }
    }
    filter
}

/// The layout face: measurement delegates to the mounted child, which
/// the leaf always owns.
struct NativeFilteredSubView {
    mounted: Rc<RefCell<Mounted>>,
}

impl std::fmt::Debug for NativeFilteredSubView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeFilteredSubView")
            .finish_non_exhaustive()
    }
}

impl SubView for NativeFilteredSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.mounted.borrow().layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.mounted.borrow().layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.mounted.borrow().layout().priority()
    }
}

/// The `CapturableSurface` face of a native-filtered output — owns only
/// the weak owner, so a dropped leaf is never captured.
struct NativeFilterCapturable {
    owner: Weak<NativeFilterOwner>,
}

impl std::fmt::Debug for NativeFilterCapturable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeFilterCapturable")
            .finish_non_exhaustive()
    }
}

/// A private-storage texture for the owned-content capture, allocated on
/// `device` at `format`.
fn new_capture_texture(
    device: &MetalDevice,
    format: objc2_metal::MTLPixelFormat,
    width: u32,
    height: u32,
) -> Retained<MetalTexture> {
    // SAFETY: creates a valid descriptor; Metal validates the arguments.
    let descriptor = unsafe {
        objc2_metal::MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            format,
            width as usize,
            height as usize,
            false,
        )
    };
    descriptor.setUsage(
        objc2_metal::MTLTextureUsage::ShaderRead | objc2_metal::MTLTextureUsage::RenderTarget,
    );
    descriptor.setStorageMode(objc2_metal::MTLStorageMode::Private);
    device
        .newTextureWithDescriptor(&descriptor)
        .expect("Failed to create the native-filter capture texture")
}

impl CapturableSurface for NativeFilterCapturable {
    fn capture_pixel_format(&self) -> objc2_metal::MTLPixelFormat {
        // The composited capture format — the extended-range target the
        // filtered leaf presents in, so captures keep EDR headroom.
        objc2_metal::MTLPixelFormat::RGBA16Float
    }

    fn content_bounds(&self, relative_to: &PlatformView) -> cocoa_ui::Rect {
        let Some(owner) = self.owner.upgrade() else {
            return cocoa_ui::Rect::ZERO;
        };
        cocoa_ui::view::convert_rect(
            &owner.view,
            cocoa_ui::view::bounds(&owner.view),
            Some(relative_to),
        )
    }

    /// Native-pass suppression — direct `hidden` mutation on the host
    /// layer only, counted so overlapping captures nest correctly. The
    /// compositor owns the enclosing disabled-actions transaction.
    fn begin_capture_suppression(&self) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let count = owner.capture_suppression.get() + 1;
        owner.capture_suppression.set(count);
        if count == 1 {
            owner.suppressed_hidden.set(owner.layer.isHidden());
            owner.layer.setHidden(true);
        }
    }

    fn end_capture_suppression(&self) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let count = owner.capture_suppression.get();
        assert!(
            count > 0,
            "native filter capture suppression scopes are unbalanced"
        );
        owner.capture_suppression.set(count - 1);
        if count == 1 {
            owner.layer.setHidden(owner.suppressed_hidden.get());
        }
    }

    /// Redirects parameter updates to the capture's redraw hook. Screen
    /// presentation continues — `CALayer.filters` is Core Animation's
    /// own presentation — while the capture pulls frames.
    fn begin_external_rendering(&self, on_redraw: Rc<dyn Fn()>) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        if owner.external_count.get() == 0 {
            *owner.external_redraw.borrow_mut() = Some(on_redraw);
        }
        owner.external_count.set(owner.external_count.get() + 1);
    }

    fn end_external_rendering(&self, _resume: bool) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let count = owner.external_count.get();
        assert!(
            count > 0,
            "native filter external rendering scopes are unbalanced"
        );
        owner.external_count.set(count - 1);
        if count == 1 {
            *owner.external_redraw.borrow_mut() = None;
            *owner.external_prepared.borrow_mut() = None;
        }
    }

    /// Imports the compositor-owned target as the external destination
    /// and issues the immutable `Prepared`: the exact GPU context and
    /// context that allocated the input texture, the issue timestamp
    /// on the shared CA clock, and a fresh unattached `CIFilter` set
    /// synced to the layer's presentation values and in-flight
    /// timelines at that timestamp — mid-animation state included.
    fn prepare_external_render(&self, texture: &MetalTexture) -> bool {
        let Some(owner) = self.owner.upgrade() else {
            return false;
        };
        // SAFETY: `texture` is the compositor's live retained target for
        // this capture; `retain` takes our own reference on it.
        let retained =
            unsafe { Retained::<MetalTexture>::retain(std::ptr::from_ref(texture).cast_mut()) }
                .expect("native filter external render received a null texture");
        // The exact context that issued this frame — its `Arc` is the
        // identity, carried so completion never re-fetches a replacement
        // issue of the same generation.
        let context = crate::gpu_runtime::runtime(&owner.env).context();
        if !std::ptr::eq(
            &raw const *retained.device(),
            &raw const *crate::gpu_runtime::raw_metal_device(&context),
        ) {
            // The compositor handed a texture the current context issue
            // cannot drive — a stale or foreign target, not a failure.
            return false;
        }
        let width = u32::try_from(retained.width()).expect("capture width exceeds u32");
        let height = u32::try_from(retained.height()).expect("capture height exceeds u32");
        let input = new_capture_texture(&retained.device(), retained.pixelFormat(), width, height);
        // Inputs sync at the issue timestamp on the shared CA clock;
        // mapping that into `FrameTime` is the #1725 `PresentationTime`
        // integration edge, recorded in the report — no stub field.
        let now = CACurrentMediaTime();
        let filters = (0..owner.capture_filters.len())
            .map(|index| build_capture_filter(&owner, index, now))
            .collect();
        *owner.external_prepared.borrow_mut() = Some(Prepared {
            target: retained,
            input,
            context,
            filters,
            width,
            height,
        });
        true
    }

    /// The owned seam: captures the mounted content — native children
    /// and resolved GPU surfaces alike — into the private input, runs
    /// the capture `CIFilter` chain over it at the current timestamp, and
    /// renders into the compositor's texture through the
    /// issued-context-owned `CIContext` with a real command submission.
    /// `Ok(())` lands only after the command buffer reports completion;
    /// `Err(CaptureDeferred)` for a stale preparation, a dropped leaf or
    /// a Metal error.
    fn render_prepared_external_texture(
        &self,
        texture: &MetalTexture,
        _width: u32,
        _height: u32,
        completion: SurfaceCaptureCompletion,
    ) {
        let Some(owner) = self.owner.upgrade() else {
            completion(Err(CaptureDeferred));
            return;
        };
        let Some(prepared) = owner.external_prepared.borrow_mut().take() else {
            completion(Err(CaptureDeferred));
            return;
        };
        if !std::ptr::eq(texture, &raw const *prepared.target) {
            // A render for a texture `prepare_external_render` never saw —
            // the preparation is stale, not a hard failure. Put it back
            // so its own render still finds it.
            *owner.external_prepared.borrow_mut() = Some(prepared);
            completion(Err(CaptureDeferred));
            return;
        }
        let Some(mtm) = MainThreadMarker::new() else {
            *owner.external_prepared.borrow_mut() = Some(prepared);
            completion(Err(CaptureDeferred));
            return;
        };
        let work = crate::main_queue_owned::shared(
            CaptureWork {
                owner: Rc::downgrade(&owner),
                prepared,
                completion: RefCell::new(Some(completion)),
            },
            mtm,
        );
        let input = Retained::clone(&work.get(mtm).prepared.input);
        owner.capture.capture(&input, move |captured| {
            let work = Arc::clone(&work);
            cocoa_ui::main_queue::enqueue(move |mtm| {
                finish_capture(&work, captured, mtm);
            });
        });
    }
}

/// The main-thread half of the external render: builds the `CIImage`
/// pipeline, runs the capture chain and submits the `CIContext` encode
/// on the issued-context-owned queue — `Ok` settles only when Metal reports
/// the command buffer completed.
#[allow(clippy::too_many_lines)]
fn finish_capture(work: &Shared<CaptureWork>, captured: bool, mtm: MainThreadMarker) {
    let work_inner = work.get(mtm);
    macro_rules! settle {
        ($result:expr) => {
            if let Some(completion) = work_inner.completion.borrow_mut().take() {
                completion($result);
            }
            return;
        };
    }
    let Some(owner) = work_inner.owner.upgrade() else {
        settle!(Err(CaptureDeferred));
    };
    if !captured {
        settle!(Err(CaptureDeferred));
    }
    // Obsolescence: the frame's exact context `Arc` was recorded at
    // prepare; if the runtime has moved to a different issue, this
    // capture publishes under a stale context — deferred, never rebound
    // onto the new one.
    if !Arc::ptr_eq(
        &crate::gpu_runtime::runtime(&owner.env).context(),
        &work_inner.prepared.context,
    ) {
        settle!(Err(CaptureDeferred));
    }
    // The captured content as a `CIImage`, tagged with its actual
    // native-raster space — extended-linear Display P3 on this branch's
    // F16 raster (#1766 conformance) — so the context converts bytes, it
    // never re-labels them.
    // SAFETY: the named colorspace exists and the `CGColorSpace`
    // pointer is valid to cast to `AnyObject`.
    let input_space = unsafe {
        Retained::<AnyObject>::retain(
            CFRetained::as_ptr(
                &CGColorSpace::with_name(Some(kCGColorSpaceExtendedLinearDisplayP3))
                    .expect("extended-linear Display P3 is always available"),
            )
            .as_ptr()
            .cast::<AnyObject>(),
        )
        .expect("color space is non-null")
    };
    // SAFETY: `kCIImageColorSpace` is a valid immutable key.
    let image_options =
        unsafe { NSDictionary::from_retained_objects(&[kCIImageColorSpace], &[input_space]) };
    // SAFETY: `input` is the live private texture this frame captured
    // into; `imageWithMTLTexture:options:` wraps it without copy.
    let Some(mut image) = (unsafe {
        CIImage::imageWithMTLTexture_options(&work_inner.prepared.input, Some(&image_options))
    }) else {
        settle!(Err(CaptureDeferred));
    };
    // The frame's own filter instances — inputs were synced at prepare
    // to the issue timestamp; only `inputImage` is wired now.
    for filter in &work_inner.prepared.filters {
        // SAFETY: `inputImage` accepts a `CIImage` on an unattached
        // filter.
        unsafe {
            filter.setValue_forKey(
                Some(&*(std::ptr::from_ref::<CIImage>(&image)).cast::<AnyObject>()),
                &NSString::from_str("inputImage"),
            );
        }
        // SAFETY: a filter with an `inputImage` produces `outputImage`.
        image = unsafe { filter.outputImage() }
            .expect("native capture filter produced no output image");
    }
    // The exact-issue context — cached per `SharedGpuContext` instance
    // and bound to its device. The work's own context is authoritative;
    // the device equality was established at prepare.
    let device = crate::gpu_runtime::raw_metal_device(&work_inner.prepared.context);
    {
        let mut slot = owner.capture_gpu.borrow_mut();
        let stale = slot
            .as_ref()
            .is_none_or(|gpu| !Arc::ptr_eq(&gpu.context, &work_inner.prepared.context));
        if stale {
            // The Cherenkov/filtrate `Working` space is extended-linear
            // Display P3 — the context's working AND output space, so
            // >1 values and P3-only colours survive end to end.
            // SAFETY: the options dictionary holds the documented
            // `CIContext` option types — `CGColorSpace` values under the
            // working/output colorspace keys.
            let working = unsafe {
                Retained::<AnyObject>::retain(
                    CFRetained::as_ptr(
                        &CGColorSpace::with_name(Some(kCGColorSpaceExtendedLinearDisplayP3))
                            .expect("extended-linear Display P3 is always available"),
                    )
                    .as_ptr()
                    .cast::<AnyObject>(),
                )
                .expect("color space is non-null")
            };
            // SAFETY: the kCIContext option keys are valid immutable keys.
            let options = unsafe {
                NSDictionary::from_retained_objects(
                    &[kCIContextWorkingColorSpace, kCIContextOutputColorSpace],
                    &[working.clone(), working],
                )
            };
            // SAFETY: `device` is the prepared context's own Metal device.
            let ci_context =
                unsafe { CIContext::contextWithMTLDevice_options(&device, Some(&options)) };
            let queue = device
                .newCommandQueue()
                .expect("capture command queue creation failed");
            *slot = Some(CaptureGpu {
                context: Arc::clone(&work_inner.prepared.context),
                ci_context,
                queue,
            });
        }
    }
    let gpu = owner.capture_gpu.borrow();
    let gpu = gpu.as_ref().expect("capture context just ensured");
    let command_buffer = gpu
        .queue
        .commandBuffer()
        .expect("capture command buffer creation failed");
    // The completion settles on the main queue after Metal reports —
    // `Ok` only on `MTLCommandBufferStatus::Completed`, never merely a
    // nil `NSError` (a nil error is legal on non-completed statuses).
    // `image`, `target`, `input` and the context are retained by this
    // scope through `commit`, and the handler block carries the settle
    // state until the GPU reports.
    {
        let work = Arc::clone(work);
        let block = block2::RcBlock::new(move |buffer: NonNull<MetalCommandBuffer>| {
            let work = Arc::clone(&work);
            // SAFETY: `buffer` is the command buffer the handler fires
            // for — valid for the call's duration.
            let status = unsafe { buffer.as_ref() }.status();
            cocoa_ui::main_queue::enqueue(move |mtm| {
                // Settle is `Ok` only when every live-state check still
                // holds at the main-queue turn: the owner survives, the
                // runtime is still on the prepared context's issue, the
                // retained context is not device-lost, and Metal reported
                // `Completed`. Anything else is `Deferred` — never a
                // synthetic success on stale or lost state.
                let work_inner = work.get(mtm);
                let Some(owner) = work_inner.owner.upgrade() else {
                    if let Some(completion) = work_inner.completion.borrow_mut().take() {
                        completion(Err(CaptureDeferred));
                    }
                    return;
                };
                let completed = status == objc2_metal::MTLCommandBufferStatus::Completed
                    && Arc::ptr_eq(
                        &crate::gpu_runtime::runtime(&owner.env).context(),
                        &work_inner.prepared.context,
                    )
                    && work_inner.prepared.context.device_lost_reason().is_none();
                if let Some(completion) = work_inner.completion.borrow_mut().take() {
                    completion(if completed {
                        Ok(())
                    } else {
                        Err(CaptureDeferred)
                    });
                }
            });
        });
        unsafe {
            // SAFETY: `block` is a valid completion block, attached
            // before `commit` as the API requires; `addCompletedHandler`
            // copies it.
            command_buffer.addCompletedHandler(
                block2::RcBlock::as_ptr(&block) as objc2_metal::MTLCommandBufferHandler
            );
        }
    }
    // The render's output space — extended-linear Display P3, the
    // texture consumer's contract (the compositor's Working space).
    // SAFETY: the named colorspace always exists.
    let color_space = unsafe {
        CGColorSpace::with_name(Some(kCGColorSpaceExtendedLinearDisplayP3))
            .expect("extended-linear Display P3 is always available")
    };
    let bounds = CGRect::new(
        objc2_core_foundation::CGPoint::new(0.0, 0.0),
        objc2_core_foundation::CGSize::new(
            f64::from(work_inner.prepared.width),
            f64::from(work_inner.prepared.height),
        ),
    );
    // SAFETY: `image` renders into `target` at `bounds`; the objects are
    // alive through `commit` below and the handler retains the work
    // until Metal reports.
    unsafe {
        gpu.ci_context
            .render_toMTLTexture_commandBuffer_bounds_colorSpace(
                &image,
                &work_inner.prepared.target,
                Some(&command_buffer),
                bounds,
                &color_space,
            );
        command_buffer.commit();
    }
}

/// Dropping clears the invalidation sink; the env-registry removal is
/// the #1683 integration's `remove_capturable(&view)` call.
struct NativeFilterGuard {
    view: Retained<PlatformView>,
}

impl std::fmt::Debug for NativeFilterGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeFilterGuard").finish_non_exhaustive()
    }
}

impl Drop for NativeFilterGuard {
    fn drop(&mut self) {
        crate::invalidation::unregister_sink(&self.view);
    }
}

/// Mounts the planned native realization: a visible content mount on a
/// `layerUsesCoreImageFilters` host — no hidden child, no `ViewCapture`
/// on the presentation path, no GPU effect setup and no display-link
/// loop. The capturable the leaf builds is registered by the #1683
/// `CaptureRegistry` integration — this branch predates it.
#[allow(clippy::too_many_lines)]
pub fn mount(
    mtm: MainThreadMarker,
    ctx: &RenderContext<'_>,
    content: AnyView,
    chained: Vec<(AnyEffect, ParamGuards)>,
    plan: &[EffectPlan],
) -> NativeLeaf {
    let view = cocoa_ui::appkit::HostView::new(mtm, cocoa_ui::Rect::ZERO);
    cocoa_ui::view::ensure_layer_backed(&view);
    let mounted = ctx.render(content).mount(&view);
    crate::primary_content::forward(&view, mounted.view());
    let content_view = cocoa_ui::view::retain_base(mounted.view());
    // The visible filtered content — `layerUsesCoreImageFilters` must be
    // set before `filters` attach, per the NSView contract for custom
    // filtered layers.
    view.setLayerUsesCoreImageFilters(true);

    let layer = cocoa_ui::view::layer(&view).expect("native filter host is layer-backed");
    let model: Vec<Vec<f32>> = plan.iter().map(|effect| effect.params.clone()).collect();

    // Screen-attached filters plus a second unattached set for the
    // capture seam — the SDK forbids mutating attached inputs.
    let mut filters = Vec::new();
    let mut capture_filters = Vec::new();
    for (index, plan) in plan.iter().enumerate() {
        for stage in &plan.stages {
            // SAFETY: `filterWithName:` returns the named filter or nil —
            // nil fails loudly, never switches realization.
            let filter = unsafe { CIFilter::filterWithName(&NSString::from_str(stage.class)) }
                .unwrap_or_else(|| {
                    panic!("native filter: CIFilter {} is unavailable", stage.class)
                });
            let name = format!("wui-filter-{}", filters.len());
            // SAFETY: `setName:` is a plain property write on a freshly
            // created, not-yet-attached filter.
            unsafe { filter.setName(&NSString::from_str(&name)) };
            filters.push(BoundFilter {
                filter,
                class: stage.class,
                name: name.clone(),
                inputs: stage.inputs.clone(),
                effect: index,
            });
            // SAFETY: `filterWithName:` on an unattached lookup is a
            // legal constructor — the class is from the approved
            // mapping table.
            let capture_filter =
                unsafe { CIFilter::filterWithName(&NSString::from_str(stage.class)) }
                    .expect("native filter: capture CIFilter must exist");
            capture_filters.push(BoundFilter {
                filter: capture_filter,
                class: stage.class,
                name,
                inputs: stage.inputs.clone(),
                effect: index,
            });
        }
    }
    {
        let array: Retained<NSArray<CIFilter>> = NSArray::from_retained_slice(
            &filters.iter().map(|f| f.filter.clone()).collect::<Vec<_>>(),
        );
        // SAFETY: `NSArray`'s element type is erased at runtime; the
        // array holds only `CIFilter` objects, which `filters` accepts.
        unsafe {
            layer.setFilters(Some(&*(std::ptr::from_ref(&*array)).cast::<NSArray>()));
        }
    }

    let capture = Rc::new(cocoa_ui::capture::ViewCapture::new(
        mtm,
        content_view,
        crate::components::gpu_surface::capturable_resolver(),
    ));
    let owner = Rc::new(NativeFilterOwner {
        view: cocoa_ui::view::retain_base(&view),
        layer,
        env: ctx.env().clone(),
        filters,
        capture_filters,
        capture,
        model: RefCell::new(model),
        animations: RefCell::new(HashMap::new()),
        gamut: Cell::new(Gamut::of_view(&view)),
        capture_gpu: RefCell::new(None),
        external_prepared: RefCell::new(None),
        capture_suppression: Cell::new(0),
        suppressed_hidden: Cell::new(false),
        external_count: Cell::new(0),
        external_redraw: RefCell::new(None),
        guards: RefCell::new(Vec::new()),
    });

    // Initial input values — through the layer key path, the only legal
    // post-attach route.
    for filter in &owner.filters {
        for (key, bound) in &filter.inputs {
            set_input(
                &owner.layer,
                &key_path(filter, key),
                bound,
                &owner.model.borrow()[filter.effect],
                owner.gamut.get(),
            );
        }
    }

    // Reactive parameters: `FilterSignal::index` is the flat index in its
    // own effect's `params()` — the effect's plan index, resolved through
    // `EffectPlan::source`, so watcher, model and filter all name the
    // same effect for any chain shape.
    let mut effect_of_source = HashMap::with_capacity(plan.len());
    for (index, plan) in plan.iter().enumerate() {
        effect_of_source.insert(plan.source, index);
    }
    {
        let mut guards = owner.guards.borrow_mut();
        for (source, (any_effect, _)) in chained.iter().enumerate() {
            let Some(&effect) = effect_of_source.get(&source) else {
                continue;
            };
            let Some(description) = any_effect.description() else {
                continue;
            };
            let weak = Rc::downgrade(&owner);
            description.visit_signals(|signal| {
                let flat = signal.index();
                let bound = crate::main_queue_owned::shared(Weak::clone(&weak), mtm);
                guards.push(signal.watch_animated(move |target| {
                    let bound = Arc::clone(&bound);
                    cocoa_ui::main_queue::enqueue(move |mtm| {
                        if let Some(owner) = bound.get(mtm).upgrade() {
                            apply_animated(&owner, effect, flat, target);
                        }
                    });
                }));
            });
        }
    }

    // Nested surfaces reporting change wake the owned capture, which in
    // turn fires the parent's redraw when external.
    {
        let weak = Rc::downgrade(&owner);
        owner.capture.set_on_redraw(move || {
            if let Some(owner) = weak.upgrade()
                && owner.external_count.get() > 0
                && let Some(on_redraw) = owner.external_redraw.borrow().as_ref()
            {
                on_redraw();
            }
        });
    }

    let mounted = Rc::new(RefCell::new(mounted));
    {
        let weak = Rc::downgrade(&owner);
        // The leaf owns `mounted` strongly through `NativeFilteredSubView`;
        // the layout callback holds only a weak slot so the callback's
        // owner chain never cycles back (#1575 callback rule).
        let mounted = Rc::downgrade(&mounted);
        view.set_layout_handler(move |view| {
            let bounds = cocoa_ui::view::bounds(view);
            if let Some(mounted) = mounted.upgrade() {
                let mounted = mounted.borrow();
                cocoa_ui::view::set_frame(mounted.view(), bounds);
                cocoa_ui::view::layout_immediately(mounted.view());
            }
            if let Some(owner) = weak.upgrade() {
                let gamut = Gamut::of_view(view);
                if gamut != owner.gamut.get() {
                    // The screen's gamut changed — every bound
                    // re-evaluates under the new resolution; the forced
                    // resample re-bakes the submitted streams.
                    owner.gamut.set(gamut);
                    resample_all(&owner, true);
                }
                if cocoa_ui::view::window(view).is_some() {
                    // Newly attached — pending off-screen timelines
                    // materialize onto CA for their remaining duration.
                    materialize_animations(&owner);
                }
            }
        });
    }

    // Invalidation forwards upward only — the native filters re-render
    // from the layer automatically; there is no capture to redo.
    {
        let weak = Rc::downgrade(&owner);
        let sink: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(owner) = weak.upgrade() {
                crate::invalidation::invalidate_rendered_content(&owner.view);
            }
        });
        crate::invalidation::register_sink(&view, sink);
    }

    let capturable = Rc::new(NativeFilterCapturable {
        owner: Rc::downgrade(&owner),
    });

    let guard = NativeFilterGuard {
        view: cocoa_ui::view::retain_base(&view),
    };
    let mut leaf = NativeLeaf::new(
        &view,
        NativeFilteredSubView {
            mounted: Rc::clone(&mounted),
        },
    );
    leaf.keep(view);
    leaf.keep(capturable);
    leaf.keep(chained);
    leaf.keep(guard);
    leaf.keep(owner);
    leaf
}
