//! Photo effect presets.
//!
//! Each preset is a zero-parameter color-only filter that bakes a hand-tuned
//! transformation directly into its fragment, so multiple presets fuse with
//! one another and with other color-only filters. Presets approximate
//! Apple's `CIPhotoEffect*` family but are not pixel-identical — they aim
//! to capture the same overall mood with simpler implementations.

use crate::FilterDerive;

/// Monochrome: luminance-based desaturation. Approximates `CIPhotoEffectMono`.
#[derive(Debug, Clone, Copy, Default, FilterDerive)]
#[filter(color_only, fragment = "stylize/photo_effects/photo_effect_mono.wgsl")]
pub struct PhotoEffectMono;

/// Noir: high-contrast luminance desaturation, midtone-stretched.
/// Approximates `CIPhotoEffectNoir`.
#[derive(Debug, Clone, Copy, Default, FilterDerive)]
#[filter(color_only, fragment = "stylize/photo_effects/photo_effect_noir.wgsl")]
pub struct PhotoEffectNoir;

/// Chrome: saturation boost with a subtle warm tint.
/// Approximates `CIPhotoEffectChrome`.
#[derive(Debug, Clone, Copy, Default, FilterDerive)]
#[filter(color_only, fragment = "stylize/photo_effects/photo_effect_chrome.wgsl")]
pub struct PhotoEffectChrome;

/// Instant: instant-camera warmth with reduced contrast.
/// Approximates `CIPhotoEffectInstant`.
#[derive(Debug, Clone, Copy, Default, FilterDerive)]
#[filter(color_only, fragment = "stylize/photo_effects/photo_effect_instant.wgsl")]
pub struct PhotoEffectInstant;

/// Fade: lifted blacks and mild desaturation. Approximates
/// `CIPhotoEffectFade`.
#[derive(Debug, Clone, Copy, Default, FilterDerive)]
#[filter(color_only, fragment = "stylize/photo_effects/photo_effect_fade.wgsl")]
pub struct PhotoEffectFade;

/// Process: cool cast with crushed highlights. Approximates
/// `CIPhotoEffectProcess`.
#[derive(Debug, Clone, Copy, Default, FilterDerive)]
#[filter(color_only, fragment = "stylize/photo_effects/photo_effect_process.wgsl")]
pub struct PhotoEffectProcess;

/// Tonal: neutral low-saturation. Approximates `CIPhotoEffectTonal`.
#[derive(Debug, Clone, Copy, Default, FilterDerive)]
#[filter(color_only, fragment = "stylize/photo_effects/photo_effect_tonal.wgsl")]
pub struct PhotoEffectTonal;

/// Transfer: warm fade with a soft sepia bias. Approximates
/// `CIPhotoEffectTransfer`.
#[derive(Debug, Clone, Copy, Default, FilterDerive)]
#[filter(color_only, fragment = "stylize/photo_effects/photo_effect_transfer.wgsl")]
pub struct PhotoEffectTransfer;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Filter;

    #[test]
    fn photo_presets_are_zero_param_color_only() {
        assert!(PhotoEffectMono::COLOR_ONLY);
        assert!(PhotoEffectNoir::COLOR_ONLY);
        assert!(PhotoEffectChrome::COLOR_ONLY);
        assert!(PhotoEffectInstant::COLOR_ONLY);
        assert!(PhotoEffectFade::COLOR_ONLY);
        assert!(PhotoEffectProcess::COLOR_ONLY);
        assert!(PhotoEffectTonal::COLOR_ONLY);
        assert!(PhotoEffectTransfer::COLOR_ONLY);

        assert_eq!(PhotoEffectMono.params().len(), 0);
        assert_eq!(PhotoEffectTransfer.params().len(), 0);
    }
}
