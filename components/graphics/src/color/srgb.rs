use core::str::FromStr;

use nami::Signal;
use waterui_core::{Environment, resolve::Resolvable};

use crate::{
    HexColorError, P3, ResolvedColor, WithOpacity, linear_srgb_to_p3, linear_to_srgb,
    parse::{parse_hex_color, parse_hex_color_runtime},
    srgb_to_linear,
};

/// Represents a color in the sRGB color space.
///
/// sRGB is the standard RGB color space used in most displays and web content.
/// Component values are in the range 0.0 to 1.0.
#[derive(Debug, Clone, Copy)]
pub struct Srgb {
    /// Red component (0.0 to 1.0)
    pub red: f32,
    /// Green component (0.0 to 1.0)
    pub green: f32,
    /// Blue component (0.0 to 1.0)
    pub blue: f32,
}

impl Srgb {
    pub(crate) const RED: Self = Self::from_hex("#F44336");
    pub(crate) const PINK: Self = Self::from_hex("#E91E63");
    pub(crate) const PURPLE: Self = Self::from_hex("#9C27B0");
    pub(crate) const DEEP_PURPLE: Self = Self::from_hex("#673AB7");
    pub(crate) const INDIGO: Self = Self::from_hex("#3F51B5");
    pub(crate) const BLUE: Self = Self::from_hex("#2196F3");
    pub(crate) const LIGHT_BLUE: Self = Self::from_hex("#03A9F4");
    pub(crate) const CYAN: Self = Self::from_hex("#00BCD4");
    pub(crate) const TEAL: Self = Self::from_hex("#009688");
    pub(crate) const GREEN: Self = Self::from_hex("#4CAF50");
    pub(crate) const LIGHT_GREEN: Self = Self::from_hex("#8BC34A");
    pub(crate) const LIME: Self = Self::from_hex("#CDDC39");
    pub(crate) const YELLOW: Self = Self::from_hex("#FFEB3B");
    pub(crate) const AMBER: Self = Self::from_hex("#FFC107");
    pub(crate) const ORANGE: Self = Self::from_hex("#FF9800");
    pub(crate) const DEEP_ORANGE: Self = Self::from_hex("#FF5722");
    pub(crate) const BROWN: Self = Self::from_hex("#795548");
    pub(crate) const GREY: Self = Self::from_hex("#9E9E9E");
    pub(crate) const BLUE_GREY: Self = Self::from_hex("#607D8B");
    /// Black color.
    pub const BLACK: Self = Self::from_hex("#000000");
    /// White color.
    pub const WHITE: Self = Self::from_hex("#FFFFFF");

    /// Creates a new sRGB color from red, green, and blue components.
    ///
    /// # Arguments
    /// * `red` - Red component (0.0 to 1.0)
    /// * `green` - Green component (0.0 to 1.0)
    /// * `blue` - Blue component (0.0 to 1.0)
    #[must_use]
    pub const fn new(red: f32, green: f32, blue: f32) -> Self {
        Self { red, green, blue }
    }

    /// Creates a new sRGB color from 8-bit red, green, and blue components.
    ///
    /// # Arguments
    /// * `red` - Red component (0-255)
    /// * `green` - Green component (0-255)
    /// * `blue` - Blue component (0-255)
    #[must_use]
    pub const fn new_u8(red: u8, green: u8, blue: u8) -> Self {
        Self {
            red: red as f32 / 255.0,
            green: green as f32 / 255.0,
            blue: blue as f32 / 255.0,
        }
    }

    /// Creates a new sRGB color from a hexadecimal color string.
    ///
    /// # Arguments
    /// * `hex` - Hex color string (e.g., "#FF5722" or "0xFF5722")
    #[must_use]
    pub const fn from_hex(hex: &str) -> Self {
        let (red, green, blue) = parse_hex_color(hex);
        Self::new_u8(red, green, blue)
    }

    /// Attempts to create a new sRGB color from a hexadecimal string without panicking.
    ///
    /// # Errors
    ///
    /// Returns an error if the string does not contain exactly six hexadecimal digits
    /// or contains invalid characters.
    pub fn try_from_hex(hex: &str) -> Result<Self, HexColorError> {
        let (red, green, blue) = parse_hex_color_runtime(hex)?;
        Ok(Self::new_u8(red, green, blue))
    }

    /// Creates a new sRGB color from a packed 0xRRGGBB value.
    #[must_use]
    pub const fn from_u32(rgb: u32) -> Self {
        Self::new_u8(
            ((rgb >> 16) & 0xFF) as u8,
            ((rgb >> 8) & 0xFF) as u8,
            (rgb & 0xFF) as u8,
        )
    }

    /// Converts this sRGB color to the P3 color space.
    #[must_use]
    pub fn to_p3(&self) -> P3 {
        // convert srgb to p3 color space
        let linear = [
            srgb_to_linear(self.red),
            srgb_to_linear(self.green),
            srgb_to_linear(self.blue),
        ];
        let p3_linear = linear_srgb_to_p3(linear);
        P3::new(
            linear_to_srgb(p3_linear[0]),
            linear_to_srgb(p3_linear[1]),
            linear_to_srgb(p3_linear[2]),
        )
    }

    /// Creates a color with the specified opacity applied.
    ///
    /// # Arguments
    /// * `opacity` - Opacity value (0.0 = transparent, 1.0 = opaque)
    #[must_use]
    pub const fn with_opacity(self, opacity: f32) -> WithOpacity<Self> {
        WithOpacity::new(self, opacity)
    }

    /// Resolves this sRGB color to a `ResolvedColor` in linear RGB color space.
    #[must_use]
    pub fn resolve(&self) -> ResolvedColor {
        ResolvedColor {
            red: srgb_to_linear(self.red),
            green: srgb_to_linear(self.green),
            blue: srgb_to_linear(self.blue),
            headroom: 0.0,
            opacity: 1.0,
        }
    }
}

impl Resolvable for Srgb {
    type Resolved = ResolvedColor;
    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        ResolvedColor {
            red: srgb_to_linear(self.red),
            green: srgb_to_linear(self.green),
            blue: srgb_to_linear(self.blue),
            headroom: 0.0,
            opacity: 1.0,
        }
    }
}

impl From<(u8, u8, u8)> for Srgb {
    fn from(value: (u8, u8, u8)) -> Self {
        Self::new_u8(value.0, value.1, value.2)
    }
}

impl From<[u8; 3]> for Srgb {
    fn from(value: [u8; 3]) -> Self {
        Self::new_u8(value[0], value[1], value[2])
    }
}

impl From<(f32, f32, f32)> for Srgb {
    fn from(value: (f32, f32, f32)) -> Self {
        Self::new(value.0, value.1, value.2)
    }
}

impl From<[f32; 3]> for Srgb {
    fn from(value: [f32; 3]) -> Self {
        Self::new(value[0], value[1], value[2])
    }
}

impl FromStr for Srgb {
    type Err = HexColorError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from_hex(s)
    }
}
