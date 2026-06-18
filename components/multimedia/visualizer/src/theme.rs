use waterui_graphics::Color;

/// Theme configuration for Waveform visualization.
#[derive(Debug, Clone)]
pub struct WaveformTheme {
    /// Background color.
    pub(crate) bg_color: Color,
    /// Primary line color.
    pub(crate) line_color: Color,
    /// Glow color.
    pub(crate) glow_color: Color,
    /// Line width in pixels.
    pub(crate) line_width: f32,
    /// Glow intensity (0.0 to 1.0).
    pub(crate) glow_intensity: f32,
}

impl Default for WaveformTheme {
    fn default() -> Self {
        Self::cyber()
    }
}

impl WaveformTheme {
    /// Cyberpunk-style theme with cyan glow.
    #[must_use]
    pub fn cyber() -> Self {
        Self {
            bg_color: Color::srgb_f32(0.05, 0.05, 0.1),
            line_color: Color::srgb_f32(0.0, 1.0, 0.8),
            glow_color: Color::srgb_f32(0.0, 0.8, 1.0),
            line_width: 2.0,
            glow_intensity: 0.6,
        }
    }

    /// Voice recorder style with red bars.
    #[must_use]
    pub fn recorder() -> Self {
        Self {
            bg_color: Color::srgb_f32(0.05, 0.05, 0.05),
            line_color: Color::srgb_f32(1.0, 0.2, 0.2),
            glow_color: Color::srgb_f32(1.0, 0.1, 0.1),
            line_width: 3.0,
            glow_intensity: 0.3,
        }
    }

    /// Minimal green oscilloscope.
    #[must_use]
    pub fn oscilloscope() -> Self {
        Self {
            bg_color: Color::srgb_f32(0.0, 0.02, 0.0),
            line_color: Color::srgb_f32(0.0, 1.0, 0.0),
            glow_color: Color::srgb_f32(0.0, 0.8, 0.0),
            line_width: 1.5,
            glow_intensity: 0.5,
        }
    }
}

/// Theme configuration for Spectrum visualization.
#[derive(Debug, Clone, Copy)]
#[allow(
    dead_code,
    reason = "fields are consumed by the upcoming Spectrum renderer (see the commented `pub use spectrum::Spectrum` in lib.rs)"
)]
pub struct SpectrumTheme {
    /// Background color [R, G, B].
    pub(crate) bg_color: [f32; 3],
    /// Bar color at bottom [R, G, B].
    pub(crate) color_low: [f32; 3],
    /// Bar color at top [R, G, B].
    pub(crate) color_high: [f32; 3],
    /// Number of frequency bars.
    pub(crate) bar_count: u32,
}

impl Default for SpectrumTheme {
    fn default() -> Self {
        Self {
            bg_color: [0.05, 0.05, 0.1],
            color_low: [0.0, 1.0, 0.8],
            color_high: [0.8, 0.0, 1.0],
            bar_count: 64,
        }
    }
}
