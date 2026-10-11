//! Text a render target lays out and draws itself.

/// A shaped text that a render target laid out and owns.
///
/// The target that registered the layout assigns the identifier and draws the
/// layout as a unit, with the colours, spans and font fallback it was laid out
/// with. Recorded content names it through [`Command::Text`](crate::Command::Text),
/// the way it names an image. A target whose text engine records glyph runs
/// registers no text layouts, and receiving the command is a recorder bug.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TextLayoutId(u64);

impl TextLayoutId {
    /// Creates an identifier from a backend-assigned raw value.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}
