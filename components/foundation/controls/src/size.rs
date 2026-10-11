//! The size a control is drawn at.

/// How large a control is drawn.
///
/// Material 3 Expressive scales a control through five sizes that change its
/// height, padding, icon size and corner shape together — not just its height
/// — so the size is a semantic choice, like the style, rather than a frame the
/// caller imposes from outside. Each control documents the size it takes when
/// none is chosen; platforms with fewer sizes map these onto theirs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ControlSize {
    /// The most compact size, for dense rows of controls.
    ExtraSmall,
    /// A compact size.
    Small,
    /// A roomier size, for a screen's main control.
    Medium,
    /// A prominent size, large enough to anchor a section.
    Large,
    /// The largest size, for hero controls on big surfaces.
    ExtraLarge,
}
