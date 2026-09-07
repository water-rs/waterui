//! The module grid a QR symbol is drawn from.
//!
//! Encoding is [`qrcode`]'s: bit stream, error-correction blocks, mask
//! selection and the placement rules of ISO/IEC 18004 are a specification this
//! crate has no business re-implementing. What comes back out of it here is
//! only the grid — [`QrMatrix`] — because everything downstream draws the
//! modules itself.

use alloc::vec::Vec;
use core::fmt;

use qrcode::types::QrError as SymbolError;
use qrcode::{Color as ModuleColor, EcLevel, QrCode as Symbol};

/// How much of a symbol may be obscured or damaged and still decode.
///
/// Higher levels spend more of the symbol on recovery data, so the same payload
/// needs a larger grid: the choice is between a small code and a robust one,
/// and it depends on where the code ends up rather than on what it says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ErrorCorrection {
    /// Recovers roughly 7% of the symbol. The smallest grid a payload fits in.
    Low,
    /// Recovers roughly 15%. The level a code is drawn at when nothing else is
    /// asked for, and the one most published codes use.
    #[default]
    Medium,
    /// Recovers roughly 25%.
    Quartile,
    /// Recovers roughly 30%. What a code printed on something that gets
    /// handled — or drawn over with a logo — needs.
    High,
}

impl ErrorCorrection {
    /// The encoder's spelling of this level.
    const fn level(self) -> EcLevel {
        match self {
            Self::Low => EcLevel::L,
            Self::Medium => EcLevel::M,
            Self::Quartile => EcLevel::Q,
            Self::High => EcLevel::H,
        }
    }
}

impl fmt::Display for ErrorCorrection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::Quartile => "quartile",
            Self::High => "high",
        })
    }
}

/// Why a payload could not be turned into a symbol.
///
/// There is no lower level to fall back to and nothing to truncate: a code that
/// says less than the payload is a code that sends whoever scans it somewhere
/// else, so encoding either produces the payload or fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QrError {
    /// The payload is longer than the largest symbol holds at this level.
    ///
    /// The ceiling is roughly 2 953 bytes at [`ErrorCorrection::Low`] and 1 273
    /// at [`ErrorCorrection::High`], so a payload that overflows one level may
    /// well fit a lower one — but which trade to make is the caller's, not this
    /// crate's.
    #[error(
        "a payload of {length} bytes does not fit any QR symbol at {correction} error correction"
    )]
    PayloadTooLong {
        /// Length of the payload in bytes.
        length: usize,
        /// The level it was encoded at.
        correction: ErrorCorrection,
    },
    /// The encoder refused the payload for a reason other than its length.
    #[error("the payload could not be encoded as a QR symbol: {reason}")]
    Unencodable {
        /// What the encoder objected to.
        reason: &'static str,
    },
}

/// The square grid of a QR symbol, one `bool` per module, dark being `true`.
///
/// This is the symbol proper: it carries no quiet zone, because the quiet zone
/// is a property of how the symbol is placed rather than of the symbol itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrMatrix {
    dark: Vec<bool>,
    width: usize,
}

impl QrMatrix {
    /// Encodes `payload` at `correction`, choosing the smallest symbol version
    /// that holds it.
    ///
    /// # Errors
    ///
    /// Returns [`QrError::PayloadTooLong`] when the payload exceeds the largest
    /// symbol at this level, and [`QrError::Unencodable`] when the encoder
    /// refuses it for any other reason.
    ///
    /// ```
    /// use waterui_qr::{ErrorCorrection, QrMatrix};
    ///
    /// let matrix = QrMatrix::encode("https://waterui.dev", ErrorCorrection::Medium)?;
    /// // The smallest symbol is 21 modules on a side, and every symbol is square.
    /// assert!(matrix.width() >= 21);
    /// # Ok::<(), waterui_qr::QrError>(())
    /// ```
    pub fn encode(payload: &str, correction: ErrorCorrection) -> Result<Self, QrError> {
        let symbol =
            Symbol::with_error_correction_level(payload, correction.level()).map_err(|error| {
                match error {
                    SymbolError::DataTooLong => QrError::PayloadTooLong {
                        length: payload.len(),
                        correction,
                    },
                    SymbolError::InvalidVersion => QrError::Unencodable {
                        reason: "the encoder picked a symbol version the payload does not fit",
                    },
                    SymbolError::UnsupportedCharacterSet | SymbolError::InvalidCharacter => {
                        QrError::Unencodable {
                            reason: "the payload contains bytes the encoder cannot place",
                        }
                    }
                    SymbolError::InvalidEciDesignator => QrError::Unencodable {
                        reason: "the payload names an invalid ECI designator",
                    },
                }
            })?;

        let width = symbol.width();
        let dark = symbol
            .into_colors()
            .into_iter()
            .map(|module| module == ModuleColor::Dark)
            .collect();
        Ok(Self { dark, width })
    }

    /// The number of modules along one side, quiet zone excluded.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Whether the module at `(x, y)` is dark, counting from the top left.
    ///
    /// # Panics
    ///
    /// Panics when either coordinate is outside the grid.
    #[must_use]
    pub fn is_dark(&self, x: usize, y: usize) -> bool {
        assert!(
            x < self.width && y < self.width,
            "module ({x}, {y}) is outside a {}x{} symbol",
            self.width,
            self.width
        );
        self.dark[y * self.width + x]
    }

    /// The grid a row at a time, top to bottom.
    #[must_use]
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &[bool]> {
        self.dark.chunks_exact(self.width)
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::String;

    use super::{ErrorCorrection, QrError, QrMatrix};

    /// The payload every test in this crate encodes unless it needs another.
    const PAYLOAD: &str = "https://waterui.dev";

    #[test]
    fn a_symbol_is_square_and_carries_its_finder_patterns() {
        let matrix = QrMatrix::encode(PAYLOAD, ErrorCorrection::Medium).expect("the payload fits");

        assert!(
            matrix.width() >= 21 && matrix.width() % 4 == 1,
            "a QR symbol is 21 + 4n modules on a side, got {}",
            matrix.width()
        );
        assert_eq!(matrix.rows().len(), matrix.width(), "the grid is square");

        // The three finder patterns are the one part of a symbol whose contents
        // the specification fixes, so they are what tells us the grid is the
        // right way up and indexed the way `is_dark` claims.
        let last = matrix.width() - 7;
        for (corner_x, corner_y) in [(0, 0), (last, 0), (0, last)] {
            for (x, y) in [(0, 0), (6, 0), (0, 6), (6, 6), (3, 3)] {
                assert!(
                    matrix.is_dark(corner_x + x, corner_y + y),
                    "the finder pattern at ({corner_x}, {corner_y}) is missing its frame"
                );
            }
            for (x, y) in [(1, 1), (5, 1), (1, 5), (5, 5)] {
                assert!(
                    !matrix.is_dark(corner_x + x, corner_y + y),
                    "the finder pattern at ({corner_x}, {corner_y}) has no light ring"
                );
            }
        }
    }

    /// A stronger level spends more of the symbol on recovery, so the same
    /// payload needs at least as large a grid.
    #[test]
    fn stronger_correction_never_shrinks_the_symbol() {
        let mut previous = 0;
        for correction in [
            ErrorCorrection::Low,
            ErrorCorrection::Medium,
            ErrorCorrection::Quartile,
            ErrorCorrection::High,
        ] {
            let width = QrMatrix::encode(PAYLOAD, correction)
                .unwrap_or_else(|error| panic!("{correction}: {error}"))
                .width();
            assert!(
                width >= previous,
                "{correction} produced a {width}-module symbol, smaller than the {previous}-module one before it"
            );
            previous = width;
        }
    }

    /// A payload past the largest symbol is an error, not a smaller code: there
    /// is nothing to truncate and nothing to fall back to.
    #[test]
    fn an_over_long_payload_is_an_error() {
        // Comfortably past the ~1 273-byte ceiling at High, and past the
        // ~2 953-byte one at Low.
        let payload = String::from_iter(core::iter::repeat_n('W', 4096));

        let error = QrMatrix::encode(&payload, ErrorCorrection::High)
            .expect_err("4096 bytes does not fit any symbol");
        assert_eq!(
            error,
            QrError::PayloadTooLong {
                length: 4096,
                correction: ErrorCorrection::High,
            }
        );
        assert_eq!(
            format!("{error}"),
            "a payload of 4096 bytes does not fit any QR symbol at high error correction"
        );
    }

    /// Encoding is deterministic, which is what lets a cached matrix be reused
    /// for an unchanged payload.
    #[test]
    fn the_same_payload_encodes_the_same_way_every_time() {
        assert_eq!(
            QrMatrix::encode(PAYLOAD, ErrorCorrection::Medium),
            QrMatrix::encode(PAYLOAD, ErrorCorrection::Medium)
        );
    }

    #[test]
    #[should_panic(expected = "is outside a")]
    fn a_module_outside_the_grid_is_a_caller_bug() {
        let matrix = QrMatrix::encode(PAYLOAD, ErrorCorrection::Low).expect("the payload fits");
        let _ = matrix.is_dark(matrix.width(), 0);
    }
}
