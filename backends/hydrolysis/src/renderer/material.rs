//! `Material` backgrounds as a backdrop treatment (water-rs/waterui#1854).
//!
//! A within-window material level realizes as a backdrop group its members
//! share — the members under one `.material_group()` scope at one level,
//! under one resolved colour scheme and on one install canvas sample one
//! group, one capture and one chain (water-rs/waterui#1999); a member
//! outside every group is a group of its own. The content painted behind
//! the members is captured at [`CAPTURE_SCALE`] of device resolution,
//! passed through the level's colour stage — a [`LumaCurve`] in encoded
//! sRGB — and then blurred with the level's Gaussian σ; each member's
//! mount samples the result inside its clip. The colour stage runs before
//! the blur, and there is no tint layer.
//!
//! The blur averages encoded sRGB too, as the reference platform's does:
//! across an edge between dark and light content, an encoded-sRGB average
//! centres the transition on the edge, where an average in linear light
//! pulls it several points toward the dark side.
//!
//! A window whose background is a within-window level mounts its root over
//! the same backdrop treatment, of the window's opaque theme background.
//!
//! The levels the reference platform blends behind the window (`UltraThin`,
//! `Thin`) show the desktop through a translucent window
//! (water-rs/waterui#1855). The compositor owns the desktop's pixels and
//! their blur, so the level's colour stage cannot run on them: it is realized
//! as the closest source-over [`Tint`] the window composites under its
//! content. As a view background, rather than a window background, a
//! behind-window level is rejected by [`WithinWindowLevel::of`].

use waterui::background::Material;
use waterui::theme::ColorScheme;
use waterui_core::Environment;
use waterui_graphics::filtrate::filters::{GaussianBlur, LumaCurve};
use waterui_graphics::filtrate::{Chain, FilterExt as _, OperatingSpace};

/// The fraction of device resolution a material's backdrop is captured at.
///
/// Source: the measured treatment recorded on water-rs/waterui#1854.
const CAPTURE_SCALE: f32 = 0.25;

/// [`CAPTURE_SCALE`] as the engine's capture scale.
pub fn capture_scale() -> cherenkov::CaptureScale {
    cherenkov::CaptureScale::new(CAPTURE_SCALE)
        .expect("hydrolysis: the material capture scale is in (0, 1]")
}

/// Where the reference platform blends a material level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blending {
    /// Over the window's own content: a backdrop treatment.
    WithinWindow(WithinWindowLevel),
    /// Over what lies behind the window: a translucent window.
    BehindWindow(BehindWindowLevel),
}

impl Blending {
    /// Where `material` is blended.
    pub(crate) const fn of(material: Material) -> Self {
        match material {
            Material::Regular => Self::WithinWindow(WithinWindowLevel::Regular),
            Material::Thick => Self::WithinWindow(WithinWindowLevel::Thick),
            Material::UltraThick => Self::WithinWindow(WithinWindowLevel::UltraThick),
            Material::UltraThin => Self::BehindWindow(BehindWindowLevel::UltraThin),
            Material::Thin => Self::BehindWindow(BehindWindowLevel::Thin),
        }
    }
}

/// A material level the reference platform blends within the window: the
/// levels a backdrop of the window's own content realizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WithinWindowLevel {
    /// `Material::Regular`.
    Regular,
    /// `Material::Thick`.
    Thick,
    /// `Material::UltraThick`.
    UltraThick,
}

impl WithinWindowLevel {
    /// The within-window level `material` names, as a view's background.
    ///
    /// # Panics
    ///
    /// When `material` is a level blended behind the window (`UltraThin`,
    /// `Thin`): Hydrolysis realizes those as a window's background only. A
    /// region of the desktop blurred behind an inner view is
    /// water-rs/waterui#1853's decision.
    pub(crate) fn of(material: Material) -> Self {
        match Blending::of(material) {
            Blending::WithinWindow(level) => level,
            Blending::BehindWindow(_) => panic!(
                "hydrolysis: Material::{material:?} is blended behind the window; Hydrolysis \
                 realizes it as a window's background (`Window::background`), not as a view's \
                 — blurring the desktop behind an inner view's region is \
                 water-rs/waterui#1853's decision"
            ),
        }
    }

    /// The level's measured treatment.
    const fn treatment(self) -> Treatment {
        match self {
            Self::Regular => REGULAR,
            Self::Thick => THICK,
            Self::UltraThick => ULTRA_THICK,
        }
    }
}

/// One appearance's colour stage: `f(Y) + k·(c − Y)` on encoded sRGB, with
/// `f(Y) = (1 − amount)·Y + amount·bezier(Y; curve) + brightness` and
/// `k = saturation·(1 − amount)`.
#[derive(Debug, Clone, Copy)]
struct Tone {
    /// The luminance curve's Bézier control values `v0..v3`.
    curve: [f32; 4],
    /// How much of the curve replaces the identity.
    amount: f32,
    /// The saturation applied to the chroma the curve leaves.
    saturation: f32,
    /// The brightness offset.
    brightness: f32,
}

impl Tone {
    /// The chroma gain `k`.
    fn chroma(self) -> f32 {
        self.saturation * (1.0 - self.amount)
    }
}

/// A within-window level's treatment: the colour stage per appearance and
/// the blur radius.
#[derive(Debug, Clone, Copy)]
struct Treatment {
    light: Tone,
    dark: Tone,
    /// The Gaussian σ, in points.
    sigma: f32,
}

impl Treatment {
    const fn tone(self, scheme: ColorScheme) -> Tone {
        match scheme {
            ColorScheme::Light => self.light,
            ColorScheme::Dark => self.dark,
        }
    }
}

// The measured treatment of each within-window level, per appearance:
// curve values, amount, saturation, brightness and σ.
// Source: the measurement recorded on water-rs/waterui#1854.

const REGULAR: Treatment = Treatment {
    light: Tone {
        curve: [0.9, 0.83, 0.925, 0.815],
        amount: 0.75,
        saturation: 1.5,
        brightness: 0.1,
    },
    dark: Tone {
        curve: [0.16, 0.26, 0.1, 0.1],
        amount: 0.75,
        saturation: 1.5,
        brightness: 0.0,
    },
    sigma: 29.5,
};

const THICK: Treatment = Treatment {
    light: Tone {
        curve: [0.99, 0.95, 0.98, 0.905],
        amount: 0.88,
        saturation: 1.5,
        brightness: 0.045,
    },
    dark: Tone {
        curve: [0.14, 0.16, 0.1, 0.03],
        amount: 0.88,
        saturation: 1.5,
        brightness: 0.0,
    },
    sigma: 45.0,
};

const ULTRA_THICK: Treatment = Treatment {
    light: Tone {
        curve: [0.8, 0.9, 1.1, 0.825],
        amount: 0.75,
        saturation: 1.1,
        brightness: 0.1,
    },
    dark: Tone {
        curve: [0.23, 0.52, 0.27, 0.255],
        amount: 0.75,
        saturation: 2.0,
        brightness: -0.1,
    },
    sigma: 22.5,
};

/// A material level the reference platform blends behind the window: the
/// levels a translucent window realizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BehindWindowLevel {
    /// `Material::UltraThin`.
    UltraThin,
    /// `Material::Thin`.
    Thin,
}

impl BehindWindowLevel {
    /// The tint the window composites over the desktop under `scheme`.
    pub(crate) const fn tint(self, scheme: ColorScheme) -> Tint {
        match (self, scheme) {
            (Self::UltraThin, ColorScheme::Light) => ULTRA_THIN_LIGHT,
            (Self::UltraThin, ColorScheme::Dark) => ULTRA_THIN_DARK,
            (Self::Thin, ColorScheme::Light) => THIN_LIGHT,
            (Self::Thin, ColorScheme::Dark) => THIN_DARK,
        }
    }
}

/// A behind-window level's colour stage realized as a source-over tint in
/// encoded sRGB, `out = alpha·color + (1 − alpha)·in`, over the desktop the
/// compositor blurs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tint {
    /// The tint's coverage.
    pub alpha: f32,
    /// The tint's grey, in encoded sRGB.
    pub color: f32,
}

impl Tint {
    /// The tint as a straight-alpha encoded-sRGB colour.
    pub(crate) const fn srgb(self) -> peniko::Color {
        peniko::Color::new([self.color, self.color, self.color, self.alpha])
    }
}

// The tint of each behind-window level, per appearance. The compositor owns
// the blur of the desktop behind the window, so the measured colour stage
// `f(Y) + k·(c − Y)` cannot run on those pixels; the tint is the source-over
// layer closest to it: `(1 − alpha)·Y + alpha·color` fitted by least squares
// to the stage's grey-ramp response `f(Y)` at Y = 0, 0.1, …, 0.9, 1. The
// curve's bend is what a straight line leaves; the chroma gain `k` becomes
// `1 − alpha`.
// Source: the measured device models recorded on water-rs/waterui#1854.
//
// level      appearance  alpha   color   worst residual  |k − (1 − alpha)|
// UltraThin  light       0.3825  0.9193  2.33 levels     0.068
// UltraThin  dark        0.4239  0.2579  2.72 levels     0.026
// Thin       light       0.6056  0.9525  5.58 levels     0.146
// Thin       dark        0.6421  0.1879  2.92 levels     0.182
//
// Residuals are in 8-bit levels of encoded sRGB, at the ramp's ends.

const ULTRA_THIN_LIGHT: Tint = Tint {
    alpha: 0.3825,
    color: 0.9193,
};

const ULTRA_THIN_DARK: Tint = Tint {
    alpha: 0.4239,
    color: 0.2579,
};

const THIN_LIGHT: Tint = Tint {
    alpha: 0.6056,
    color: 0.9525,
};

const THIN_DARK: Tint = Tint {
    alpha: 0.6421,
    color: 0.1879,
};

/// The backdrop chain a material group runs: the colour stage, then the
/// blur.
pub type MaterialChain = Chain<LumaCurve<f32>, GaussianBlur<f32>>;

/// The resolved treatment of one `(level, colour scheme)` backdrop-group
/// key: its colour stage and blur radius as plain values.
///
/// An appearance change re-keys the member's backdrop group, so the scheme
/// reaches the filter through a new group and the colour stage snaps to
/// the new appearance — it does not animate.
#[derive(Clone, Copy, Debug)]
pub struct MaterialRuntime {
    tone: LumaCurve<f32>,
    /// The Gaussian σ, in points.
    sigma: f32,
}

impl MaterialRuntime {
    /// The treatment of `level` under `scheme`, resolved once.
    pub(crate) fn new(level: WithinWindowLevel, scheme: ColorScheme) -> Self {
        let treatment = level.treatment();
        let tone = treatment.tone(scheme);
        Self {
            tone: LumaCurve {
                curve: tone.curve,
                amount: tone.amount,
                chroma: tone.chroma(),
                offset: tone.brightness,
            },
            sigma: treatment.sigma,
        }
    }

    /// The colour stage's resolved parameters — a test-facing answer.
    #[cfg(test)]
    pub(crate) fn tone_params(&self) -> [f32; 7] {
        use waterui_graphics::filtrate::Filter as _;
        self.tone.params()
    }

    /// The backdrop chain for a surface at `display_scale` device pixels per
    /// point: the colour stage, then the blur in encoded sRGB with σ
    /// converted from points to capture texels.
    pub(crate) fn chain(&self, display_scale: f64) -> MaterialChain {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a display scale is a small factor; f32 carries it exactly enough for a blur radius"
        )]
        let texels_per_point = (display_scale * f64::from(CAPTURE_SCALE)) as f32;
        self.tone
            .then(GaussianBlur::new(self.sigma * texels_per_point).in_space(OperatingSpace::Srgb))
    }
}

/// A window background's within-window material: the backdrop the window's
/// root is mounted over, sampled as a backdrop group of its own.
pub struct WindowBackdrop {
    level: WithinWindowLevel,
}

impl crate::renderer::HydrolysisRenderer {
    /// Sets the within-window material the window's background names, or
    /// `None` for any other background. The window's root is mounted over
    /// it, as a view is over its material background. A change of level
    /// asks for a refresh, so the frame re-flushes over it.
    pub(crate) fn set_window_backdrop(&mut self, level: Option<WithinWindowLevel>) {
        if self.window_backdrop.as_ref().map(|backdrop| backdrop.level) == level {
            return;
        }
        self.window_backdrop = level.map(|level| WindowBackdrop { level });
        // A window-level input written outside any node marks the root for
        // re-layout (§B.1); the mark carries the pump wake.
        self.core.root_cell().mark_layout();
    }

    /// Presents the window's backdrop, if its background names one: the
    /// window layer's material request for the next commit — a solo group
    /// over the whole window at `bounds`, under the colour scheme `env`
    /// resolves. Called before the root flushes; reading the scheme
    /// subscribes the flush, so an appearance flip re-keys the group.
    pub(crate) fn present_window_backdrop(&mut self, bounds: kurbo::Rect, env: &Environment) {
        self.core.window_material = if let Some(backdrop) = &self.window_backdrop {
            let level = backdrop.level;
            let scheme = self.read_signal(&waterui::theme::current_color_scheme(env));
            Some(crate::renderer::mount::MaterialRequest {
                scope: None,
                level,
                scheme,
                bounds,
                visible: true,
            })
        } else {
            None
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_graphics::filtrate::Filter as _;

    #[test]
    #[should_panic(expected = "water-rs/waterui#1853")]
    fn a_behind_window_view_background_names_its_decision() {
        let _ = WithinWindowLevel::of(Material::Thin);
    }

    /// The colour stage `tone` applies to a uniform `grey` backdrop: on grey
    /// the chroma term vanishes and the stage is
    /// `(1 − amount)·Y + amount·bezier(Y) + brightness`, the formula filtrate
    /// checks the `LumaCurve` shader against.
    fn stage(tone: Tone, grey: f32) -> f32 {
        let [v0, v1, v2, v3] = tone.curve;
        let u = 1.0 - grey;
        let bezier = (u * u * u).mul_add(
            v0,
            (3.0 * grey * u * u).mul_add(
                v1,
                (3.0 * grey * grey * u).mul_add(v2, grey * grey * grey * v3),
            ),
        );
        tone.amount.mul_add(bezier - grey, grey) + tone.brightness
    }

    /// Each tint reproduces the colour stage it stands in for on the grey
    /// ramp within the recorded worst residual.
    #[test]
    fn the_tints_reproduce_the_grey_ramp() {
        // The stage measured on water-rs/waterui#1854 per behind-window level
        // and appearance, and the tint's recorded worst residual in 8-bit
        // levels.
        let stages = [
            (
                BehindWindowLevel::UltraThin,
                ColorScheme::Light,
                Tone {
                    curve: [0.45, 0.55, 0.65, 0.68],
                    amount: 0.5,
                    saturation: 1.1,
                    brightness: 0.12,
                },
                2.33,
            ),
            (
                BehindWindowLevel::UltraThin,
                ColorScheme::Dark,
                Tone {
                    curve: [0.24, 0.24, 0.3, 0.39],
                    amount: 0.5,
                    saturation: 1.1,
                    brightness: 0.0,
                },
                2.72,
            ),
            (
                BehindWindowLevel::Thin,
                ColorScheme::Light,
                Tone {
                    curve: [0.725, 0.825, 0.76, 0.73],
                    amount: 0.6,
                    saturation: 1.35,
                    brightness: 0.12,
                },
                5.58,
            ),
            (
                BehindWindowLevel::Thin,
                ColorScheme::Dark,
                Tone {
                    curve: [0.2, 0.21, 0.1, 0.15],
                    amount: 0.6,
                    saturation: 1.35,
                    brightness: 0.0,
                },
                2.92,
            ),
        ];
        for (level, scheme, tone, worst) in stages {
            let tint = level.tint(scheme);
            for step in 0..=10_u8 {
                let grey = f32::from(step) / 10.0;
                let tinted = tint.alpha.mul_add(tint.color - grey, grey);
                let residual = (stage(tone, grey) - tinted).abs() * 255.0;
                assert!(
                    residual <= worst + 0.01,
                    "{level:?} {scheme:?} over {grey}: {residual} levels from the stage"
                );
            }
        }
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "the tone parameters are the treatment's literal table bound through untouched"
    )]
    fn each_appearance_resolves_its_colour_stage() {
        assert_eq!(
            MaterialRuntime::new(WithinWindowLevel::Regular, ColorScheme::Light)
                .tone
                .params(),
            [0.9, 0.83, 0.925, 0.815, 0.75, 0.375, 0.1]
        );
        assert_eq!(
            MaterialRuntime::new(WithinWindowLevel::Regular, ColorScheme::Dark)
                .tone
                .params(),
            [0.16, 0.26, 0.1, 0.1, 0.75, 0.375, 0.0]
        );
        // σ = 29.5 pt at 2 px/pt, captured at a quarter: 14.75 texels.
        let runtime = MaterialRuntime::new(WithinWindowLevel::Regular, ColorScheme::Light);
        assert!((runtime.chain(2.0).second.sigma - 14.75).abs() <= f32::EPSILON);
    }

    /// The interiors measured on water-rs/waterui#1854 through each
    /// within-window level, over black, grey 0.5 and white: 8-bit encoded
    /// sRGB on the device.
    const MEASURED_INTERIORS: [(WithinWindowLevel, ColorScheme, [u8; 3]); 6] = [
        (
            WithinWindowLevel::Regular,
            ColorScheme::Light,
            [197, 225, 245],
        ),
        (WithinWindowLevel::Regular, ColorScheme::Dark, [31, 64, 83]),
        (
            WithinWindowLevel::Thick,
            ColorScheme::Light,
            [233, 243, 245],
        ),
        (WithinWindowLevel::Thick, ColorScheme::Dark, [31, 42, 37]),
        (
            WithinWindowLevel::UltraThick,
            ColorScheme::Light,
            [178, 240, 247],
        ),
        (
            WithinWindowLevel::UltraThick,
            ColorScheme::Dark,
            [18, 75, 87],
        ),
    ];

    /// Each level's colour stage maps a uniform grey backdrop to the measured
    /// interior within 2 levels; the blur after it leaves a uniform image
    /// alone.
    #[test]
    fn the_table_reproduces_the_measured_interiors() {
        for (level, scheme, interiors) in MEASURED_INTERIORS {
            let tone = level.treatment().tone(scheme);
            for (grey, interior) in [0.0_f32, 0.5, 1.0].into_iter().zip(interiors) {
                let level8 = stage(tone, grey).clamp(0.0, 1.0) * 255.0;
                assert!(
                    (level8 - f32::from(interior)).abs() <= 2.0,
                    "{level:?} {scheme:?} over {grey}: {level8} against the measured {interior}"
                );
            }
        }
    }
}
