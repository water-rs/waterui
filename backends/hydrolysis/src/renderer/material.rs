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
//! The levels the reference platform blends behind the window (`UltraThin`,
//! `Thin`) need the compositor's blur-behind protocol rather than a backdrop
//! of the window's own content; Hydrolysis does not realize them yet
//! (water-rs/waterui#1855), and [`WithinWindowLevel::of`] rejects them.

use waterui::background::Material;
use waterui::theme::ColorScheme;
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
    /// alone. On grey the chroma term vanishes and the stage is
    /// `(1 − amount)·Y + amount·bezier(Y) + offset`, the formula filtrate
    /// checks the `LumaCurve` shader against.
    #[test]
    fn the_table_reproduces_the_measured_interiors() {
        for (level, scheme, interiors) in MEASURED_INTERIORS {
            let runtime = MaterialRuntime::new(level, scheme);
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
