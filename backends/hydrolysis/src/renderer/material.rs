//! `Material` backgrounds as a backdrop treatment (water-rs/waterui#1854).
//!
//! A within-window material level realizes as one backdrop group per
//! material surface. The content painted behind the view is captured at
//! [`CAPTURE_SCALE`] of device resolution, passed through the level's colour
//! stage — a [`LumaCurve`] in encoded sRGB — and then blurred with the
//! level's Gaussian σ; the view's mount samples the result inside its clip.
//! The colour stage runs before the blur, and there is no tint layer.
//!
//! The blur averages encoded sRGB too, as the reference platform's does: an
//! edge between dark and light content settles halfway in encoded value, a
//! point or two to the dark side of the edge, where an average in linear
//! light would pull the halfway point several points further into the dark
//! side.
//!
//! The levels the reference platform blends behind the window (`UltraThin`,
//! `Thin`) need the compositor's blur-behind protocol rather than a backdrop
//! of the window's own content; Hydrolysis does not realize them yet
//! (water-rs/waterui#1855), and [`WithinWindowLevel::of`] rejects them.

use nami::{Computed, SignalExt as _};
use waterui::background::Material;
use waterui::theme::ColorScheme;
use waterui_graphics::filtrate::filters::{GaussianBlur, LumaCurve};
use waterui_graphics::filtrate::{Chain, FilterExt as _, OperatingSpace};
use waterui_graphics::{ParamGuards, Reactive};

/// The fraction of device resolution a material's backdrop is captured at.
///
/// Source: the measured treatment recorded on water-rs/waterui#1854.
const CAPTURE_SCALE: f32 = 0.25;

/// [`CAPTURE_SCALE`] as the engine's capture scale.
pub fn capture_scale() -> cherenkov::CaptureScale {
    cherenkov::CaptureScale::new(CAPTURE_SCALE)
        .expect("hydrolysis: the material capture scale is in (0, 1]")
}

/// A material level the reference platform blends within the window: the
/// levels a backdrop of the window's own content realizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithinWindowLevel {
    /// `Material::Regular`.
    Regular,
    /// `Material::Thick`.
    Thick,
    /// `Material::UltraThick`.
    UltraThick,
}

impl WithinWindowLevel {
    /// The within-window level `material` names.
    ///
    /// # Panics
    ///
    /// When `material` is a level blended behind the window (`UltraThin`,
    /// `Thin`): those need the compositor's blur-behind protocol, which
    /// Hydrolysis does not realize yet.
    pub(crate) fn of(material: Material) -> Self {
        match material {
            Material::Regular => Self::Regular,
            Material::Thick => Self::Thick,
            Material::UltraThick => Self::UltraThick,
            Material::UltraThin | Material::Thin => panic!(
                "hydrolysis: Material::{material:?} is blended behind the window, which \
                 needs the compositor's blur-behind protocol; Hydrolysis does not realize \
                 behind-window materials yet (water-rs/waterui#1855)"
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

/// The backdrop chain a material group runs: the colour stage, then the
/// blur.
pub type MaterialChain = Chain<LumaCurve<Reactive>, GaussianBlur<f32>>;

/// The UI-side state of one material surface: its colour stage, whose
/// parameters follow the environment's colour scheme, and its blur radius.
///
/// The parameters are [`Reactive`] slots fed by the subscriptions in
/// `guards`, so an appearance change reaches the engine's filter without a
/// rebuild of the subtree or the backdrop group.
pub struct MaterialRuntime {
    tone: LumaCurve<Reactive>,
    /// The Gaussian σ, in points.
    sigma: f32,
    _guards: ParamGuards,
}

impl core::fmt::Debug for MaterialRuntime {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MaterialRuntime")
            .field("sigma", &self.sigma)
            .finish_non_exhaustive()
    }
}

impl MaterialRuntime {
    /// The treatment of `level` under the appearance `scheme` follows.
    pub(crate) fn new(level: WithinWindowLevel, scheme: &Computed<ColorScheme>) -> Self {
        let treatment = level.treatment();
        let mut guards = ParamGuards::default();
        let mut bind = |pick: fn(Tone) -> f32| {
            guards.bind(
                scheme
                    .clone()
                    .map(move |scheme| pick(treatment.tone(scheme))),
            )
        };
        let tone = LumaCurve {
            curve: [
                bind(|tone| tone.curve[0]),
                bind(|tone| tone.curve[1]),
                bind(|tone| tone.curve[2]),
                bind(|tone| tone.curve[3]),
            ],
            amount: bind(|tone| tone.amount),
            chroma: bind(Tone::chroma),
            offset: bind(|tone| tone.brightness),
        };
        Self {
            tone,
            sigma: treatment.sigma,
            _guards: guards,
        }
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
            .clone()
            .then(GaussianBlur::new(self.sigma * texels_per_point).in_space(OperatingSpace::Srgb))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_graphics::filtrate::Filter as _;

    #[test]
    #[should_panic(expected = "water-rs/waterui#1855")]
    fn a_behind_window_level_names_its_issue() {
        let _ = WithinWindowLevel::of(Material::Thin);
    }

    #[test]
    fn the_colour_stage_follows_the_appearance() {
        let scheme = waterui_core::binding(ColorScheme::Light);
        let runtime = MaterialRuntime::new(WithinWindowLevel::Regular, &scheme.computed());
        assert_eq!(
            runtime.tone.params(),
            [0.9, 0.83, 0.925, 0.815, 0.75, 0.375, 0.1]
        );
        scheme.set(ColorScheme::Dark);
        assert_eq!(
            runtime.tone.params(),
            [0.16, 0.26, 0.1, 0.1, 0.75, 0.375, 0.0]
        );
        // σ = 29.5 pt at 2 px/pt, captured at a quarter: 14.75 texels.
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
    /// alone. On grey the chroma term vanishes and the stage is
    /// `(1 − amount)·Y + amount·bezier(Y) + offset`, the formula filtrate
    /// checks the `LumaCurve` shader against.
    #[test]
    fn the_table_reproduces_the_measured_interiors() {
        for (level, scheme, interiors) in MEASURED_INTERIORS {
            let runtime = MaterialRuntime::new(level, &Computed::constant(scheme));
            let [v0, v1, v2, v3, amount, _, offset] = runtime.tone.params();
            for (grey, interior) in [0.0_f32, 0.5, 1.0].into_iter().zip(interiors) {
                let u = 1.0 - grey;
                let bezier = (u * u * u).mul_add(
                    v0,
                    (3.0 * grey * u * u).mul_add(
                        v1,
                        (3.0 * grey * grey * u).mul_add(v2, grey * grey * grey * v3),
                    ),
                );
                let out = amount.mul_add(bezier - grey, grey) + offset;
                let level8 = out.clamp(0.0, 1.0) * 255.0;
                assert!(
                    (level8 - f32::from(interior)).abs() <= 2.0,
                    "{level:?} {scheme:?} over {grey}: {level8} against the measured {interior}"
                );
            }
        }
    }
}
