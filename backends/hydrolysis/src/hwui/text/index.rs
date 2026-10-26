//! Byte ↔ UTF-16 offsets of one text: the seam indexes bytes, the platform
//! layout indexes UTF-16 units.

use crate::hwui::HwuiError;

/// Bytes between checkpoints: a lookup scans at most this many bytes.
const STRIDE: usize = 64;

/// The byte and UTF-16 offsets of one text.
///
/// It holds the text itself, and a `(byte, utf16)` checkpoint at the first character boundary of every [`STRIDE`] bytes, so
/// a lookup is a binary search plus a scan of under one stride. An ASCII
/// text needs no checkpoints. Offsets are `i32`, Java's `int`; a text with
/// more UTF-16 units than that cannot be laid out by the platform.
#[derive(Clone, Debug)]
pub struct Utf16Index {
    text: Box<str>,
    units: i32,
    /// Empty for an ASCII text, whose offsets are equal.
    checkpoints: Box<[(usize, i32)]>,
}

fn out_of_range(utf16: i32, units: i32) -> HwuiError {
    HwuiError::Text {
        reason: format!("the platform answered UTF-16 offset {utf16} in a text of {units} units"),
    }
}

impl Utf16Index {
    /// The index of `text`.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Text`] when `text` has more UTF-16 units than an `i32`
    /// counts.
    pub fn new(text: &str) -> Result<Self, HwuiError> {
        let too_long = |units: usize| HwuiError::Text {
            reason: format!(
                "a text of {units} UTF-16 units is longer than the platform's offsets reach"
            ),
        };
        if text.is_ascii() {
            let units = i32::try_from(text.len()).map_err(|_| too_long(text.len()))?;
            return Ok(Self {
                text: text.into(),
                units,
                checkpoints: Box::new([]),
            });
        }
        let mut checkpoints = Vec::with_capacity(text.len() / STRIDE + 1);
        let mut units = 0usize;
        let mut next = 0;
        for (byte, character) in text.char_indices() {
            if byte >= next {
                checkpoints.push((byte, i32::try_from(units).map_err(|_| too_long(units))?));
                next = byte + STRIDE;
            }
            units += character.len_utf16();
        }
        Ok(Self {
            text: text.into(),
            units: i32::try_from(units).map_err(|_| too_long(units))?,
            checkpoints: checkpoints.into_boxed_slice(),
        })
    }

    /// The index of the empty text.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            text: Box::from(""),
            units: 0,
            checkpoints: Box::new([]),
        }
    }

    /// The text's length in bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.text.len()
    }

    /// Whether the text is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The text's length in UTF-16 units.
    #[must_use]
    pub const fn utf16_len(&self) -> i32 {
        self.units
    }

    /// The UTF-16 offset of the seam's byte offset `byte`: a byte inside a
    /// character maps to the character's start, and a byte past the end to
    /// the end, as the seam's layouts treat their positions.
    #[must_use]
    pub fn utf16(&self, byte: usize) -> i32 {
        let byte = byte.min(self.len());
        if self.checkpoints.is_empty() {
            // ASCII: offsets are equal and fit an `i32` (checked in `new`).
            return i32::try_from(byte).unwrap_or(self.units);
        }
        let at = self
            .checkpoints
            .partition_point(|&(start, _)| start <= byte)
            - 1;
        let (mut position, mut units) = self.checkpoints[at];
        for character in self.text[position..].chars() {
            let next = position + character.len_utf8();
            if next > byte {
                break;
            }
            position = next;
            units += if character.len_utf16() == 2 { 2 } else { 1 };
        }
        units
    }

    /// The byte offset of the platform's UTF-16 offset `utf16`; the second
    /// unit of a surrogate pair maps to the character's start.
    ///
    /// # Errors
    ///
    /// [`HwuiError::Text`] when `utf16` is outside the text: the platform
    /// layout answered for a text it was not given.
    pub fn byte(&self, utf16: i32) -> Result<usize, HwuiError> {
        if !(0..=self.units).contains(&utf16) {
            return Err(out_of_range(utf16, self.units));
        }
        if self.checkpoints.is_empty() {
            return usize::try_from(utf16).map_err(|_| out_of_range(utf16, self.units));
        }
        let at = self
            .checkpoints
            .partition_point(|&(_, start)| start <= utf16)
            - 1;
        let (mut position, mut units) = self.checkpoints[at];
        for character in self.text[position..].chars() {
            let width = if character.len_utf16() == 2 { 2 } else { 1 };
            if units + width > utf16 {
                break;
            }
            position += character.len_utf8();
            units += width;
        }
        Ok(position)
    }

    /// Whether `byte` is a character boundary of the text.
    #[must_use]
    pub fn is_boundary(&self, byte: usize) -> bool {
        self.text.is_char_boundary(byte)
    }
}
