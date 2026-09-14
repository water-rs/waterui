//! The color notation `Water.toml` accepts.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A `#RRGGBB` color as written in `Water.toml`.
///
/// Parsing is the only way to build one, so every color a manifest carries is
/// well-formed by the time staging reads it, and a typo fails when the
/// manifest is opened rather than when some platform's resource file is
/// written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HexColor([u8; 3]);

/// A string that is not a `#RRGGBB` color.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("Invalid color '{value}'; expected #RRGGBB")]
pub struct InvalidHexColor {
    /// The rejected text.
    pub value: String,
}

impl HexColor {
    /// The red, green and blue channels.
    #[must_use]
    pub const fn rgb(self) -> [u8; 3] {
        self.0
    }

    /// A color from its three channels.
    #[must_use]
    pub const fn from_rgb(rgb: [u8; 3]) -> Self {
        Self(rgb)
    }

    /// WCAG relative luminance, 0 for black and 1 for white.
    #[must_use]
    pub fn relative_luminance(self) -> f64 {
        let linear = |channel: u8| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.039_28 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        let [red, green, blue] = self.0;
        0.0722f64.mul_add(
            linear(blue),
            0.7152f64.mul_add(linear(green), 0.2126 * linear(red)),
        )
    }
}

impl FromStr for HexColor {
    type Err = InvalidHexColor;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidHexColor {
            value: value.to_string(),
        };
        let digits = value.strip_prefix('#').unwrap_or(value);
        // `from_str_radix` accepts a sign, so the digits are checked first.
        if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let channel = |range: std::ops::Range<usize>| {
            u8::from_str_radix(&digits[range], 16).map_err(|_| invalid())
        };
        Ok(Self([channel(0..2)?, channel(2..4)?, channel(4..6)?]))
    }
}

impl TryFrom<String> for HexColor {
    type Error = InvalidHexColor;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<HexColor> for String {
    fn from(color: HexColor) -> Self {
        color.to_string()
    }
}

/// The canonical `#RRGGBB` form, the `#` always present.
impl fmt::Display for HexColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [red, green, blue] = self.0;
        write!(f, "#{red:02X}{green:02X}{blue:02X}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_with_or_without_hash_and_canonicalizes_case() {
        assert_eq!(
            "#0a84ff".parse::<HexColor>().unwrap().to_string(),
            "#0A84FF"
        );
        assert_eq!("0A84FF".parse::<HexColor>().unwrap().to_string(), "#0A84FF");
    }

    #[test]
    fn rejects_short_and_non_hex_text() {
        for bad in ["#fff", "#GG0000", "red", "#0A84FF00", "#+A84FF"] {
            assert_eq!(
                bad.parse::<HexColor>(),
                Err(InvalidHexColor {
                    value: bad.to_string()
                })
            );
        }
    }

    #[test]
    fn luminance_spans_black_to_white() {
        assert_eq!(HexColor::from_rgb([0, 0, 0]).relative_luminance(), 0.0);
        assert!((HexColor::from_rgb([255, 255, 255]).relative_luminance() - 1.0).abs() < 1e-9);
        let navy = HexColor::from_rgb([0x0B, 0x1E, 0x3F]).relative_luminance();
        assert!(navy < 0.05, "navy is dark: {navy}");
    }

    #[test]
    fn channels_round_trip() {
        let color = HexColor::from_rgb([10, 132, 255]);
        assert_eq!(color.to_string(), "#0A84FF");
        assert_eq!(color.rgb(), [10, 132, 255]);
    }

    #[test]
    fn deserializes_from_a_toml_string_and_rejects_bad_ones() {
        #[derive(Deserialize)]
        struct Doc {
            color: HexColor,
        }
        let doc: Doc = toml::from_str("color = \"#123456\"").unwrap();
        assert_eq!(doc.color.to_string(), "#123456");
        assert!(toml::from_str::<Doc>("color = \"nope\"").is_err());
    }
}
