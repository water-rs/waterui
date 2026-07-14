use waterui_core::impl_constant;
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

impl_constant!(WaveformTheme);

impl Default for WaveformTheme {
    fn default() -> Self {
        Self::cyber()
    }
}

impl WaveformTheme {
    /// Creates a reusable waveform theme.
    #[must_use]
    pub fn new(
        background: impl Into<Color>,
        line: impl Into<Color>,
        glow: impl Into<Color>,
        line_width: f32,
        glow_intensity: f32,
    ) -> Self {
        Self {
            bg_color: background.into(),
            line_color: line.into(),
            glow_color: glow.into(),
            line_width,
            glow_intensity,
        }
    }

    /// Replaces the background color.
    #[must_use]
    pub fn background(mut self, color: impl Into<Color>) -> Self {
        self.bg_color = color.into();
        self
    }

    /// Replaces the waveform line color.
    #[must_use]
    pub fn line(mut self, color: impl Into<Color>) -> Self {
        self.line_color = color.into();
        self
    }

    /// Replaces the glow color.
    #[must_use]
    pub fn glow_color(mut self, color: impl Into<Color>) -> Self {
        self.glow_color = color.into();
        self
    }

    /// Replaces the waveform line width in pixels.
    #[must_use]
    pub const fn line_width(mut self, width: f32) -> Self {
        self.line_width = width;
        self
    }

    /// Replaces the glow intensity.
    #[must_use]
    pub const fn glow(mut self, intensity: f32) -> Self {
        self.glow_intensity = intensity;
        self
    }

    /// Cyberpunk-style theme with cyan glow.
    #[must_use]
    pub fn cyber() -> Self {
        Self::new(
            Color::srgb_f32(0.05, 0.05, 0.1),
            Color::srgb_f32(0.0, 1.0, 0.8),
            Color::srgb_f32(0.0, 0.8, 1.0),
            2.0,
            0.6,
        )
    }

    /// Voice recorder style with red bars.
    #[must_use]
    pub fn recorder() -> Self {
        Self::new(
            Color::srgb_f32(0.05, 0.05, 0.05),
            Color::srgb_f32(1.0, 0.2, 0.2),
            Color::srgb_f32(1.0, 0.1, 0.1),
            3.0,
            0.3,
        )
    }

    /// Minimal green oscilloscope.
    #[must_use]
    pub fn oscilloscope() -> Self {
        Self::new(
            Color::srgb_f32(0.0, 0.02, 0.0),
            Color::srgb_f32(0.0, 1.0, 0.0),
            Color::srgb_f32(0.0, 0.8, 0.0),
            1.5,
            0.5,
        )
    }
}
