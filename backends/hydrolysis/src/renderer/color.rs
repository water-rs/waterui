//! How a render target expects colour to be written into it.
//!
//! Everything that puts colour into a target — the clear value and the
//! compositor's fragment shader alike — has to answer the same question: does
//! the attachment apply the sRGB transfer function on write, or does it store
//! exactly what it is given? Answering it twice, in two places, from two
//! assumptions is what produced water-rs/waterui#233, where every colour the
//! renderer emitted came out as `srgb_to_linear` of itself. Both answers are
//! derived here, from the format itself.

use waterui_graphics::color::{ResolvedColor, Srgb};

/// The encoding a render target's format implies for the values written to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetEncoding {
    /// An `…Srgb` format. The hardware applies the linear-to-sRGB transfer
    /// function on write, so the values handed to it are linear.
    HardwareSrgb,
    /// A plain 8-bit UNORM format. Nothing transforms the value on the way in,
    /// so it has to arrive already carrying the sRGB transfer function.
    StoredSrgb,
    /// A float format, holding extended-range linear values. This is what
    /// wide-gamut and HDR output are written into.
    Linear,
}

impl TargetEncoding {
    /// Reads the encoding off a target's format.
    pub(crate) fn of(format: wgpu::TextureFormat) -> Self {
        if format.is_srgb() {
            Self::HardwareSrgb
        } else if matches!(
            format,
            wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
        ) {
            Self::Linear
        } else {
            Self::StoredSrgb
        }
    }

    /// Whether a colour written to this target has to carry the sRGB transfer
    /// function itself, because nothing downstream will apply it.
    pub(crate) const fn stores_encoded_srgb(self) -> bool {
        matches!(self, Self::StoredSrgb)
    }

    /// The clear value for this target.
    ///
    /// `wgpu` hands the value to the attachment untouched, so what it must
    /// contain depends entirely on what the attachment does with it.
    pub(crate) fn clear_value(self, color: vello::peniko::Color) -> wgpu::Color {
        let srgb = Srgb::new(
            color.components[0],
            color.components[1],
            color.components[2],
        );
        let alpha = f64::from(color.components[3]);
        if self.stores_encoded_srgb() {
            return wgpu::Color {
                r: f64::from(srgb.red),
                g: f64::from(srgb.green),
                b: f64::from(srgb.blue),
                a: alpha,
            };
        }
        let linear = ResolvedColor::from_srgb(srgb).linear_with_headroom();
        wgpu::Color {
            r: f64::from(linear[0]),
            g: f64::from(linear[1]),
            b: f64::from(linear[2]),
            a: alpha,
        }
    }

    /// Whether the compositor's shader should decode the sRGB it samples.
    ///
    /// The scene it samples is always sRGB — vello composites in sRGB and
    /// stores it into a plain `Rgba8Unorm` texture. Decoding is right exactly
    /// when something downstream puts the transfer function back: the hardware
    /// on an `…Srgb` attachment, or nothing at all on a float attachment, which
    /// wants linear in the first place.
    pub(crate) const fn compositor_decodes_source(self) -> bool {
        !self.stores_encoded_srgb()
    }
}

#[cfg(test)]
mod tests {
    use super::TargetEncoding;

    #[test]
    fn a_plain_unorm_target_stores_the_srgb_it_is_given() {
        let encoding = TargetEncoding::of(wgpu::TextureFormat::Rgba8Unorm);
        assert_eq!(encoding, TargetEncoding::StoredSrgb);
        assert!(encoding.stores_encoded_srgb());
        // Decoding here is the #233 defect: nothing would put the transfer
        // function back, so every colour would land as linear.
        assert!(!encoding.compositor_decodes_source());
    }

    #[test]
    fn an_srgb_target_is_handed_linear_because_the_hardware_encodes() {
        for format in [
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::Bgra8UnormSrgb,
        ] {
            let encoding = TargetEncoding::of(format);
            assert_eq!(encoding, TargetEncoding::HardwareSrgb);
            assert!(!encoding.stores_encoded_srgb());
            assert!(encoding.compositor_decodes_source());
        }
    }

    #[test]
    fn a_float_target_holds_extended_range_linear() {
        for format in [
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Rgba32Float,
        ] {
            let encoding = TargetEncoding::of(format);
            assert_eq!(encoding, TargetEncoding::Linear);
            assert!(encoding.compositor_decodes_source());
        }
    }

    /// The clear value and the compositor have to agree, because they paint the
    /// same pixels: the clear fills the frame and the compositor draws over it.
    /// Disagreeing is what made the window background and the content wrong by
    /// the same amount but for different reasons.
    #[test]
    fn the_clear_value_and_the_shader_answer_the_same_question() {
        for format in [
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::Rgba16Float,
        ] {
            let encoding = TargetEncoding::of(format);
            let white = encoding.clear_value(vello::peniko::Color::WHITE);
            // White is the fixed point of the transfer function, so it pins the
            // plumbing without depending on which branch was taken.
            assert!((white.r - 1.0).abs() < 1e-6, "{format:?}");
            assert_eq!(
                encoding.compositor_decodes_source(),
                !encoding.stores_encoded_srgb(),
                "{format:?}"
            );
        }
    }

    /// The value a mid-tone clear carries differs by exactly the transfer
    /// function, which is the whole of #233 in one assertion.
    #[test]
    fn a_mid_tone_differs_between_the_two_kinds_of_target() {
        let mid = vello::peniko::Color::new([0.5, 0.5, 0.5, 1.0]);
        let stored = TargetEncoding::of(wgpu::TextureFormat::Rgba8Unorm).clear_value(mid);
        let hardware = TargetEncoding::of(wgpu::TextureFormat::Rgba8UnormSrgb).clear_value(mid);
        assert!((stored.r - 0.5).abs() < 1e-6, "stored sRGB keeps the value");
        assert!(
            (hardware.r - 0.214_04).abs() < 1e-3,
            "an sRGB attachment is handed linear, got {}",
            hardware.r
        );
    }
}
