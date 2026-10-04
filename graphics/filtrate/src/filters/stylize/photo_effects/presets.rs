//! Photo effect presets.
//!
//! Each preset is a zero-parameter colour filter that bakes a hand-tuned
//! transformation into its stage, so presets compose with one another and
//! with other colour filters into one colour segment. Presets approximate
//! Apple's `CIPhotoEffect*` family but are not pixel-identical — they aim
//! to capture the same overall mood with simpler implementations. The
//! presets that clamp are not linear; mono and tonal are.

use crate::Filter;

/// Monochrome: luminance-based desaturation. Approximates `CIPhotoEffectMono`.
#[derive(Debug, Clone, Copy, Default, Filter)]
#[filter(
    color,
    shader = "stylize/photo_effects/photo_effect_mono.wgsl",
    linear = true
)]
pub struct PhotoEffectMono;

/// Noir: high-contrast luminance desaturation, midtone-stretched.
/// Approximates `CIPhotoEffectNoir`.
#[derive(Debug, Clone, Copy, Default, Filter)]
#[filter(
    color,
    shader = "stylize/photo_effects/photo_effect_noir.wgsl",
    linear = false
)]
pub struct PhotoEffectNoir;

/// Chrome: saturation boost with a subtle warm tint.
/// Approximates `CIPhotoEffectChrome`.
#[derive(Debug, Clone, Copy, Default, Filter)]
#[filter(
    color,
    shader = "stylize/photo_effects/photo_effect_chrome.wgsl",
    linear = false
)]
pub struct PhotoEffectChrome;

/// Instant: instant-camera warmth with reduced contrast.
/// Approximates `CIPhotoEffectInstant`.
#[derive(Debug, Clone, Copy, Default, Filter)]
#[filter(
    color,
    shader = "stylize/photo_effects/photo_effect_instant.wgsl",
    linear = false
)]
pub struct PhotoEffectInstant;

/// Fade: lifted blacks and mild desaturation. Approximates
/// `CIPhotoEffectFade`.
#[derive(Debug, Clone, Copy, Default, Filter)]
#[filter(
    color,
    shader = "stylize/photo_effects/photo_effect_fade.wgsl",
    linear = false
)]
pub struct PhotoEffectFade;

/// Process: cool cast with crushed highlights. Approximates
/// `CIPhotoEffectProcess`.
#[derive(Debug, Clone, Copy, Default, Filter)]
#[filter(
    color,
    shader = "stylize/photo_effects/photo_effect_process.wgsl",
    linear = false
)]
pub struct PhotoEffectProcess;

/// Tonal: neutral low-saturation. Approximates `CIPhotoEffectTonal`.
#[derive(Debug, Clone, Copy, Default, Filter)]
#[filter(
    color,
    shader = "stylize/photo_effects/photo_effect_tonal.wgsl",
    linear = true
)]
pub struct PhotoEffectTonal;

/// Transfer: warm fade with a soft sepia bias. Approximates
/// `CIPhotoEffectTransfer`.
#[derive(Debug, Clone, Copy, Default, Filter)]
#[filter(
    color,
    shader = "stylize/photo_effects/photo_effect_transfer.wgsl",
    linear = false
)]
pub struct PhotoEffectTransfer;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorFilter, Filter};

    #[test]
    fn photo_presets_are_zero_param_colour_filters() {
        const { assert!(<PhotoEffectMono as ColorFilter>::LINEAR) };
        const { assert!(<PhotoEffectTonal as ColorFilter>::LINEAR) };
        const { assert!(!<PhotoEffectNoir as ColorFilter>::LINEAR) };
        const { assert!(!<PhotoEffectChrome as ColorFilter>::LINEAR) };
        const { assert!(!<PhotoEffectInstant as ColorFilter>::LINEAR) };
        const { assert!(!<PhotoEffectFade as ColorFilter>::LINEAR) };
        const { assert!(!<PhotoEffectProcess as ColorFilter>::LINEAR) };
        const { assert!(!<PhotoEffectTransfer as ColorFilter>::LINEAR) };

        assert_eq!(PhotoEffectMono.params().len(), 0);
        assert_eq!(PhotoEffectTransfer.params().len(), 0);
    }
}
